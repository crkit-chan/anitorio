import { resolve } from "node:path";
import { defineConfig } from "vite";

export default defineConfig({
  publicDir: false,
  build: {
    outDir: "public",
    emptyOutDir: false,
    lib: {
      entry: resolve(__dirname, "coi-serviceworker.ts"),
      formats: ["iife"],
      name: "COIServiceWorker",
      fileName: () => "coi-serviceworker.js",
    },
    minify: false,
  },
});
