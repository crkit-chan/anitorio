import type { BlueprintArgs, InitOutput } from "@/../pkg/anitorio_wasm";
import { WASM_BUILD_ARGS } from "@/../pkg/build-info";
import type { WasmSharedMemory } from "@/types";
import { BASE_URL, nullify } from "@/utils";
import type { InputVideoTrack, VideoSample, VideoSampleSink } from "mediabunny";

const FILE_PREFIX_MAP = { blueprint: "txt", json: "json", rawCompressed: "zlib" } as const;
let hasHevcDecoder = false;
let wasm: InitOutput | null = null;
let WasmTracingAllocator: (typeof import("@/wasm-tracing-allocator/hooks"))["WasmTracingAllocator"] | null = null;

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
  let prevMemoryUsage = 0;

  const wasmReadyPromises = {
    init: (async () => {
      if (WASM_BUILD_ARGS.includes("--profiling")) {
        ({ WasmTracingAllocator } = await import("@/wasm-tracing-allocator/hooks"));

        if (false) {
          const u = new URL(import.meta.url);
          const BASE_PATH = [u.origin, u.pathname.split("/").filter((v) => !!v)[0], "pkg"].join("/");

          function cleanWasmStack(stack?: string): string {
            if (!stack) return "";
            return (
              stack
                .split("\n")
                .map((line) => line.trim())
                .filter(Boolean)
                // Remove generated runtime frames
                .filter((line) => !/wasm_bindgen|js_sys::futures|externref shim/.test(line))
                // Remove WASM module paths and function indexes
                .map((line) => line.replace(/^.*?\.wasm\./, "").replace(/@.*$/, ""))
                // Remove Rust crate disambiguator hashes
                .map((line) => line.replace(/\[[0-9a-f]{16}\]/g, ""))
                .filter((line) => !/wasm_tracing_allocator::hooks::on_(?:alloc|dealloc|realloc)/.test(line))
                .filter((line) => !/<js_sys::futures::queue::QueueState>::run_all/.test(line))
                .filter((line) => !/<js_sys::futures::queue::Queue>::new()/.test(line))
                .join("\n")
                .replaceAll("::{closure#0}", "()")
            );
          }

          WasmTracingAllocator?.addListener("alloc", ({ size, getStack }) => {
            // if (size > 100000) {
            //   const stack = getStack()
            //     ?.replaceAll(BASE_PATH, "")
            //     .split("\n")
            //     .filter((v) => /anitorio_wasm/.test(v) && !/anitorio_wasm.js/.test(v))
            //     .join("\n");
            //   console.log(formatFileSize(+size), cleanWasmStack(stack));
            // }
            const wasmMemoryUsage = {
              bytes: wasm?.memory.buffer.byteLength ?? 0,
              commited: WasmTracingAllocator?.getTotalAllocated(),
            };
            postMessage({ wasmMemoryUsage });
          });
          WasmTracingAllocator?.addListener("dealloc", ({ size, getStack }) => {
            // if (size > 100000) {
            //   const stack = getStack()
            //     ?.replaceAll(BASE_PATH, "")
            //     .split("\n")
            //     .filter((v) => /anitorio_wasm/.test(v) && !/anitorio_wasm.js/.test(v))
            //     .join("\n");
            //   console.log(formatFileSize(-size), cleanWasmStack(stack));
            // }
            const wasmMemoryUsage = {
              bytes: wasm?.memory.buffer.byteLength ?? 0,
              commited: WasmTracingAllocator?.getTotalAllocated(),
            };
            postMessage({ wasmMemoryUsage });
          });
        }
      }

      setInterval(() => {
        const bytes = wasm?.memory.buffer.byteLength ?? 0;
        const wasmMemoryUsage = {
          bytes,
          commited: WasmTracingAllocator?.getTotalAllocated(),
        };
        if (!WasmTracingAllocator && bytes === prevMemoryUsage) return;

        prevMemoryUsage = bytes;
        postMessage({ wasmMemoryUsage });
      }, 50);

      const { default: init, init: prepare_wasm, init_rayon, set_progress_callbacks } = await import("@/../pkg/anitorio_wasm.js");
      wasm = await init();

      try {
        await init_rayon();
      } catch (error) {
        console.warn("Rayon unavailable:", error);
      }

      prepare_wasm();
      set_progress_callbacks({
        onProgress: (percentage: number, status: string) => postMessage({ progress: { percentage, status } }),
        onLampGroup: (percentage) => postMessage({ lampProgress: { percentage } }),
      });
    })(),
    setCombPosData: null as Promise<void> | null,
  };

  function logWasmAllocationInfo() {
    WasmTracingAllocator?.dumpLiveAllocations({
      keyLabel: "Allocation function",
      valueLabel: "Memory (MiB)",
      // getKey(allocation: { stack: string }) {
      //   const frame = allocation.stack.split("\n").find((line) => line.includes("anitorio_wasm.wasm."));
      //   const first = frame?.split("@")[0];
      //   return first?.replace("anitorio_wasm.wasm.", "").replace(/\[[a-f0-9]{16}\]/g, "") ?? "Unknown";
      // },
      getValue: (allocation: { size: number }) => allocation.size / 1024 / 1024,
    });
  }

  // For preview of the rendered GIF
  let writerReadyResolve: (() => void) | null = null;
  let writerReady: Promise<void> = Promise.resolve();

  const state = {
    canvas: nullify<OffscreenCanvas>(),
    ctx: nullify<OffscreenCanvasRenderingContext2D>(),

    sourceCanvas: nullify<OffscreenCanvas>(),
    sourceCtx: nullify<OffscreenCanvasRenderingContext2D>(),

    shouldShowPreview: false,
  };
  const waitForWriterReady = () => new Promise<void>((resolve) => (writerReadyResolve = resolve));

  const pendingWrites = new Map();
  let nextWriteId = 0;

  // Used to allow rust to yield to JS at regular intervals
  const syncChannel = new MessageChannel();
  const syncQ: ((...args: readonly any[]) => any)[] = [];

  syncChannel.port1.onmessage = () => syncQ.shift()?.();

  // Immediately add event listeners
  addEventListener("message", async (evt) => {
    await wasmReadyPromises.init;

    const { encode_gif, run_blueprint, set_comb_pos_data, set_signal_data } = await import("@/../pkg/anitorio_wasm");

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
    } else if (evt.data.sourceCanvas) {
      const { canvas } = evt.data.sourceCanvas;
      state.sourceCanvas = canvas;
      state.sourceCtx = state.sourceCanvas!.getContext("2d", { colorSpace: "srgb", willReadFrequently: true })!;
      // state.ctx!.imageSmoothingEnabled = false;
    } else if (evt.data.showPreview) {
      const { showPreview } = evt.data.showPreview;
      state.shouldShowPreview = showPreview;
    } else if (evt.data.generate) {
      const { args: blueprintArgs, imageData }: { args: BlueprintArgs; imageData: Uint8Array | SharedArrayBuffer | File } =
        evt.data.generate;
      const { imageArgs, outputFormat } = blueprintArgs;
      const useJsDecodedVideo = imageArgs.mimeType.startsWith("video/");

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
          set_comb_pos_data(new Uint8Array(data));
        })();
      }

      await wasmReadyPromises.setCombPosData;

      if (useJsDecodedVideo) {
        prefetchVideoSamples();
      }

      try {
        if (blueprintArgs.streamingDownload) {
          waitForWriterReady();
        }

        const prefix = FILE_PREFIX_MAP[outputFormat as keyof typeof FILE_PREFIX_MAP];
        if (!prefix) throw new Error(`Unknown output format: ${outputFormat}`);

        const filename = `blueprint.${prefix}`;

        postMessage({
          writerArgs: {
            type: "start",
            filename,
            streamingDownload: blueprintArgs.streamingDownload,
            dryRun: blueprintArgs.dryRun,
          },
        });

        await writerReady;

        const inputData =
          imageData instanceof File && !useJsDecodedVideo
            ? { byteLength: imageData.size }
            : imageData instanceof SharedArrayBuffer
              ? new Uint8Array(imageData)
              : null;

        try {
          const [width, height] = imageArgs.dimensions!;
          if (state.canvas!.width !== width || state.canvas!.height !== height) {
            Object.assign(state.canvas!, { width, height });
          }
          state.ctx!.fillStyle = "#ffffff44";
          state.ctx!.fillRect(0, 0, state.canvas!.width, state.canvas!.height);
        } catch {}

        const reader = new FileReaderSync();
        const blueprintMetadata = await run_blueprint(blueprintArgs, {
          getDataLen: () => inputData?.byteLength ?? 0,
          getData(from, to) {
            if (useJsDecodedVideo) throw new Error("No data to pass through when using JS decoding");
            if (imageData instanceof File) return new Uint8Array(reader.readAsArrayBuffer(imageData.slice(from, to)));

            if (!(inputData instanceof Uint8Array)) throw new Error("Expected inputData to be instanceof Uint8Array");
            return inputData.slice(from, to);
          },
          onChunk(data) {
            return new Promise((resolve, reject) => {
              try {
                const id = nextWriteId++;
                pendingWrites.set(id, { resolve, reject });
                postMessage({ writerArgs: { chunk: { id, data } } }, { targetOrigin: "*", transfer: [data.buffer] });
              } catch (e) {
                console.error(e);
                reject(e);
                throw e;
              }
            });
          },
          async onFrame({ frame, dimensions }) {
            try {
              const { canvas, ctx, sourceCanvas, sourceCtx } = state;
              if (!canvas || !ctx || !sourceCanvas || !sourceCtx) return;

              const [width, height] = dimensions;
              if (sourceCanvas.width !== width || sourceCanvas.height !== height) {
                Object.assign(sourceCanvas, { width, height });
              }
              sourceCtx.putImageData(new ImageData(new Uint8ClampedArray(frame), width, height), 0, 0);

              const rotation = imageArgs.rotate === "none" ? 0 : 360 - parseInt(imageArgs.rotate.match(/\d+/)![0]);
              const swap = rotation === 90 || rotation === 270;
              const outWidth = swap ? height : width;
              const outHeight = swap ? width : height;

              if (canvas.width !== outWidth || canvas.height !== outHeight) {
                canvas.width = outWidth;
                canvas.height = outHeight;
              }

              ctx.setTransform(1, 0, 0, 1, 0, 0);
              ctx.clearRect(0, 0, outWidth, outHeight);
              ctx.translate(outWidth / 2, outHeight / 2);

              if (imageArgs.flip !== "none") {
                const flipMap: Record<string, [number, number]> = { both: [-1, -1], x: [-1, 1], y: [1, -1] };
                ctx.scale(...flipMap[imageArgs.flip]);
              }
              if (rotation % 360 !== 0) {
                ctx.rotate((rotation * Math.PI) / 180);
              }

              ctx.drawImage(sourceCanvas, -width / 2, -height / 2);
            } catch (e) {
              console.error(e);
              throw e;
            }
          },
          async getNextFrame({ inBuffer, dimensions }: { inBuffer: WasmSharedMemory; dimensions: [number, number] }) {
            try {
              const frameInterval = 1 / imageArgs.targetFps;

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
              (canvas.width !== width || canvas.height !== height) && Object.assign(canvas, { width, height });

              sample.draw(ctx, 0, 0, width, height);

              const pixels = ctx.getImageData(0, 0, width, height).data;
              const dest = new Uint8Array(inBuffer.memory.buffer, inBuffer.ptr, inBuffer.len);
              if (pixels.byteLength > dest.byteLength) throw new Error("Video frame exceeds WASM buffer capacity");

              dest.set(pixels);

              const timestamp = nextOutputTime;
              nextOutputTime += frameInterval;

              return { isFinished: false, duration: 1 / imageArgs.targetFps, timestamp };
            } catch (e) {
              console.error(e);
              throw e;
            }
          },
          shouldPreview() {
            try {
              postMessage({ shouldShowPreview: { showPreview: state.shouldShowPreview } });
              return state.shouldShowPreview;
            } catch (e) {
              console.error(e);
              throw e;
            }
          },
          async resetGenerator() {
            if (useJsDecodedVideo) {
              await videoSampleTail;

              videoSampleQueue.length = 0; // clear samples
              videoSamples = videoSink!.samples();
              currVideoSample = null;
              nextOutputTime = 0;
              videoEnded = false;
              videoLookahead = null;
              videoSampleTail = Promise.resolve();

              prefetchVideoSamples();
            }
          },
          jsYield: () => new Promise((resolve) => (syncQ.push(resolve), syncChannel.port2.postMessage(null))),
        });

        postMessage({ blueprintMetadata });
        postMessage({ writerArgs: { type: "done" } });
      } catch (e) {
        console.error(e);
        postMessage({ error: e?.toString() ?? String(e) });
      }
    } else if (evt.data.signalPresetRaw) {
      set_signal_data(evt.data.signalPresetRaw);
    } else if (evt.data.type === "writerReady") {
      writerReadyResolve?.();
      writerReadyResolve = null;
    } else if (evt.data.encodeToGif) {
      const { ALL_FORMATS, BlobSource, Input, VideoSampleSink } = await import("mediabunny");

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
        async get_next_frame({ inBuffer }: { inBuffer: WasmSharedMemory }) {
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
