# 剧本预制体

[English](scenario-prefab.md)

`scenario-prefab` 形式是剧本中的一个特效或一个预制体背景，连同它引用的每个对象，写成记录这些对象存储字段的 JSON 文档；贴图写成图像，动画 clip 另外写成 clip 文档。转换器：`sekai-moesekai` 中的 `scenario-prefab`（feature `episode`）。形式版本：1。

## 1. 运行时

按游戏的方式绘制一话中特效与预制体背景的剧本播放器和导出器：它们实例化预制体，摆放其 transform 与 rect transform，用材质绘制其 image、raw image 与 sprite renderer，按模块模拟其粒子系统，用 controller 与 clip 运行其 animator，并显示背景的影片视图。它们按引擎的存储方式读取对象，因此本形式保留对象的字段，不加解释。

资产从游戏加载的对象开始。一话以资源包与名称指明一个特效（见 [`episode`](episode.zh-CN.md)），游戏从该资源包加载同名的预制体：特效资源包中的每个预制体都是一个资产。预制体背景经由其构建数据加载，即一个 `ScenarioBackgroundBuildData`，其 `BackgroundPrefab` 与 `ForegroundPrefab` 是游戏实例化的预制体：构建数据就是资产，因此它包含这两个预制体以及构建数据存储的其他字段。

## 2. 内容

| 角色 | 内容 |
|---|---|
| `prefab` | 文档（见下），扩展名 `json` |
| `texture/<n>` | 文档中第 `n` 个对象这张贴图的图像，同 [`texture`](texture.zh-CN.md) 形式 |
| `clip/<n>` | 第 `n` 个对象这个动画 clip，写成 [clip 文档](clip-documents.zh-CN.md)，扩展名 `json`；仅当 clip 文档能表示该 clip 时存在 |
| `bytes/<sha256>` | 大于 4096 字节的字节载荷（见下），扩展名 `bin` |

`prefab` 成员在最前，其后是按对象顺序排列的贴图与 clip，最后是按文档首次指明顺序排列的载荷。

```json
{ "format": "scenario-prefab", "version": 1, "objects": [
  { "file": "CAB-…", "id": "-4407209310344673407", "class": "GameObject", "class_id": 1, "name": "kirakira_01",
    "fields": { "m_Component": [{ "component": { "$ref": 1 } }], "m_Name": "kirakira_01", "…": "…" } },
  { "file": "CAB-…", "id": "-3998864310059250282", "class": "Shader", "class_id": 48, "name": "Sekai/Particles/Additive" },
  { "file": "CAB-…", "id": "8811209310344673407", "class": "Texture2D", "class_id": 28, "name": "tex_kirakira", "member": "texture/2",
    "fields": { "m_Width": 256, "…": "…" } }
] }
```

| 键 | 值 |
|---|---|
| `format`、`version` | `"scenario-prefab"` 与本文档格式的版本 1 |
| `objects` | 根对象在前，其后是遍历（§3）到达的每个对象，按到达顺序排列 |

每个对象：

| 键 | 值 |
|---|---|
| `file` | 存放该对象的序列化文件，例如 `CAB-0123456789abcdef` |
| `id` | path ID，写成十进制字符串 |
| `class`、`class_id` | 类名与类 ID |
| `script` | 引用脚本的对象：`"<assembly>/<namespace>.<class>"`。其他对象没有此键 |
| `name` | 顶层 `m_Name` 为非空字符串时取它；shader 取其解析形式中的名称；脚本取与 `script` 相同的全名 |
| `member` | 持有图像的 `Texture2D` 为 `texture/<n>`；clip 文档能表示的动画 clip 为 `clip/<n>`。其他对象没有此键 |
| `fields` | 存储的字段，写法见下。shader 与脚本没有此键 |

shader 与脚本只列出名称：引擎之外的渲染器按名称找到它们，不运行存储的程序。动画 clip 保留其存储字段，clip 文档能表示它时再以成员给出 clip 文档：clip 文档报告为 `unsupported` 的 clip（例如带整数曲线，或事件带对象参数）只有字段。`Texture2D` 保留除 `image data` 与 `m_StreamData` 之外的字段，其图像就是成员。

字段值的写法同[对象文档](object-documents.zh-CN.md#24-字段值)，区别如下，因为读取者看到的是值而不是存储类型：

| 值 | 写法 |
|---|---|
| 整数 | 数字；超出 ±2^53 时写成十进制字符串 |
| 浮点数 | 值是 32 位浮点数时，写成读回同一 32 位浮点数的最短十进制；否则写成读回同一 64 位浮点数的最短十进制；`"NaN"`、`"Infinity"` 与 `"-Infinity"` 写成字符串 |
| 指向文档中对象的引用 | `{"$ref": n}`，`n` 为它在 `objects` 中的位置 |
| 指向引擎自带资源中对象的引用 | `{"$ref": {"external": "<file>", "id": "<path ID>"}}`，例如 `Library/unity default resources` 的内置 quad |
| 空引用 | `null` |
| 字节载荷 | 不超过 4096 字节时为 `{"$bytes": {"size": n, "sha256": "…", "base64": "…"}}`，更大时为 `{"$bytes": {"size": n, "sha256": "…", "member": "bytes/<sha256>"}}` |
| 路径非空的顶层 `m_StreamData`（`offset`、`size`、`path`）或 `m_Resource`（`m_Offset`、`m_Size`、`m_Source`） | 它在对象所属资源包的资源文件中指明的字节，写成字节载荷。贴图、网格、音频 clip 与视频 clip 通过这两个字段把数据存放在对象之外 |

## 3. 来源

**根对象：**

- 脚本为 `Assembly-CSharp/Sekai.ScenarioBackgroundBuildData` 的每个 `MonoBehaviour`（类 114），即预制体背景的构建数据；
- 容器列出的路径包含 `/scenario/effect/` 且以 `.prefab` 结尾（不区分大小写）的每个 `GameObject`（类 1）：特效资源包 `scenario/effect/<name>` 的预制体。

背景预制体经由其构建数据到达，本身不是根对象。

**遍历。** 从根对象开始，遍历按引用首次出现的顺序访问每个对象一次：按存储顺序读取一个对象的字段，每个解析到某资源包中对象的引用，若该对象尚未列出，就把它加到末尾。指向引擎自带资源中对象的引用不添加任何对象。遍历不跟随 shader 与脚本的引用，它们只列出名称。

**读取的字段：** 遍历到达的每个对象的全部存储字段；shader 与脚本只读取名称。

**文档以外的结果：**

| 情形 | 结果 |
|---|---|
| 引用指向该版本不包含的对象 | 解析它的错误 `missing_object` |
| 贴图无法解码，或 clip 格式错误 | 该错误，同 `texture` 形式与 clip 文档 |
| `m_StreamData` 或 `m_Resource` 指明的字节超出其资源文件 | 错误 `malformed` |

## 4. 容器

文档是不含无意义空白的 UTF-8 JSON，键按上文顺序排列。遍历以固定顺序访问对象，因此同一预制体总是得到相同的字节。图像的编码同 `texture` 形式。

## 5. 正确性

文档列出游戏实例化预制体时加载的对象及其存储的值，因此按引擎方式解释这些值的读取者，画出的内容与游戏相同。贴图与 clip 即 `texture` 形式与 clip 文档的结果，它们各有自己的检查。

资源包构建时为每个容器条目记录加载它会加载的对象：条目的 `preloadIndex` 与 `preloadSize` 在资源包 `m_PreloadTable` 中选出的一段。遍历若在其他资源包的对象处停下（引擎随持有它们的资源包加载这些对象），并且不计引擎对象与 shader 的引用，得到的集合与之相同。对一个版本的每个资产比较这两个集合，即用构建结果检查遍历；复现方法是带对象文档导出 `scenario/effect/` 资源包，把每个资产的 `objects` 与其根对象的 preload 段比较。

`crates/sekai-moesekai` 的测试（`tests/scenario_prefab.rs`）用内存中构造的对象检查：

- 根规则：构建数据按脚本识别，特效预制体按路径识别，背景预制体与其他路径都不是根；
- 一个包含 transform、sprite renderer、材质、贴图、animator 及其 controller 与 clip 的特效的文档；
- 背景从其构建数据开始，并经 `BackgroundPrefab` 到达其预制体；
- 遍历按首次引用顺序把每个对象列出一次，包括有环的情形；
- clip 文档不能表示的 clip 保留其字段与引用，没有成员；
- shader 与脚本只列出名称，不跟随其引用；
- 引擎引用、空引用、大小字节载荷、两种流式数据、超出 ±2^53 的整数与各种浮点情形按上文写出；
- 同一预制体得到相同的字节；
- 引用缺失对象得到 `missing_object`，超出资源文件的流式字节得到 `malformed`。

## 6. 版本

| 版本 | 变更 |
|---|---|
| 1 | 第一版 |
