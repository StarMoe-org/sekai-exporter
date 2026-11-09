# Third-party crates with local changes

English | [简体中文](README.zh-CN.md)

Canonical files must be the same bytes on every target, including `wasm32`. These crates are used through `[patch.crates-io]` with the changes below, and are otherwise the published sources.

| Crate | Version | Change |
|---|---|---|
| `image-webp` | 0.2.4 | When Huffman code lengths are limited, symbols of equal frequency are ordered by symbol. |
| `flacenc` | 0.5.1 | `cos`, `log2` and `powf` come from the `libm` crate, whose results do not depend on the target. |
