// Writes the object documents of a directory of bundles with the WebAssembly core, to the same
// paths as `sekai-exporter objects`.
//
// Usage: node objects.mjs <package> <input> <output>
//
// <package> is the output directory of `wasm-bindgen --target nodejs`. Every file directly inside
// <input> is a bundle. Exits with 1 when a bundle could not be read or written.

import { mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";

const [packageDir, input, output] = process.argv.slice(2);
if (output === undefined) {
  console.error("usage: node objects.mjs <package> <input> <output>");
  process.exit(2);
}

const { objects } = createRequire(import.meta.url)(join(resolve(packageDir), "sekai_wasm.js"));

const names = readdirSync(input).filter((name) => statSync(join(input, name)).isFile()).sort();
const bundles = names.map((name) => readFileSync(join(input, name)));

const failed = objects(names, bundles, (path, bytes) => {
  const file = join(output, path);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, bytes);
});
process.stderr.write(failed);
process.exit(failed === "" ? 0 : 1);
