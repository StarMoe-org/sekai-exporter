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
| TPK 文件格式的引擎类布局 | [Tpk](https://github.com/AssetRipper/Tpk)、[UnityPy](https://github.com/K0lb3/UnityPy) | 文件格式及按版本选择布局的参考；布局逐节点比对。类表文件由使用者提供，不随本项目分发 | MIT |
| 本游戏所用的资源包头部去混淆 | [sssekai](https://github.com/MetaSekaiLab/sssekai) | 参考：该变换在其中有公开说明 | MIT |

## 贴图

| 组件 | 来源 | 使用方式 | 许可证 |
|---|---|---|---|
| ASTC 解码（LDR 与 HDR） | Khronos ASTC 规范；[astc-encoder](https://github.com/ARM-software/astc-encoder) | 规范；参考：解码值逐位比对 | Apache-2.0（astc-encoder） |
| crunch DXT5 解码 | [crunch](https://github.com/Unity-Technologies/crunch)（`unity` 分支） | 流布局译自 `crn_decomp.h`；参考：比对解包后的块 | ZLIB |
| BC3 解码 | Khronos Data Format Specification | 规范；DXT 块按其公式以精确算术比对 | — |
| ETC1、ETC2 与 EAC 解码 | Khronos Data Format Specification；[texture2ddecoder](https://github.com/K0lb3/texture2ddecoder) | 规范；参考：解码像素逐字节比对 | MIT（texture2ddecoder） |
| BC1 至 BC7 解码 | Khronos Data Format Specification；Microsoft 块压缩文档 | 规范 | — |
| PNG 写出与读取（图片的规范形式） | [`png`](https://crates.io/crates/png) | 链接 | MIT OR Apache-2.0 |
| 有损 WebP 写出 | [libwebp](https://chromium.googlesource.com/webm/libwebp)，经由 [`webp`](https://crates.io/crates/webp) 与 [`libwebp-sys`](https://crates.io/crates/libwebp-sys) | 在原生构建中静态链接 | BSD-3-Clause（libwebp），MIT（`libwebp-sys`） |

## 音频与视频

| 组件 | 来源 | 使用方式 | 许可证 |
|---|---|---|---|
| HCA 与 ADX 解码 | [vgmstream](https://github.com/vgmstream/vgmstream)（clHCA 与其 ADX 解码器） | 移植：解码器沿用其结构，常量由其生成，声明保留在生成的文件中；解码结果逐采样比对 | ISC |
| CRI ACB 表、AWB 归档 | [vgmstream](https://github.com/vgmstream/vgmstream) | 参考：与其读取和解码的结果比对 | ISC |
| USM 解复用 | [WannaCRI](https://github.com/donmai-me/WannaCRI) | 参考：分离出的流逐字节比对 | MIT |
| HCA 测试样例 | [VGAudio](https://github.com/Thealexbarney/VGAudio) | 用于编码生成的音频，不链接也不分发 | MIT |
| OpenEXR 写出（规范 HDR 图像形式） | [`exr`](https://crates.io/crates/exr) | 链接 | BSD-3-Clause |
| MP3 写出（规范音频形式） | [`rusty_mp3`](https://crates.io/crates/rusty_mp3)，修改见 [`third_party`](../third_party/README.zh-CN.md)；[`libm`](https://crates.io/crates/libm) | 链接 | Apache-2.0；MIT |
| 测试中的 MP3 读取 | [`symphonia`](https://crates.io/crates/symphonia) | 用于测试，解码写出的 MP3 并与其采样比对；不链接进导出器 | MPL-2.0 |
| AAC 写出 | [FFmpeg libavcodec](https://ffmpeg.org/) | 在原生构建中运行时加载，不随导出器分发 | LGPL-2.1-or-later |
| 运行时加载 libavcodec | [`libloading`](https://crates.io/crates/libloading) | 原生构建中链接 | ISC |
| MP4 音轨的 AAC 写出 | [FFmpeg](https://ffmpeg.org/) libavcodec | 在原生构建中运行时加载，不随导出器分发 | 不含 GPL 组件构建时为 LGPL-2.1-or-later |
| H.264 与 AAC 的 MP4 写出 | [FFmpeg](https://ffmpeg.org/) 与 [x264](https://www.videolan.org/developers/x264.html) | 作为独立程序运行；不链接，也不随导出器分发 | GPL-2.0-or-later |

## 模型与文档

| 组件 | 来源 | 使用方式 | 许可证 |
|---|---|---|---|
| glTF 写出 | Khronos glTF 2.0 规范；[glTF Validator](https://github.com/KhronosGroup/glTF-Validator) | 规范；写出的每个文件均须通过校验器且无错误 | Apache-2.0（校验器） |
| JSON | [`serde`](https://crates.io/crates/serde)、[`serde_json`](https://crates.io/crates/serde_json) | 链接 | MIT OR Apache-2.0 |

## Live2D

| 组件 | 来源 | 使用方式 | 许可证 |
|---|---|---|---|
| [`compare-live2d-motion`](../tools/compare-live2d-motion/README.zh-CN.md) 中 `motion3.json` 曲线的求值 | [Cubism Web Framework](https://github.com/Live2D/CubismWebFramework) `5-r.5` | 参考：曲线由其 `evaluateCurve` 求值，框架取自使用者提供的检出；不随本项目分发 | Live2D Open Software License |
| `motion3.json` 分段到 Unity 关键帧的映射 | [Cubism SDK for Unity](https://github.com/Live2D/CubismUnityComponents) `5-r.5`（`CubismMotion3Json`） | 阅读其映射，见 [Live2D 动作](live2d-motion.zh-CN.md)；未复制代码 | Live2D Open Software License |
| 为比较打包框架 | [`esbuild`](https://www.npmjs.com/package/esbuild) | 随比较脚本安装；不随本项目分发 | MIT |

## 输入、存储与命令行

| 组件 | 来源 | 使用方式 | 许可证 |
|---|---|---|---|
| 清单与接口解密 | [`aes`](https://crates.io/crates/aes)、[`cbc`](https://crates.io/crates/cbc) | 链接 | MIT OR Apache-2.0 |
| Base64 | [`base64`](https://crates.io/crates/base64) | 随 `cdn` feature 链接，用于读取账号凭据中的用户 ID | MIT 或 Apache-2.0 |
| JavaScript 绑定 | [`wasm-bindgen`](https://crates.io/crates/wasm-bindgen)、[`js-sys`](https://crates.io/crates/js-sys) | 在 `wasm32` 上链接 | MIT 或 Apache-2.0 |
| MessagePack | [`rmp-serde`](https://crates.io/crates/rmp-serde) | 链接 | MIT |
| 哈希 | [`sha2`](https://crates.io/crates/sha2) | 链接 | MIT OR Apache-2.0 |
| 节点路径的 CRC-32 | [`crc32fast`](https://crates.io/crates/crc32fast) | 启用 `model` feature 时链接 | MIT OR Apache-2.0 |
| HTTP 与 TLS | [`reqwest`](https://crates.io/crates/reqwest)、[`hyper`](https://crates.io/crates/hyper)、[`h2`](https://crates.io/crates/h2)、[`tokio`](https://crates.io/crates/tokio)、[`ureq`](https://crates.io/crates/ureq)、[`rustls`](https://crates.io/crates/rustls)、[`ring`](https://crates.io/crates/ring)、[`webpki-roots`](https://crates.io/crates/webpki-roots)、[`socks`](https://crates.io/crates/socks) | 启用 `cdn` 或 `s3` feature 时链接 | MIT OR Apache-2.0；Apache-2.0 OR ISC OR MIT；Apache-2.0 AND ISC；CDLA-Permissive-2.0；MIT OR Apache-2.0 |
| 目录 | [`redb`](https://crates.io/crates/redb) | 启用 `catalog-db` feature 时链接 | MIT OR Apache-2.0 |
| 运行时加载库 | [`libloading`](https://crates.io/crates/libloading) | 链接 | ISC |
| 命令行解析 | [`lexopt`](https://crates.io/crates/lexopt) | 链接 | MIT |
| CPU 与内存采样 | [`sysinfo`](https://crates.io/crates/sysinfo)、[`cpu-time`](https://crates.io/crates/cpu-time) | 链接进命令行程序 | MIT；MIT OR Apache-2.0 |

## 运行时加载的库

libavcodec 不链接进导出器，也不随其分发。当一次运行需要 AAC 后端时，导出器加载系统中已安装的库并检查其版本，将后端名称、版本与参数记入受影响输出的指纹。库缺失或版本不符时，运行以注明所缺库的错误终止，不改用其他后端。
