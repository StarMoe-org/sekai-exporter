// Evaluates a directory of bundles with the WebAssembly core and writes the same store and list
// of assets as `sekai-exporter export`.
//
// Usage: node run.mjs <package> <input> <output>
//
// <package> is the output directory of `wasm-bindgen --target nodejs`. Every file directly inside
// <input> is a bundle.

import { appendFileSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";

const [packageDir, input, output] = process.argv.slice(2);
if (output === undefined) {
  console.error("usage: node run.mjs <package> <input> <output>");
  process.exit(2);
}

const { evaluate } = createRequire(import.meta.url)(join(resolve(packageDir), "sekai_wasm.js"));

const names = readdirSync(input).filter((name) => statSync(join(input, name)).isFile()).sort();
const bundles = names.map((name) => readFileSync(join(input, name)));

mkdirSync(output, { recursive: true });
const list = join(output, "assets.tsv");
writeFileSync(list, "");

const report = evaluate(
  names,
  bundles,
  (path, bytes) => {
    const file = join(output, "store", path);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, bytes);
  },
  (row) => appendFileSync(list, row),
);
process.stdout.write(report);
