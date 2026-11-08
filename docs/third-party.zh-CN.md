# 第三方来源

[English](third-party.md)

导出器的每个组件都依据一项上游来源：格式规范、所链接的库，或用于核对输出的参考实现。本文按组件列出这些来源，并为每一项注明使用方式：

- **链接**：该库是某个 crate 的依赖，按其自身的许可证分发。
- **规范**：该组件依据格式说明为本项目编写。
- **参考**：该组件为本项目编写，其输出与所列实现逐值核对，不复制其代码。
- **移植**：代码在其许可证允许的范围内由所列项目翻译而来，被移植的文件保留该项目的版权声明。

条目与对应组件在同一次改动中加入，并在移除该组件的改动中删除。

## Unity 文件

| 组件 | 来源 | 使用方式 | 许可证 |
|---|---|---|---|
| UnityFS 容器、SerializedFile、type tree | [UnityPy](https://github.com/K0lb3/UnityPy) | 参考：对象字段逐值比对 | MIT |
| 类布局与读取顺序 | [AssetStudio](https://github.com/Perfare/AssetStudio)，及其维护分支 [AssetStudioMod](https://github.com/aelurum/AssetStudio) | 读取器结构的参考 | MIT |
| 引擎序列化规则 | [UnityCsReference](https://github.com/Unity-Technologies/UnityCsReference) | 用于查阅序列化字段的语义，不复制代码 | Unity Reference-Only License |
| type tree 公共字符串表 | Unity 编辑器 2022.3.62f2（`UnityFileSystemApi.dll`） | 复制：type tree 按偏移引用的引擎字段名与类型名清单，UnityPy 分发同一张表 | 标识符清单，不适用 |
| LZ4 块解压 | [`lz4_flex`](https://crates.io/crates/lz4_flex) | 链接 | MIT |
| LZMA 块解压 | [`lzma-rs`](https://crates.io/crates/lzma-rs) | 链接 | MIT |
| 按类 ID 的引擎类名 | [UnityPy](https://github.com/K0lb3/UnityPy) `ClassIDType` | 移植：该表由其生成，生成文件中保留其版权声明 | MIT |
| 本游戏所用的资源包头部去混淆 | [sssekai](https://github.com/MetaSekaiLab/sssekai) | 参考：该变换在其中有公开说明 | MIT |

## 贴图

| 组件 | 来源 | 使用方式 | 许可证 |
|---|---|---|---|
| ASTC 解码 | Khronos ASTC 规范；[astc-encoder](https://github.com/ARM-software/astc-encoder) | 规范；参考：解码像素逐字节比对 | Apache-2.0（astc-encoder） |
| ETC1、ETC2 与 EAC 解码 | Khronos Data Format Specification；[texture2ddecoder](https://github.com/K0lb3/texture2ddecoder) | 规范；参考：解码像素逐字节比对 | MIT（texture2ddecoder） |
| BC1 至 BC7 解码 | Khronos Data Format Specification；Microsoft 块压缩文档 | 规范 | — |
| PNG 写出 | [`png`](https://crates.io/crates/png) | 链接 | MIT OR Apache-2.0 |
| 无损 WebP 写出（图片的规范形式） | [`image-webp`](https://crates.io/crates/image-webp) | 链接 | MIT OR Apache-2.0 |
| 有损 WebP 写出 | [libwebp](https://chromium.googlesource.com/webm/libwebp)，经由 [`libwebp-sys`](https://crates.io/crates/libwebp-sys) | 在原生构建中静态链接 | BSD-3-Clause（libwebp），MIT（`libwebp-sys`） |

## 音频与视频

| 组件 | 来源 | 使用方式 | 许可证 |
|---|---|---|---|
| HCA 解码 | [vgmstream](https://github.com/vgmstream/vgmstream) clHCA | 移植：解码器沿用其结构，常量由其生成，声明保留在生成的文件中；解码结果逐采样比对 | ISC |
| CRI ACB 表、AWB 归档、USM 解复用、ADX 解码 | [vgmstream](https://github.com/vgmstream/vgmstream) | 参考：与其读取和解码的结果比对 | ISC |
| HCA 测试样例 | [VGAudio](https://github.com/Thealexbarney/VGAudio) | 用于编码生成的音频，不链接也不分发 | MIT |
| FLAC 写出 | [`flacenc`](https://crates.io/crates/flacenc) | 链接 | Apache-2.0 |
| 测试中读取 FLAC | [`claxon`](https://crates.io/crates/claxon) | 仅在测试中链接：写出的 FLAC 解回后逐采样比对 | Apache-2.0 |
| MP3 写出 | [LAME](https://lame.sourceforge.io/) | 在原生构建中运行时加载，不随导出器分发 | LGPL-2.0-or-later |
| MP4 音轨的 AAC 写出 | [FFmpeg](https://ffmpeg.org/) libavcodec | 在原生构建中运行时加载，不随导出器分发 | 不含 GPL 组件构建时为 LGPL-2.1-or-later |
| MP4 容器 | ISO/IEC 14496-12 与 14496-14；VP9 的 ISO-BMFF 封装规范 | 规范 | — |

## 模型与文档

| 组件 | 来源 | 使用方式 | 许可证 |
|---|---|---|---|
| glTF 写出 | Khronos glTF 2.0 规范；[glTF Validator](https://github.com/KhronosGroup/glTF-Validator) | 规范；写出的每个文件均须通过校验器且无错误 | Apache-2.0（校验器） |
| JSON | [`serde`](https://crates.io/crates/serde)、[`serde_json`](https://crates.io/crates/serde_json) | 链接 | MIT OR Apache-2.0 |

## 输入、存储与命令行

| 组件 | 来源 | 使用方式 | 许可证 |
|---|---|---|---|
| 清单与接口解密 | [`aes`](https://crates.io/crates/aes)、[`cbc`](https://crates.io/crates/cbc) | 链接 | MIT OR Apache-2.0 |
| MessagePack | [`rmp-serde`](https://crates.io/crates/rmp-serde) | 链接 | MIT |
| 哈希 | [`sha2`](https://crates.io/crates/sha2) | 链接 | MIT OR Apache-2.0 |
| HTTP 与 TLS | [`ureq`](https://crates.io/crates/ureq)、[`rustls`](https://crates.io/crates/rustls) | 启用 `cdn` 或 `s3` feature 时链接 | MIT OR Apache-2.0；Apache-2.0 OR ISC OR MIT |
| 目录 | [`rusqlite`](https://crates.io/crates/rusqlite)，内置 SQLite | 链接 | MIT（`rusqlite`），公有领域（SQLite） |
| 运行时加载库 | [`libloading`](https://crates.io/crates/libloading) | 链接 | ISC |
| 命令行解析 | [`lexopt`](https://crates.io/crates/lexopt) | 链接 | MIT |

## 运行时加载的库

LAME 与 libavcodec 不链接进导出器，也不随其分发。当一次运行需要 MP3 或 AAC 后端时，导出器加载系统中已安装的库并检查其版本，将后端名称、版本与参数记入受影响输出的指纹。库缺失或版本不符时，运行以注明所缺库的错误终止，不改用其他后端。
