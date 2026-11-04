import App, { AppApi } from "@/App";
import ResetAllDataButton from "@/components/ResetAllDataButton";
import { ChunkWriter, DefaultFileDBChunkStore } from "@/db";
import "@/index.css";
import { ErrorBoundary, Show } from "solid-js";
import { render } from "solid-js/web";
import streamSaver from "streamsaver"; // If unsupported, this will fall back to a polyfill that buffers in RAM

streamSaver.mitm = `${import.meta.env.BASE_URL}streamsaver/mitm.html`;

let refApp: AppApi;

const STREAMSAVER_MITM = new URL(streamSaver.mitm, location.href);
const SCOPE = new URL("./", STREAMSAVER_MITM.href);
const ORIGIN = location.origin.replace(/(^\w+:|^)\/\//, "");

let chunkWriter: ChunkWriter | null = null;
let dbStore: DefaultFileDBChunkStore | null = null;
let deleteAllChunksPromise = Promise.resolve();
let downloadFrame!: HTMLIFrameElement;
let downloadUrl: string | null = null;
let lastFlushPromise = Promise.resolve();
let streamWriter: WritableStreamDefaultWriter<Uint8Array<ArrayBufferLike>> | null = null;

const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });

// Workaround for Firefox to force streamsaver to work
const observer = new PerformanceObserver((list) => {
  for (const entry of list.getEntries() as PerformanceResourceTiming[]) {
    if (entry.responseStatus === 404 && entry.name === downloadUrl) {
      openDownloadWindow();
    }
  }
});
observer.observe({ type: "resource" });
worker.addEventListener("message", async (evt) => {
  const { writerArgs } = evt.data;
  if (!writerArgs) return;

  if (writerArgs.type === "start") {
    const { filename, streamingDownload } = writerArgs;
    const pathname = `${Math.random().toString(36).slice(2)}-${Date.now().toString(36).slice(2)}/${filename}`;

    downloadUrl = new URL(`${SCOPE.href}${ORIGIN}/${pathname}`).toString();
    refApp.setDownloadUrl(downloadUrl);
    streamWriter = streamSaver.createWriteStream(filename, { pathname }).getWriter();

    if (!streamingDownload) {
      dbStore = new DefaultFileDBChunkStore("default");
      deleteAllChunksPromise = dbStore!.deleteAllChunks();
      chunkWriter = new ChunkWriter(async (blob) => {
        await deleteAllChunksPromise; // Ensure we finished waiting for the chunks from last generation to be removed
        await lastFlushPromise;
        lastFlushPromise = dbStore!.writeChunk(blob);
      });
    }

    worker.postMessage({ type: "writerReady" });
  } else if (writerArgs.chunk) {
    const { id, data } = writerArgs.chunk;
    try {
      await (chunkWriter ?? streamWriter)!.write(data);
      worker.postMessage({ type: "chunkWritten", id });
    } catch (e) {
      console.error(e);
      worker.postMessage({ type: "chunkWritten", id, error: e?.toString() ?? String(e) });
    }
  } else if (writerArgs.type === "done") {
    const { dryRun } = writerArgs;
    try {
      if (chunkWriter) {
        await chunkWriter.flush();
        await lastFlushPromise;
        for await (const chunk of dbStore!.readAllChunks()) {
          try {
            !dryRun && (await streamWriter!.write(chunk)); // Safe to await now
          } catch (e) {
            console.error(e);
            throw e;
          }
        }
      }
    } finally {
      try {
        await streamWriter!.close();
      } finally {
        chunkWriter = null;
        dbStore = null;
        deleteAllChunksPromise = Promise.resolve();
        lastFlushPromise = Promise.resolve();
        streamWriter = null;
      }
    }
  }
});

window.addEventListener("beforeunload", () => streamWriter?.abort());

function openDownloadWindow() {
  refApp.showOpenStreamingWindowWarning();

  const a = document.createElement("a");
  a.href = downloadUrl!;
  a.target = downloadFrame.name;
  a.click();
}
const root = document.getElementById("root");

render(() => {
  return (
    <ErrorBoundary
      fallback={(err) => {
        console.error(err);
        return (
          <div class="w-screen h-screen bg-gray-800">
            <pre class="whitespace-pre-wrap text-red-300 p-4 rounded">
              <h2>Error:</h2>
              <div>{String(err)}</div>

              <Show when={err instanceof Error}>
                <hr class="my-5" />
                <h2>Stack Trace:</h2>
                <div>{(err as Error).stack}</div>
              </Show>
            </pre>

            <ResetAllDataButton />
          </div>
        );
      }}
    >
      <iframe ref={downloadFrame} class="hidden" name="anitorio-download" />
      <App ref={(api) => (refApp = api)} worker={worker} streamingAvailable={streamSaver.supported} />
    </ErrorBoundary>
  );
}, root!);

// Cleanup on page exit
window.addEventListener("beforeunload", () => {
  observer.disconnect();
  worker.terminate();

  const activeWriter = streamWriter;
  streamWriter = null;
  downloadUrl = null;

  void activeWriter?.abort().catch((error) => console.warn("Failed to abort download during page exit:", error));
});
