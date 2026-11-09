# sekai-exporter

[English](README.md)

面向《世界计划 彩色舞台 feat. 初音未来》（Project SEKAI COLORFUL STAGE! feat. Hatsune Miku）的导出器。它读取游戏的 Unity 资源包与客户端安装包，将其中的内容转换为开放格式的资产，包括图片、音频、视频、模型、剧本、曲线与各类文档。每一项输出都记录其来源对象，使用者可据此查明任意文件的含义。

## 状态

开发中。解包已可用：`sekai-exporter objects <资源包目录> <输出目录>` 为目录中的每个资源包写出[对象文档](docs/spec/object-documents.zh-CN.md)，其字段值在持续集成中与 UnityPy 相比对。`sekai-codec` 以纯 Rust 解码 Alpha8、RGB24、RGBA32、RGB565、ETC1、ETC2 RGBA8、crunch DXT5、ASTC LDR 与 ASTC HDR 贴图，HCA 与 ADX 音频，ACB 与 AWB 归档，以及 USM 容器。`sekai-encode` 写出 PNG、无损与有损 WebP、OpenEXR、FLAC、MP3 和 AAC。贴图、精灵、字体、文本资产、剧情场景、CRI 视频、CRI cue 表以及 Live2D 模型与动作集的转换器已经可用，[功能开关](docs/features.zh-CN.md)列出了每个 feature 转换什么、哪些还不转换任何内容。`sekai-exporter export` 和 `update` 逐个资源包求值，写入本地内容寻址 store，并把结果发布为索引、路径树或 `storage` 视图。核心也可构建为 WebAssembly，持续集成会比对它与原生构建的产物。[`docs/`](docs/) 中的文档描述了代码所实现的设计。

## 功能

- **解包**：将资源包与客户端的 player data 还原为精确的对象图，并解析全部跨文件引用。
- **转换**：将对象闭包转换为资产，每种资产形式对应一个转换器。默认形式即 [Moesekai](https://github.com/StarMoe-org/Moesekai) 网站及其查看器所消费的形式，其他形式作为可选 feature 提供。
- **发布**：将结果发布为带有自描述索引的内容寻址 store，或发布为普通的路径树，目标可以是本地目录或 S3 兼容存储。
- **仅重建发生变化的部分**：每一项输出都是以内容寻址的输入为参数的纯函数的结果，因此未变化的工作会被跳过，各区服之间相同的内容只保存一份。

本项目以转换而非解包为核心的原因，参见[对象与资产](docs/objects-and-assets.zh-CN.md)。

## 文档

| 文档 | 内容 |
|---|---|
| [对象与资产](docs/objects-and-assets.zh-CN.md) | 导出 Unity 内容的含义，以及本项目定位为导出器的原因 |
| [架构](docs/architecture.zh-CN.md) | 派生模型、求值策略与 crate 结构 |
| [管线](docs/pipeline.zh-CN.md) | 一次运行的各个阶段及每个阶段保持的不变式 |
| [性能](docs/performance.zh-CN.md) | 设计如何避免工作，以及其余工作在何处运行 |
| [来源](docs/sources.zh-CN.md) | 输入、区服、版本发现与凭据 |
| [发布](docs/publishing.zh-CN.md) | store、布局、资产文档与发布协议 |
| [Feature](docs/features.zh-CN.md) | crate、Cargo feature 与预设 |
| [Live2D 动作](docs/live2d-motion.zh-CN.md) | 游戏如何保存 Live2D 动作，以及其规范形式为何是 clip 的曲线 |
| [对象文档](docs/spec/object-documents.zh-CN.md) | 对象文档的规范 |
| [贴图](docs/spec/texture.md) | 贴图形式的规范（英文） |
| [Live2D](docs/spec/live2d.zh-CN.md) | Live2D 模型与动作集形式的规范 |
| [剪辑文档](docs/spec/clip-documents.zh-CN.md) | 动画剪辑文档的规范 |
| [术语表](docs/glossary.md) | 文档与代码中统一使用的术语 |
| [第三方来源](docs/third-party.zh-CN.md) | 每个组件所依据的上游规范、库或参考实现 |
| [贡献指南](CONTRIBUTING.zh-CN.md) | 改动的位置、规则与检查 |

## 游戏数据

本仓库不包含任何游戏数据。输入读取自使用者自行下载或安装的客户端，输出写在仓库之外。测试使用合成输入。

## 许可

MIT，参见 [LICENSE](LICENSE)。
