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

## 对象文档

`objects.mjs` 用同一个核心写出目录中每个资源包的对象文档，路径与 `sekai-exporter objects` 相同：`<资源包>/_objects.json`，以及每个超出内联上限的载荷 `_payloads/<aa>/<sha256>.bin`。两者的输出逐字节相同。

```bash
sekai-exporter objects <输入目录> <原生输出>
node tools/wasm-runner/objects.mjs wasm-package <输入目录> <输出目录>
diff -r <原生输出> <输出目录>
```

程序列出无法读取或写出的资源包，此时以状态 1 退出。
