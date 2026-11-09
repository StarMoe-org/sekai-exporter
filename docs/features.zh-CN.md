# Feature

[English](features.md)

导出器由 Cargo feature 组装而成。每个转换器族、视图、编码后端与输入来源都是一个 feature，因此一次构建恰好包含其所需的部分。各 crate 的说明参见[架构](architecture.zh-CN.md#crate)。

## 规则

1. **一个 feature 对应一个转换器族、一个视图、一个后端或一个来源。** feature 启用一个完整的模块（`#[cfg(feature = "x")] mod x;`）或一项可选依赖。函数体内不出现 `#[cfg]`。
2. **feature 只做加法。** 任何两个 feature 都不互斥。同一格式的多个后端都编译进来时，由配置选择其一，所选后端计入其影响的每一个指纹。
3. **feature 之间的依赖在 `Cargo.toml` 中声明。** 启用一个 feature 即启用它所需的 feature。
4. **未编译进构建的转换器予以报告，而非跳过。** 其本应产出的每个资产以 `not_compiled` 状态及该转换器的名称记录。关闭某个 feature 不会导致输出被静默移除。
5. **不提供运行期插件接口。** 扩展导出器的方式是新增一个模块与一个 feature。
6. **持续集成将每个 feature 单独构建一次**（`cargo hack check --each-feature`），并分别构建不启用默认 feature、启用默认 feature 与启用全部 feature 三种组合。依赖其他 feature 而未声明的 feature 无法通过这项检查。

## 转换器族

`sekai-convert` 中适用于一般 Unity 内容的转换器族：

| Feature | 转换的内容 |
|---|---|
| `image` | `Texture2D`、`Sprite` 与精灵图集，转换为图片 |
| `audio` | `AudioClip` 与 CRI ACB/AWB 的 cue，转换为音频文件与 cue 表 |
| `video` | 通过 `ffmpeg` 程序（需带 `libx264`，不随导出器分发）把 CRI USM 文件转换为 H.264 与 AAC 的 MP4 |
| `model` | 预制体层级、网格、蒙皮与材质，转换为 glTF |
| `animation` | `AnimationClip`，转换为曲线文档；绑定关系允许时同时转换为 glTF 动画 |
| `live2d` | Live2D Cubism 模型及其动作 |
| `ugui` | UI 预制体，转换为布局文档 |
| `text` | `TextAsset` 与字体 |
| `timeline` | Timeline 资产，转换为时间轴文档 |
| `particles` | 粒子系统，转换为粒子文档 |

`sekai-moesekai` 中本游戏专有、服务于默认使用者的转换器族：

| Feature | 转换或产出的内容 |
|---|---|
| `scenario` | 剧本对象，转换为剧本文档 |
| `episode` | 剧情的一话：一条数据表记录，以及其所引用的全部已解析资产 |
| `virtual-live` | 虚拟演唱会：时间轴、剧本与语音 |
| `costume` | 供 3D 浏览的服装组合 |
| `storage-view` | [storage 视图](publishing.zh-CN.md#storage-视图) |

`sekai-mysekai` 中只有 `mysekai` 一个 feature，按复刻游戏的运行时所需的完整形式产出 MySekai 的站点、家具与现象。

## 后端

`sekai-encode` 中的后端：

| Feature | 格式 | 库 |
|---|---|---|
| （始终启用） | PNG | `png` |
| （始终启用） | 无损 WebP，图片的规范形式 | `image-webp` |
| （始终启用） | OpenEXR，规范 HDR 图像形式 | `exr` |
| （始终启用） | FLAC，音频的规范形式 | `flacenc` |
| `webp` | 有损 WebP | libwebp，静态链接 |
| `mp3` | MP3 | LAME，运行时加载 |
| `aac` | MP4 中的 AAC 音轨 | libavcodec 59 至 63 及同一 FFmpeg 版本的 libavutil（FFmpeg 5.1 至 9.0），在 64 位平台上运行时加载 |

规范形式的编码器以纯 Rust 编写，因此原生构建与 WebAssembly 构建产出相同的规范字节。依赖 C 库的后端仅在原生构建中可用。默认构建包含 `mp3` 与 `aac`，因此运行时需要系统中安装 LAME 与 libavcodec；导出器在启动时检查二者，并指明缺失的库。这些库的许可证列于[第三方来源](third-party.zh-CN.md)。

## 来源与发布

| Feature | Crate | 启用的内容 |
|---|---|---|
| `cdn` | `sekai-game` | `cp` 与 `nuverse` 来源，含 HTTP 与 TLS |
| `client` | `sekai-game`、`sekai-pipeline` | `apk` 来源，以及读取客户端安装包内资产所用的字段布局表（`sekai-layouts`） |
| `catalog-db` | `sekai-catalog` | 保存于 redb 数据库的目录 |
| `s3` | `sekai-publish` | S3 兼容的 store |
| `serve` | `sekai-exporter` | 以 HTTP 服务形式运行的惰性求值器 |

## 预设

| 预设 | 内容 |
|---|---|
| `default` | 默认使用者所需的全部内容：`cdn`、`catalog-db`、全部通用转换器族、`scenario`、`episode`、`virtual-live`、`costume`、`storage-view`、`webp`、`mp3`、`aac` |
| `preset-mysekai` | `default` 加 `mysekai` |
| `preset-client` | `default` 加 `client` |
| `full` | 全部 feature |
