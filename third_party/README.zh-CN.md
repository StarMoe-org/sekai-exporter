# 带本地修改的第三方 crate

[English](README.md) | 简体中文

规范层文件在所有目标上（包括 `wasm32`）都必须是相同的字节。这些 crate 通过 `[patch.crates-io]` 使用，除下列修改外与发布版源码相同。

| Crate | 版本 | 修改 |
|---|---|---|
| `image-webp` | 0.2.4 | 限制 Huffman 码长时，频次相同的符号按符号排序。 |
| `flacenc` | 0.5.1 | `cos`、`log2`、`powf` 改用 `libm` crate，其结果不依赖目标平台。 |
