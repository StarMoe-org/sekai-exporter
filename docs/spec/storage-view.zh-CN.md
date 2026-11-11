# storage 视图

`storage` 视图把一个版本的资产写成资源存储站提供的路径树。模块：`sekai-moesekai` 的 `storage_view`（feature `storage-view`）。`sekai-exporter update` 以 `<区服>/storage` 下的路径索引发布该视图，`--storage <目录>` 还会把它写成路径树。视图版本：2。

## 1. 运行时

按路径读取资源存储站文件的网站与播放器。

## 2. 内容

| 资产形式 | 文件 |
|---|---|
| `texture`、`sprite` | 图像的 `<file>.png` 与 `<file>.webp` |
| `audio` | 每个 cue 一个 `<cue>.mp3`；有多个波形的 cue，其余为 `<cue>_2.mp3` 等 |
| `video` | `<base>.mp4` |
| `scenario` | `<file>.json` |
| `text` | 见下文 |
| `font` | `<file>.<扩展名>` |

其他成员（如 Sprite 文档）不属于该视图。`music/long` 下名称以 `_VR` 或 `_SCREEN` 结尾的 cue 不进入视图。

## 3. 来源

每个文件来自资产文档的一个成员。文件位于 `<bundle>/<名称>`，其中 `<bundle>` 是资产根对象所在的资源包，`<file>` 是资产名称的最后一段。`<base>` 是最后一段去掉 `.usm` 和形如 `-001` 的数字分段后缀。

以 `.bytes` 为扩展名存储的文本资产去掉该扩展名；位于 `music/music_score` 下的文件命名为 `<file>.txt`。其他扩展名的文本资产保留扩展名。

共享名称的资产遵循[架构](../architecture.zh-CN.md)中的命名规则。多个资产落到同一路径时，资产名称排序最前者保留该路径，其余不进入视图。

## 4. 容器

| 文件 | 参数 |
|---|---|
| PNG | 规范层成员，不作改动 |
| WebP | PNG 成员的像素，有损，libwebp 质量因子 75、方法 5，`webp` 编码器 |
| MP3 | 规范成员，原样不变；见[音频](audio.zh-CN.md) |
| MP4 | 规范层成员，不作改动；见[视频](video.zh-CN.md) |
| JSON 与文本 | 规范层成员，不作改动 |

变换也是派生：其指纹由视图版本、变换、编码器版本与成员哈希构成。第二次运行复用全部结果。

[解析器](../serving.zh-CN.md)让视图的所有路径同时切换到新版本。用 `--storage` 写出的路径树无法原子切换，其写入顺序见[发布](../publishing.zh-CN.md)。

## 5. 正确性

将视图与由相同输入生成的参照树比较：文档逐值，图像比较解码后的像素，有损媒体比较格式契约（编码、容器、采样率、声道、时长差不超过一帧、码率档）。比较由 `tools/compare-storage` 完成。

## 6. 版本

| 版本 | 变更 |
|---|---|
| 1 | 第一版 |
| 2 | 图片为规范 PNG 及由其生成的 WebP；声音为规范 MP3 |
