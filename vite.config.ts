import path from "node:path";
import { defineConfig, UserConfig } from "vite";
import solidPlugin from "vite-plugin-solid";
import { viteStaticCopy } from "vite-plugin-static-copy";

export const config = {
  appType: "mpa",
  assetsInclude: ["**/*.png", "**/*.jpg", "**/*.gif", "**/*.jpeg", "**/*.lua"],
  base: "/",
  build: { target: "esnext" },
  define: {
    __BUILD_INFO__: {
      commit: JSON.stringify(process.env.VERCEL_GIT_COMMIT_SHA?.slice(0, 7) ?? ""),
      isLocal: process.env.VERCEL_ENV !== "production",
      timestamp: JSON.stringify(new Date().toISOString()),
    },
  } as const,
  plugins: [
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
} satisfies UserConfig;

export default defineConfig(config);
