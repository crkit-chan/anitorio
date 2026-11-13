from __future__ import annotations

from dataclasses import dataclass
from math import gcd
from pathlib import Path
import time
import warnings

import numpy as np
import soundfile as sf
from scipy.linalg import LinAlgError, LinAlgWarning, solve
from scipy.signal import resample_poly
from threadpoolctl import threadpool_limits


# =========================================================
# Configuration
# =========================================================

SAMPLE_RATE = 44_100
TICKS_PER_SECOND = 60
SAMPLES_PER_TICK = SAMPLE_RATE // TICKS_PER_SECOND  # 735

# Five 441-sample audio blocks span three 735-sample game ticks.
# These phases are a provisional representation of the three
# alignments, not yet verified against Factorio's exact callback model.
AUDIO_BLOCK_SIZE = 441
ALIGNMENT_OFFSETS = (0, 147, 294)

MAX_SOUND_AGE_TICKS = 16
MAX_SOUNDS = 100

# Cheap correlation screening keeps only this many candidates before
# building the much more expensive square Gram matrix.
PRESELECT_CANDIDATES = 128

# One second is a practical first test. Set to None to process the
# entire target after the model has been validated.
MAX_SECONDS = None

BLAS_THREADS = 16

TARGET_FILE = Path("test_10s.wav")
SOUND_DIRECTORY = Path("sounds")

RECONSTRUCTION_FILE = Path("reconstruction.wav")
RESIDUAL_FILE = Path("residual.wav")


# =========================================================
# Audio loading
# =========================================================


def load_audio(path: Path) -> np.ndarray:
    """Load audio as stereo float32 and resample to SAMPLE_RATE."""
    audio, sample_rate = sf.read(path, dtype="float32", always_2d=True)

    if audio.shape[1] == 1:
        audio = np.repeat(audio, 2, axis=1)
    elif audio.shape[1] != 2:
        raise ValueError(f"{path} has {audio.shape[1]} channels; expected mono or stereo.")

    if sample_rate != SAMPLE_RATE:
        divisor = gcd(sample_rate, SAMPLE_RATE)
        audio = resample_poly(audio, SAMPLE_RATE // divisor, sample_rate // divisor, axis=0).astype(np.float32, copy=False)

    return np.ascontiguousarray(audio, dtype=np.float32)


def load_target(path: Path) -> np.ndarray:
    target = load_audio(path)
    if len(target) == 0:
        raise ValueError(f"Target audio is empty: {path}")
    return target


def load_sounds(directory: Path) -> tuple[list[np.ndarray], list[str]]:
    paths = sorted(p for p in directory.rglob("*") if p.is_file() and p.suffix.lower() in {".ogg", ".wav", ".flac"})

    if not paths:
        raise FileNotFoundError(f"No OGG/WAV/FLAC files found in {directory}")

    sounds: list[np.ndarray] = []
    names: list[str] = []

    for i, path in enumerate(paths, 1):
        print(f"\rLoading sounds: {i}/{len(paths)}  {path.name[:55]:55}", end="")
        sounds.append(load_audio(path))
        names.append(path.name)

    print()
    return sounds, names


# =========================================================
# Candidate metadata and precomputed row indices
# =========================================================


@dataclass(frozen=True)
class Candidate:
    sound_index: int
    age_ticks: int
    name: str


def make_candidates(sound_names: list[str]) -> list[Candidate]:
    return [Candidate(sound_index, age, name) for sound_index, name in enumerate(sound_names) for age in range(MAX_SOUND_AGE_TICKS + 1)]


def make_alignment_indices(tick: int, target_length: int):
    """
    Return source offsets and target indices for the three alignment
    windows. The row order is alignment-major.
    """
    within = np.tile(np.arange(SAMPLES_PER_TICK, dtype=np.int32), len(ALIGNMENT_OFFSETS))
    phases = np.repeat(np.asarray(ALIGNMENT_OFFSETS, dtype=np.int32), SAMPLES_PER_TICK)

    target_indices = tick * SAMPLES_PER_TICK + phases + within
    valid_target = target_indices < target_length

    return within, phases, target_indices, valid_target


# =========================================================
# Fast candidate matrix construction
# =========================================================


def build_candidate_matrix(sounds: list[np.ndarray], tick: int, channel: int, within: np.ndarray, phases: np.ndarray) -> np.ndarray:
    """
    Build A with shape (3 * 735, sound_count * 17).

    This replaces the previous Python loop over all 6,120 candidates.
    It loops over source sounds (about 360) and gathers all 17 ages
    for each sound in one NumPy operation.
    """
    sound_count = len(sounds)
    ages = np.arange(MAX_SOUND_AGE_TICKS + 1, dtype=np.int32) * SAMPLES_PER_TICK

    A = np.zeros((len(within), sound_count * len(ages)), dtype=np.float32)

    # Source position = candidate playback age + alignment phase
    # + sample position within the game tick.
    base = (phases + within).astype(np.int32, copy=False)

    for sound_index, audio in enumerate(sounds):
        source_indices = base[:, None] + ages[None, :]
        valid = source_indices < len(audio)

        # Clip only for safe indexing; invalid entries are zeroed below.
        gathered = audio[np.minimum(source_indices, max(0, len(audio) - 1)), channel]
        gathered[~valid] = 0.0

        start = sound_index * len(ages)
        A[:, start : start + len(ages)] = gathered

    return A


def build_target_vector(target: np.ndarray, channel: int, target_indices: np.ndarray, valid_target: np.ndarray) -> np.ndarray:
    y = np.zeros(len(target_indices), dtype=np.float32)
    y[valid_target] = target[target_indices[valid_target], channel]
    return y


# =========================================================
# Candidate screening and Gram construction
# =========================================================


def screen_candidates(A: np.ndarray, y: np.ndarray, limit: int) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """
    Rank candidates by positive target correlation divided by waveform
    norm. This is a cheap first-pass filter; NNLS then resolves
    redundancy among the shortlisted candidates.
    """
    correlations = A.T @ y
    norms_squared = np.einsum("ij,ij->j", A, A, optimize=True)

    valid = (norms_squared > 1e-12) & (correlations > 0.0)
    valid_indices = np.flatnonzero(valid)

    if len(valid_indices) == 0:
        return (np.empty(0, dtype=np.int64), np.empty((A.shape[0], 0), dtype=np.float32), np.empty(0, dtype=np.float32))

    scores = correlations[valid_indices] / np.sqrt(norms_squared[valid_indices])
    count = min(limit, len(valid_indices))

    if count < len(valid_indices):
        chosen_local = np.argpartition(scores, -count)[-count:]
    else:
        chosen_local = np.arange(count)

    chosen = valid_indices[chosen_local]
    chosen = chosen[np.argsort(scores[chosen_local])[::-1]]

    return chosen, A[:, chosen], correlations[chosen]


def build_gram(A: np.ndarray, y: np.ndarray) -> tuple[np.ndarray, np.ndarray, float]:
    """
    Build the small shortlisted Gram matrix.

    A is float32, so BLAS can use fast SGEMM. The result is promoted
    to float64 for the active-set solve.
    """
    G = A.T @ A
    b = A.T @ y

    G = np.asarray(G, dtype=np.float64)
    b = np.asarray(b, dtype=np.float64)
    G = (G + G.T) * 0.5

    yy = float(y @ y)
    return G, b, yy


# =========================================================
# Active-set NNLS in Gram space
# =========================================================


def solve_reduced_system(Gp: np.ndarray, bp: np.ndarray) -> np.ndarray:
    """Fast symmetric solve, with least-squares fallback for redundancy."""
    with warnings.catch_warnings():
        warnings.simplefilter("error", LinAlgWarning)
        try:
            return solve(Gp, bp, assume_a="sym", check_finite=False)
        except (LinAlgError, LinAlgWarning, ValueError):
            return np.linalg.lstsq(Gp, bp, rcond=1e-10)[0]


def nnls_gram(G: np.ndarray, b: np.ndarray, max_nonzero: int = MAX_SOUNDS, tol: float = 1e-9) -> np.ndarray:
    """
    Active-set NNLS using only G=A.T@A and b=A.T@y.

    The active set is capped at max_nonzero. The solve is performed
    on the small passive submatrix, not on the original waveform data.
    """
    n = len(b)
    result = np.zeros(n, dtype=np.float64)

    if n == 0 or max_nonzero <= 0:
        return result

    diagonal = np.maximum(np.diag(G), 0.0)
    keep = diagonal > max(float(diagonal.max(initial=0.0)) * 1e-13, 1e-20)

    if not np.any(keep):
        return result

    indices = np.flatnonzero(keep)
    scale = np.sqrt(diagonal[keep])

    Gn = G[np.ix_(indices, indices)] / np.outer(scale, scale)
    bn = b[keep] / scale

    m = len(bn)
    x = np.zeros(m, dtype=np.float64)
    passive = np.zeros(m, dtype=bool)

    threshold = tol * max(1.0, float(np.max(np.abs(bn))))
    max_iterations = max(500, 15 * min(m, max_nonzero))
    iterations = 0

    while np.count_nonzero(passive) < min(max_nonzero, m):
        gradient = bn - Gn @ x
        gradient[passive] = -np.inf

        j = int(np.argmax(gradient))
        if gradient[j] <= threshold:
            break

        passive[j] = True

        while True:
            iterations += 1
            if iterations > max_iterations:
                raise RuntimeError("NNLS iteration limit reached")

            p = np.flatnonzero(passive)
            z = np.zeros(m, dtype=np.float64)
            z[p] = solve_reduced_system(Gn[np.ix_(p, p)], bn[p])

            if np.all(z[p] > threshold):
                x = z
                break

            bad = p[z[p] <= threshold]
            denominator = x[bad] - z[bad]
            valid = denominator > 0.0

            if np.any(valid):
                alpha = np.min(x[bad[valid]] / denominator[valid])
                x += alpha * (z - x)

            hit = passive & (x <= threshold)
            x[hit] = 0.0
            passive[hit] = False

            if not np.any(passive):
                break

    result[indices] = x / scale
    return result


def gram_error(G: np.ndarray, b: np.ndarray, yy: float, x: np.ndarray) -> float:
    value = yy - 2.0 * float(b @ x) + float(x @ G @ x)
    return max(0.0, value)


# =========================================================
# Per-tick fit
# =========================================================


def solve_tick_channel(sounds: list[np.ndarray], target: np.ndarray, tick: int, channel: int) -> tuple[np.ndarray, np.ndarray, float, np.ndarray]:
    within, phases, target_indices, valid_target = make_alignment_indices(tick, len(target))

    A = build_candidate_matrix(sounds, tick, channel, within, phases)
    y = build_target_vector(target, channel, target_indices, valid_target)

    selected_indices, A_small, _ = screen_candidates(A, y, PRESELECT_CANDIDATES)

    del A

    if len(selected_indices) == 0:
        return selected_indices, np.zeros(0), float(y @ y), np.zeros(0)

    G, b, yy = build_gram(A_small, y)
    volumes = nnls_gram(G, b, max_nonzero=MAX_SOUNDS)
    error = gram_error(G, b, yy, volumes)

    return selected_indices, volumes, error, y


def render_tick_channel(sounds: list[np.ndarray], selected_indices: np.ndarray, volumes: np.ndarray, channel: int) -> np.ndarray:
    """Render one central game tick using selected candidate ages."""
    output = np.zeros(SAMPLES_PER_TICK, dtype=np.float64)

    ages_per_sound = MAX_SOUND_AGE_TICKS + 1

    for candidate_index, volume in zip(selected_indices, volumes):
        if volume <= 0.0:
            continue

        sound_index = int(candidate_index) // ages_per_sound
        age_ticks = int(candidate_index) % ages_per_sound
        audio = sounds[sound_index]

        start = age_ticks * SAMPLES_PER_TICK
        end = min(start + SAMPLES_PER_TICK, len(audio))

        if end > start:
            output[: end - start] += audio[start:end, channel].astype(np.float64) * volume

    return output.astype(np.float32)


# =========================================================
# Main
# =========================================================


def main() -> None:
    started = time.perf_counter()

    with threadpool_limits(limits=BLAS_THREADS):
        print("Loading target...")
        target = load_target(TARGET_FILE)

        if MAX_SECONDS is not None:
            target = target[: int(MAX_SECONDS * SAMPLE_RATE)]

        print("Loading Factorio sounds...")
        sounds, sound_names = load_sounds(SOUND_DIRECTORY)

        candidate_count = len(sounds) * (MAX_SOUND_AGE_TICKS + 1)
        tick_count = (len(target) + SAMPLES_PER_TICK - 1) // SAMPLES_PER_TICK

        rows = len(ALIGNMENT_OFFSETS) * SAMPLES_PER_TICK
        matrix_mib = rows * candidate_count * 4 / (1024**2)

        print("\nConfiguration")
        print(f"  Target duration:       {len(target) / SAMPLE_RATE:.2f}s")
        print(f"  Sound assets:          {len(sounds):,}")
        print(f"  Candidate instances:   {candidate_count:,}")
        print(f"  Rows per tick fit:     {rows:,}")
        print(f"  Full A matrix/channel: {matrix_mib:.1f} MiB")
        print(f"  Gram shortlist:        {PRESELECT_CANDIDATES}")
        print(f"  Maximum selected:      {MAX_SOUNDS}")
        print(f"  Ticks to process:      {tick_count}")
        print(f"  BLAS threads:           {BLAS_THREADS}")
        print()

        reconstruction = np.zeros_like(target, dtype=np.float32)
        total_errors = np.zeros(2, dtype=np.float64)

        for tick in range(tick_count):
            tick_started = time.perf_counter()
            start = tick * SAMPLES_PER_TICK
            end = min(start + SAMPLES_PER_TICK, len(target))

            print(f"Tick {tick + 1}/{tick_count}")

            for channel, label in enumerate(("left", "right")):
                selected, volumes, error, _ = solve_tick_channel(sounds, target, tick, channel)

                rendered = render_tick_channel(sounds, selected, volumes, channel)

                reconstruction[start:end, channel] = rendered[: end - start]
                total_errors[channel] += error

                nonzero = int(np.count_nonzero(volumes > 1e-8))
                print(f"  {label}: {nonzero} active, {len(selected)} screened, error={error:.5g}")

            elapsed = time.perf_counter() - tick_started
            print(f"  Tick time: {elapsed:.3f}s")

        residual = target - reconstruction

        sf.write(RECONSTRUCTION_FILE, reconstruction, SAMPLE_RATE)
        sf.write(RESIDUAL_FILE, residual, SAMPLE_RATE)

    elapsed = time.perf_counter() - started

    print("\nFinished")
    print(f"  Reconstruction: {RECONSTRUCTION_FILE}")
    print(f"  Residual:       {RESIDUAL_FILE}")
    print(f"  Left error:     {total_errors[0]:.6g}")
    print(f"  Right error:    {total_errors[1]:.6g}")
    print(f"  Total time:     {elapsed:.2f}s")


if __name__ == "__main__":
    main()
