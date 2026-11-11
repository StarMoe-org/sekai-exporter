# Feature

[English](features.md)

导出器由 Cargo feature 组装而成。每个转换器族、视图、编码后端与输入来源都是一个 feature，因此一次构建恰好包含其所需的部分。各 crate 的说明参见[架构](architecture.zh-CN.md#crate)。

## 规则

1. **一个 feature 对应一个转换器族、一个视图、一个后端或一个来源。** feature 启用一个完整的模块（`#[cfg(feature = "x")] mod x;`）或一项可选依赖。函数体内不出现 `#[cfg]`。
2. **feature 只做加法。** 任何两个 feature 都不互斥。同一格式的多个后端都编译进来时，由配置选择其一，所选后端计入其影响的每一个指纹。
3. **feature 之间的依赖在 `Cargo.toml` 中声明。** 启用一个 feature 即启用它所需的 feature。
4. **未编译进构建的转换器予以报告，而非跳过。** 关闭一个其转换器已有根规则的 feature 时，这些转换器本应产出的每个资产记录为错误 `not_compiled`，并写明该转换器的名称。资产形式尚无根规则的 feature 列于[转换器族](#转换器族)；它们不产出任何内容，因此关闭时不会移除任何内容。
5. **不提供运行期插件接口。** 扩展导出器的方式是新增一个模块与一个 feature。
6. **持续集成将每个 feature 单独构建一次**（`cargo hack check --each-feature`），并分别构建不启用默认 feature、启用默认 feature 与启用全部 feature 三种组合。依赖其他 feature 而未声明的 feature 无法通过这项检查。

## 转换器族

`sekai-convert` 中适用于一般 Unity 内容的转换器族：

| Feature | 转换的内容 |
|---|---|
| `image` | `Texture2D`、`Sprite` 与精灵图集，转换为图片 |
| `audio` | CRI ACB/AWB 的 cue，转换为音频文件与 cue 表 |
| `video` | 通过 `ffmpeg` 程序（需带 `libx264`，不随导出器分发）把 CRI USM 文件转换为 H.264 与 AAC 的 MP4 |
| `model` | 预制体层级、网格、蒙皮与材质，转换为 glTF |
| `animation` | `AnimationClip`，转换为曲线文档；绑定关系允许时同时转换为 glTF 动画 |
| `live2d` | Live2D Cubism 模型及其动作集，形式见 [Live2D](spec/live2d.zh-CN.md) |
| `ugui` | UI 预制体，转换为布局文档 |
| `text` | `TextAsset` 与字体 |
| `timeline` | Timeline 资产，转换为时间轴文档 |
| `particles` | 粒子系统，转换为粒子文档 |

`ugui`、`timeline` 与 `particles` 这几个 feature 已定义，目前尚不转换任何内容。这三种资产形式没有根规则，因此不含它们的构建不会以 `not_compiled` 报告任何内容，如规则 4 所述。`model` 把预制体读成[模型文件](spec/model.zh-CN.md)与[组件文档](spec/component-documents.zh-CN.md)，`animation` 把 `AnimationClip` 解码为[剪辑文档](spec/clip-documents.zh-CN.md)。Live2D、服装与 MySekai 形式包含它们；这两个转换器族目前都不单独产出资产，也不产出 glTF 动画。`audio` 目前尚不转换 `AudioClip`。

`sekai-moesekai` 中本游戏专有、服务于默认使用者的转换器族：

| Feature | 转换或产出的内容 |
|---|---|
| `scenario` | 剧本对象，转换为剧本文档 |
| `episode` | 剧情的一话：一条数据表记录，以及其所引用的全部已解析资产 |
| `virtual-live` | 虚拟演唱会的[剧本](spec/virtual-live-script.zh-CN.md) |
| `costume` | 供 3D 浏览的服装部件、动作与组合，见[服装形式](spec/costume.zh-CN.md) |

`costume` 产出 `costume-model` 与 `costume-motion` 两种形式，并由游戏数据表产出 `costume-role` 形式（每个角色在每个单元一份，含其可穿戴的全部部件）和 `costume` 形式（每套服装的各个造型）。

[storage 视图](publishing.zh-CN.md#storage-视图)是视图而非转换器，不写出任何资产。不含 `storage-view` 的构建会拒绝 `update --storage`，并且不会以 `not_compiled` 报告任何资产。

`sekai-mysekai` 中只有 `mysekai` 一个 feature，按复刻游戏的运行时所需的完整形式产出 MySekai 的各种形式：[家具模型](spec/fixture-model.zh-CN.md)与[场地模型](spec/site-model.zh-CN.md)、家具与站点文档、角色动作与对话、场地相机，每种在[规格](spec/README.zh-CN.md)中各有一页。不含它的构建会把其对象形式的根以 `not_compiled` 报告。

## 后端

`sekai-encode` 中的后端：

| Feature | 格式 | 库 |
|---|---|---|
| （始终启用） | PNG，图片的规范形式 | `png` |
| （始终启用） | OpenEXR，规范 HDR 图像形式 | `exr` |
| （始终启用） | MP3，音频的规范形式 | `rusty_mp3` |
| `webp` | 有损 WebP | libwebp，静态链接 |

规范形式的编码器以纯 Rust 编写，因此原生构建与 WebAssembly 构建产出相同的规范字节。有损 WebP 后端链接 C 库 libwebp，仅在原生构建中可用。各库的许可证列于[第三方来源](third-party.zh-CN.md)。

## 来源与发布

| Feature | Crate | 启用的内容 |
|---|---|---|
| `cdn` | `sekai-game` | `cp` 与 `nuverse` 来源，含 HTTP 与 TLS |
| `client` | `sekai-game`、`sekai-exporter` | `apk` 来源，以及读取客户端安装包内资产所用的字段布局表（`sekai-layouts`） |
| `catalog-db` | `sekai-catalog` | 保存于 redb 数据库的目录 |
| `s3` | `sekai-publish` | S3 兼容的 store |
| `serve` | `sekai-exporter` | `serve` 命令：解析已发布视图路径的 HTTP 服务（[服务](serving.zh-CN.md)） |
| `server` | `sekai-serve` | `serve` 命令背后的 HTTP 服务，使用 `tokio` 与 `hyper`。不启用时该 crate 不做 I/O |

## 预设

| 预设 | 内容 |
|---|---|
| `default` | 默认使用者所需的全部内容：`cdn`、`catalog-db`、`image`、`audio`、`video`、`model`、`animation`、`live2d`、`text`、`scenario`、`episode`、`virtual-live`、`costume`、`storage-view`、`webp` |
| `preset-mysekai` | `default` 加 `mysekai` |
| `preset-client` | `default` 加 `client` |
| `full` | 全部 feature |
