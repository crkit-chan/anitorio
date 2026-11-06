import fs from "node:fs/promises";
await fs.writeFile("pkg/build-info.ts", `export const WASM_BUILD_ARGS = Object.freeze(${JSON.stringify(process.argv.slice(2))});\n`);
