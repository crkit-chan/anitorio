import fs from "node:fs";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";
import solidPlugin from "vite-plugin-solid";
import { viteStaticCopy } from "vite-plugin-static-copy";

function coiServiceWorker(): Plugin {
  return {
    name: "coi-serviceworker-copy",
    generateBundle() {
      const source = path.resolve(process.cwd(), "node_modules/coi-serviceworker/coi-serviceworker.js");
      this.emitFile({ type: "asset", fileName: "coi-serviceworker.js", source: fs.readFileSync(source) });
    },
  };
}

export default defineConfig({
  appType: "mpa",
  assetsInclude: ["**/*.png", "**/*.jpg", "**/*.gif", "**/*.jpeg", "**/*.lua"],
  base: "/anitorio/",
  build: { target: "esnext" },
  define: {
    __BUILD_NUMBER__: JSON.stringify(process.env.GITHUB_RUN_NUMBER ?? ""),
    __BUILD_COMMIT__: JSON.stringify(process.env.GITHUB_SHA?.slice(0, 7) ?? ""),
  },
  plugins: [
    coiServiceWorker(),
    solidPlugin(),
    viteStaticCopy({ targets: [{ src: "robots.txt", dest: "." }] }),
    viteStaticCopy({
      targets: [
        { src: "node_modules/@hevcjs/core/dist/transcode-worker.js", dest: "." },
        { src: "node_modules/@hevcjs/core/dist/wasm/hevc-decode.js", dest: "." },
        { src: "node_modules/@hevcjs/core/dist/wasm/hevc-decode.wasm", dest: "." },
      ],
    }),
  ],
  resolve: { alias: { "@": path.resolve(__dirname, "web") } },
  server: {
    host: "0.0.0.0",
    port: 3000,
    headers: {
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "require-corp",
    },
  },
  worker: { format: "es" },
  preview: {
    headers: {
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "require-corp",
    },
  },
});
