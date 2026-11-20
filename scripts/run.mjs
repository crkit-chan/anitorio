// Utility file for removing overhead of using multiple `npm run` scripts

import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";

/**
 * @param {string} command
 * @param {number} depth
 *
 * @returns {string}
 */
function expand(command, depth = 0) {
  if (depth > 20) throw new Error("Circular script reference");
  return command.replace(/npm run ([\w:-]+)(?: -- (.*?))?(?= && | \|\| | ; |$)/g, (_, name, args = "") => {
    const script = pkg.scripts[name];
    if (!script) throw new Error(`Unknown npm script: ${name}`);
    return expand(script + (args ? ` ${args}` : ""), depth + 1);
  });
}

const name = process.argv[2];
const args = process.argv.slice(3);

const pkg = JSON.parse(readFileSync("package.json", "utf8"));
if (!pkg.scripts[name]) throw new Error(`Unknown npm script: ${name}`);

const command = expand(pkg.scripts[name] + (args.length ? ` ${args.join(" ")}` : ""));
console.log(`> ${command}`);

const result = spawnSync(command, { shell: true, stdio: "inherit", env: process.env });

process.exit(result.status ?? 1);
