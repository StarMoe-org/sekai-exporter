# WebAssembly 运行器

[English](README.md) | 简体中文

在 Node 下用 WebAssembly 核心求值一个目录中的资源包，写出与 `sekai-exporter export` 相同的 `store` 目录和 `assets.tsv`，两者可以逐字节比较。

```bash
cargo build --release -p sekai-wasm --target wasm32-unknown-unknown
wasm-bindgen --target nodejs --out-dir wasm-package \
  target/wasm32-unknown-unknown/release/sekai_wasm.wasm
node tools/wasm-runner/run.mjs wasm-package <输入目录> <输出目录>
```

`wasm-bindgen` 程序的版本须与 `Cargo.lock` 中 `wasm-bindgen` crate 的版本一致。

程序输出资产数量以及每种错误代码的资产数量。需要当前目标不具备的编码器的形式报告 `encoder_unavailable`，超出 32 位地址空间的资产报告 `too_large`。这些资产带错误列出且没有成员，比较时只比较其余资产。

```bash
sekai-exporter export <输入目录> <原生输出>
python tools/compare-storage/determinism.py <原生输出>/store <输出目录>/store
```
