import type { InputVideoTrack, VideoSample, VideoSampleSink } from "mediabunny";
import type { InitOutput } from "../pkg/anitorio_wasm.js";
import type { WasmSharedMemory } from "./types";
import { BASE_URL, nullify } from "./utils";

const FILE_PREFIX_MAP = { blueprint: "txt", json: "json", rawCompressed: "zlib" } as const;
let hasHevcDecoder = false;
let wasm: InitOutput | null = null;

export async function getVideoData(source: Blob) {
  async function getData() {
    const { ALL_FORMATS, BlobSource, Input } = await import("mediabunny");
    const videoInput = new Input({ source: new BlobSource(source), formats: ALL_FORMATS });
    const videoTrack = await videoInput.getPrimaryVideoTrack();
    if (!videoTrack) throw new Error("No video track found");
    return { videoInput, videoTrack };
  }

  let { videoInput, videoTrack } = await getData();

  if (!(await videoTrack.canDecode())) {
    const codec = await videoTrack?.getCodec();

    if (codec === "hevc" && !hasHevcDecoder) {
      const { registerHevcDecoder } = await import("@hevcjs/mediabunny-plugin");
      await registerHevcDecoder({ workerUrl: `${import.meta.env.BASE_URL}hevcjs/decode-worker.js` });

      ({ videoTrack } = await getData());

      // Check again now that the HEVC decoder is registered.
      if (!(await videoTrack.canDecode())) throw new Error("HEVC decoding is unavailable.");

      hasHevcDecoder = true;
    } else {
      throw new Error(`This browser cannot decode this video with format ${await videoTrack.getCodecParameterString()}`);
    }
  }

  return { videoInput, videoTrack };
}

async function main() {
  const wasmReadyPromises = {
    init: (async () => {
      const { default: init, init: prepare_wasm, set_progress_callback } = await import("../pkg/anitorio_wasm.js");
      wasm = await init();
      prepare_wasm();
      set_progress_callback((percentage: number, status: string) => postMessage({ progress: { percentage, status } }));

      setInterval(() => {
        postMessage({ wasmMemoryUsage: { bytes: wasm?.memory.buffer.byteLength ?? 0 } });
      }, 100);
    })(),
    setCombPosData: null as Promise<void> | null,
    setSignalData: Promise.resolve(),
  };

  // For preview of the rendered GIF
  let writerReadyResolve: (() => void) | null = null;
  let writerReady: Promise<void> = Promise.resolve();

  const state = {
    canvas: nullify<OffscreenCanvas>(),
    ctx: nullify<OffscreenCanvasRenderingContext2D>(),

    shouldShowPreview: false,
  };
  const waitForWriterReady = () => new Promise<void>((resolve) => (writerReadyResolve = resolve));

  const pendingWrites = new Map();
  let nextWriteId = 0;

  // Used to allow rust to yield to JS at regular intervals
  const syncChannel = new MessageChannel();
  const syncQ: (() => void)[] = [];

  syncChannel.port1.onmessage = () => syncQ.shift()?.();

  // Immediately add event listeners
  addEventListener("message", async (evt) => {
    await wasmReadyPromises.init;

    if (evt.data.type === "chunkWritten") {
      const pending = pendingWrites.get(evt.data.id);
      if (!pending) return;

      pendingWrites.delete(evt.data.id);
      evt.data.error ? pending.reject(new Error(evt.data.error)) : pending.resolve();
    } else if (evt.data.canvas) {
      const { canvas } = evt.data.canvas;
      state.canvas = canvas;
      state.ctx = state.canvas!.getContext("2d", { colorSpace: "srgb", willReadFrequently: true })!;
      // state.ctx!.imageSmoothingEnabled = false;
    } else if (evt.data.showPreview) {
      const { showPreview } = evt.data.showPreview;
      state.shouldShowPreview = showPreview;
    } else if (evt.data.generate) {
      const { run_blueprint, set_comb_pos_data } = await import("../pkg/anitorio_wasm.js");

      const { imageData }: { imageData: File | Uint8Array<ArrayBuffer> | ImageData } = evt.data.generate;
      const { args: blueprintArgs } = evt.data.generate;
      const { outputFormat, targetFps } = blueprintArgs;
      const useJsDecodedVideo = blueprintArgs.imageMetadata.mimeType.startsWith("video/");

      let videoDuration: number | null = null;

      let videoSink: VideoSampleSink | null = null;
      let videoSamples: ReturnType<VideoSampleSink["samples"]> | null = null;
      let videoTrack: InputVideoTrack | null = null;

      if (useJsDecodedVideo) {
        const { VideoSampleSink } = await import("mediabunny");

        ({ videoTrack } = await getVideoData(new Blob([imageData as File])));

        videoDuration = await videoTrack.computeDuration();
        videoSink = new VideoSampleSink(videoTrack, { hardwareAcceleration: "prefer-hardware" });
        videoSamples = videoSink.samples();
      }

      type VideoSamples = ReturnType<VideoSampleSink["samples"]>;
      type VideoSampleNext = ReturnType<VideoSamples["next"]>;

      const MAX_LOOKAHEAD_FRAMES = 50;
      const MAX_VIDEO_MEMORY = 512 * 1024 * 1024; // 512 MiB

      const videoSampleQueue: VideoSampleNext[] = [];
      let currVideoSample: VideoSample | null = null;
      let nextOutputTime = 0;
      let videoEnded = false;
      let videoLookahead: number | null = null;
      let videoSampleTail: Promise<void> = Promise.resolve();

      function prefetchVideoSamples() {
        while (videoSamples && !videoEnded && videoSampleQueue.length < (videoLookahead ?? 1)) {
          const pending = videoSampleTail.then(async () => {
            if (videoEnded) return { done: true, value: undefined } as Awaited<VideoSampleNext>;

            const result = await videoSamples!.next();
            if (result.done) {
              videoEnded = true;
            } else if (videoLookahead === null) {
              const bytesPerFrame = result.value!.allocationSize();
              videoLookahead = Math.min(MAX_LOOKAHEAD_FRAMES, Math.max(1, Math.floor(MAX_VIDEO_MEMORY / bytesPerFrame)));
            }

            return result;
          });

          videoSampleTail = pending.then(
            () => {},
            () => {},
          ); // Keep calls to the iterator sequential, even if a request rejects.

          videoSampleQueue.push(pending);
        }
      }

      async function getNextVideoSample(): VideoSampleNext {
        prefetchVideoSamples();

        const pending = videoSampleQueue.shift();
        if (!pending) return { done: true, value: undefined } as Awaited<VideoSampleNext>;

        const result = await pending;
        prefetchVideoSamples(); // Refill the queue as soon as a frame is consumed.
        return result;
      }

      // Safe to do this now as we've already waited for init()
      if (wasmReadyPromises.setCombPosData === null) {
        wasmReadyPromises.setCombPosData = (async () => {
          const data = await fetch(`${BASE_URL}assets/data/combinator-positions.json`).then((v) => v.arrayBuffer());
          await set_comb_pos_data(new Uint8Array(data));
        })();
      }

      await wasmReadyPromises.setCombPosData;
      await wasmReadyPromises.setSignalData;

      if (useJsDecodedVideo) {
        prefetchVideoSamples();
      }

      try {
        waitForWriterReady();

        const prefix = FILE_PREFIX_MAP[outputFormat as keyof typeof FILE_PREFIX_MAP];
        if (!prefix) throw new Error(`Unknown output format: ${outputFormat}`);

        postMessage({ writerArgs: { type: "start", filename: `blueprint.${prefix}` } });

        await writerReady;

        const inputData =
          imageData instanceof File && !useJsDecodedVideo
            ? new Uint8Array(await imageData.arrayBuffer())
            : imageData instanceof ImageData
              ? new Uint8Array(imageData.data.buffer, imageData.data.byteOffset, imageData.data.byteLength)
              : new Uint8Array();

        const blueprintMetadata = await run_blueprint(blueprintArgs, {
          getDataLength: () => inputData.byteLength,
          async getData(inBuffer: WasmSharedMemory) {
            if (useJsDecodedVideo) return;
            if (imageData instanceof File) {
              const reader = imageData.stream().getReader();

              let offset = 0;

              try {
                while (true) {
                  const { done, value } = await reader.read();
                  if (done) break;

                  new Uint8Array(inBuffer.memory.buffer, inBuffer.ptr + offset, value.length).set(value);
                  offset += value.length;
                }

                if (offset !== imageData.size) throw new Error("Incomplete file read");
              } finally {
                reader.releaseLock();
              }
            }
          },
          onChunk(data: Uint8Array) {
            return new Promise<void>((resolve, reject) => {
              try {
                const id = nextWriteId++;
                pendingWrites.set(id, { resolve, reject });
                postMessage({ writerArgs: { chunk: { id, data } } }, { targetOrigin: "*", transfer: [data.buffer] });
              } catch (e) {
                console.error(e);
                reject(e);
              }
            });
          },
          onFrame({ outBuffer, dimensions }: { outBuffer: WasmSharedMemory; dimensions: [number, number] }) {
            try {
              const { canvas, ctx } = state;
              const [width, height] = dimensions;

              if (!canvas || !ctx) return;
              if (canvas.width !== width || canvas.height !== height) Object.assign(canvas, { width, height });

              const data = new Uint8ClampedArray(outBuffer.memory.buffer, outBuffer.ptr, outBuffer.len);
              ctx.putImageData(new ImageData(data, width, height), 0, 0);
            } catch (e) {
              console.error(e);
            }
          },
          async getFrameData({ inBuffer, dimensions }: { inBuffer: WasmSharedMemory; dimensions: [number, number] }) {
            try {
              const frameInterval = 1 / targetFps;

              // Stop only when the output timeline reaches the video duration.
              if (nextOutputTime >= videoDuration!) {
                currVideoSample?.close();
                currVideoSample = null;
                return { isFinished: true };
              }

              // Advance to the source frame covering this output timestamp.
              while (!currVideoSample || nextOutputTime >= currVideoSample.timestamp + currVideoSample.duration) {
                currVideoSample?.close();
                currVideoSample = null;

                const result = await getNextVideoSample();
                if (result.done) return { isFinished: true }; // No new source frames. Keep the last frame if it covers this time.

                currVideoSample = result.value!;
              }

              const sample = currVideoSample;
              const [width, height] = dimensions;
              const { canvas, ctx } = state;
              if (!canvas || !ctx) throw new Error("Canvas is not initialized");
              if (canvas.width !== width || canvas.height !== height) Object.assign(canvas, { width, height });

              sample.draw(ctx, 0, 0, width, height);

              const pixels = ctx.getImageData(0, 0, width, height).data;
              const dest = new Uint8Array(inBuffer.memory.buffer, inBuffer.ptr, inBuffer.len);
              if (pixels.byteLength > dest.byteLength) throw new Error("Video frame exceeds WASM buffer capacity");

              dest.set(pixels);

              const timestamp = nextOutputTime;
              nextOutputTime += frameInterval;

              return { isFinished: false, duration: 1 / targetFps, timestamp };
            } catch (e) {
              console.error(e);
              throw e;
            }
          },
          shouldShowPreview() {
            try {
              postMessage({ shouldShowPreview: { showPreview: state.shouldShowPreview } });
              return state.shouldShowPreview;
            } catch (e) {
              console.error(e);
            }
          },
          yieldToJs: () => new Promise<void>((resolve) => (syncQ.push(resolve), syncChannel.port2.postMessage(null))),
        });
        postMessage({ blueprintMetadata });
        postMessage({ writerArgs: { type: "done" } });
      } catch (e) {
        postMessage({ error: e?.toString() ?? String(e) });
      }
    } else if (evt.data.signalPresetRaw) {
      const { set_signal_data } = await import("../pkg/anitorio_wasm.js");
      const preset: Uint8Array = evt.data.signalPresetRaw;
      wasmReadyPromises.setSignalData = set_signal_data(preset);
    } else if (evt.data.type === "writerReady") {
      writerReadyResolve?.();
      writerReadyResolve = null;
    } else if (evt.data.encodeToGif) {
      const { ALL_FORMATS, BlobSource, Input, VideoSampleSink } = await import("mediabunny");
      const { encode_gif, WasmBuffer, wasm_memory } = await import("../pkg/anitorio_wasm.js");

      const { imageData } = evt.data.encodeToGif;
      const videoInput = new Input({ source: new BlobSource(new Blob([imageData])), formats: ALL_FORMATS });
      const videoTrack = await videoInput.getPrimaryVideoTrack();
      if (!videoTrack) throw new Error("No video track found");
      if (!(await videoTrack.canDecode())) throw new Error("This browser cannot decode this video");

      const [width, height] = [await videoTrack.getDisplayWidth(), await videoTrack.getDisplayHeight()];

      const videoSink = new VideoSampleSink(videoTrack);
      const videoSamples = videoSink.samples();

      const args = { dim: [width, height], speed: 10 };

      postMessage({ writerArgs: { type: "start", filename: `image.gif` } });

      await encode_gif(args, {
        async getFrameData({ inBuffer }: { inBuffer: WasmSharedMemory }) {
          try {
            const nextSample = await videoSamples!.next();
            if (nextSample.done) return { isFinished: true };

            const sample = nextSample.value;

            try {
              const { canvas, ctx } = state;
              if (!canvas || !ctx) throw new Error("Canvas is not initialized");
              if (canvas.width !== width || canvas.height !== height) {
                canvas.width = width;
                canvas.height = height;
              }
              const { duration, timestamp } = sample;

              // Mediabunny handles the sample's rotation and flip metadata.
              sample.draw(ctx, 0, 0, width, height);

              const pixels = ctx.getImageData(0, 0, width, height).data;
              const dest = new Uint8Array(inBuffer.memory.buffer, inBuffer.ptr, inBuffer.len);

              if (pixels.byteLength > dest.byteLength) throw new Error("Video frame exceeds WASM buffer capacity");

              dest.set(pixels);

              return { isFinished: false, duration, timestamp };
            } finally {
              sample.close();
            }
          } catch (e) {
            console.error(e);
            throw e;
          }
        },
        async onChunk(data: Uint8Array) {
          return new Promise<void>((resolve, reject) => {
            try {
              const id = nextWriteId++;
              pendingWrites.set(id, { resolve, reject });
              postMessage({ writerArgs: { chunk: { id, data } } }, { targetOrigin: "*", transfer: [data.buffer] });
            } catch (e) {
              console.error(e);
              reject(e);
            }
          });
        },
      });
      postMessage({ writerArgs: { type: "done" } });
    } else if (evt.data.wasmMemoryUsage) {
    } else {
      console.error("Worker: Unhandled message", evt.data);
    }
  });
}
main();
