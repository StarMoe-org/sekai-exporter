# WebAssembly runner

English | [简体中文](README.zh-CN.md)

Evaluates a directory of bundles with the WebAssembly core under Node and writes the same `store` directory and `assets.tsv` as `sekai-exporter export`, so the two can be compared byte for byte.

```bash
cargo build --release -p sekai-wasm --target wasm32-unknown-unknown
wasm-bindgen --target nodejs --out-dir wasm-package \
  target/wasm32-unknown-unknown/release/sekai_wasm.wasm
node tools/wasm-runner/run.mjs wasm-package <input> <output>
```

Use the `wasm-bindgen` program of the same version as the `wasm-bindgen` crate in `Cargo.lock`.

The program prints the number of assets, and the number of assets with each error code. Forms that need an encoder this target does not have report `encoder_unavailable`, and assets too large for the 32-bit address space report `too_large`. Those are listed with their error and have no members, so compare the other assets.

```bash
sekai-exporter export <input> <native>
python tools/compare-storage/determinism.py <native>/store <output>/store
```
