# sekai-exporter

[![ci](https://github.com/StarMoe-org/sekai-exporter/actions/workflows/ci.yml/badge.svg)](https://github.com/StarMoe-org/sekai-exporter/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.99+](https://img.shields.io/badge/rust-1.99%2B-orange.svg)](https://www.rust-lang.org)

[English](README.md)

面向《世界计划 彩色舞台 feat. 初音未来》（Project SEKAI COLORFUL STAGE! feat. Hatsune Miku）的导出器。它读取游戏的 Unity 资源包与客户端安装包，将其中的内容转换为开放格式的资产，包括图片、音频、视频、模型、剧本、曲线与各类文档。每一项输出都记录其来源对象，使用者可据此查明任意文件的含义。

> [!TIP]
> **请先阅读[对象与资产](docs/objects-and-assets.zh-CN.md)。** 它说明导出 Unity 内容意味着什么、本项目为何以转换而非解包为核心，以及默认产出哪些内容。想了解本仓库的工作，从这篇读起最快。

## 特点

- **完整的资产，而非零散的对象**：跨资源包沿引用找齐一个资产所需的对象，按某个具体运行时读取的形式整体写出，例如带骨骼、材质与贴图的 GLB 模型，或带贴图、物理与动作的 Live2D 模型。
- **可追溯的输出**：每个文件都记录其来源的资源包、对象与转换器。
- **仅重建发生变化的部分**：每一项输出都是以内容寻址的输入为参数的纯函数的结果，因此未变化的工作会被跳过，各区服之间相同的内容只保存一份。
- **纯 Rust**：解码器与规范编码器以不含 `unsafe` 代码的 Rust 编写，核心可构建为 WebAssembly，输出与原生构建逐字节相同。
- **发布到任意位置**：结果发布为带有自描述索引的内容寻址 store，或普通的路径树，目标可以是本地目录或 S3 兼容存储。

## 快速开始

构建命令行程序：

```bash
cargo build --release -p sekai-exporter
```

为目录中的每个资源包写出对象文档：

```bash
sekai-exporter objects <资源包目录> <输出目录>
```

把目录中的每个资源包转换为资产：

```bash
sekai-exporter export <资源包目录> <输出目录>
```

把输出目录更新到某个区服的当前版本：

```bash
sekai-exporter update <区服> <输出目录> --config <配置文件> --secrets <凭据目录>
```

不带参数运行 `sekai-exporter` 会列出全部命令与选项。区服的配置与凭据见[来源](docs/sources.zh-CN.md)，Cargo feature 与预设见[功能开关](docs/features.zh-CN.md)。

## 状态

开发中。目前可用的部分：

| 方面 | 状态 |
|---|---|
| 解包 | `objects` 为每个资源包写出[对象文档](docs/spec/object-documents.zh-CN.md)，其字段值在持续集成中与 UnityPy 相比对 |
| 解码 | Alpha8、R8、RGB24、RGBA32、RGB565、ETC1、ETC2 RGB、ETC2 RGBA8、EAC R、crunch DXT5、ASTC LDR 与 ASTC HDR 贴图（含贴图数组与 3D 贴图），HCA 与 ADX 音频，ACB 与 AWB 归档，以及 USM 容器 |
| 编码 | PNG、有损 WebP、OpenEXR 与 MP3，以及经 `ffmpeg` 程序写出的 MP4 |
| 转换器 | 贴图、精灵、字体、文本资产、剧情场景、动作集、剧情背景、虚拟演唱会剧本、CRI 视频、CRI cue 表、Live2D 模型与动作集、服装部件与动作，以及 MySekai 家具、场地模型、表情、角色动作与对话。[功能开关](docs/features.zh-CN.md)列出了每个 feature 转换什么、哪些还不转换任何内容 |
| 求值与发布 | `export` 和 `update` 对资源包求值，写入本地内容寻址 store，并把结果发布为索引、路径树或 `storage` 视图，目标可以是本地目录或 S3 兼容存储 |
| WebAssembly | 核心可构建为 `wasm32`，持续集成会把它写出的对象文档和 store 与原生构建的逐字节比对 |

[路线图](docs/roadmap.zh-CN.md)列出了每个部分的状态和尚未完成的工作。

## 文档

[`docs/`](docs/) 中的文档描述了代码所实现的设计。

| 文档 | 内容 |
|---|---|
| [对象与资产](docs/objects-and-assets.zh-CN.md) | **从这里开始。** 导出 Unity 内容的含义，以及本项目定位为导出器的原因 |
| [架构](docs/architecture.zh-CN.md) | 派生模型、求值策略与 crate 结构 |
| [管线](docs/pipeline.zh-CN.md) | 一次运行的各个阶段及每个阶段保持的不变式 |
| [性能](docs/performance.zh-CN.md) | 设计如何避免工作，以及其余工作在何处运行 |
| [来源](docs/sources.zh-CN.md) | 输入、区服、版本发现与凭据 |
| [发布](docs/publishing.zh-CN.md) | store、布局、资产文档与发布协议 |
| [功能开关](docs/features.zh-CN.md) | crate、Cargo feature 与预设 |
| [路线图](docs/roadmap.zh-CN.md) | 每个部分的状态，以及如何认领未完成的工作 |
| [Live2D 动作](docs/live2d-motion.zh-CN.md) | 游戏如何保存 Live2D 动作，以及其规范形式为何是 clip 的曲线 |
| [规范](docs/spec/README.zh-CN.md) | 每种资产形式与每种发布文档格式的规范 |
| [术语表](docs/glossary.md) | 文档与代码中统一使用的术语 |
| [第三方来源](docs/third-party.zh-CN.md) | 每个组件所依据的上游规范、库或参考实现 |
| [贡献指南](CONTRIBUTING.zh-CN.md) | 改动的位置、规则与检查 |

## 游戏数据

本仓库不包含任何游戏数据。输入读取自使用者自行下载或安装的客户端，输出写在仓库之外。测试使用合成输入。

## 许可

MIT，参见 [LICENSE](LICENSE)。
