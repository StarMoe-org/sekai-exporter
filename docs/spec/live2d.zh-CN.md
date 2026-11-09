# Live2D

[English](live2d.md)

游戏的 Live2D 内容由两种形式承载：

- **`live2d-model`**：游戏所加载的一个模型，包括 `.moc3` 文件、贴图、物理设置，以及属于模型本身的动作。转换器：`sekai-convert` 中的 `live2d-model`（feature `live2d`）。形式版本：2。
- **`live2d-motions`**：游戏所加载的一组动作，即一个角色在其任一模型上播放的身体动作与表情动作。转换器：`sekai-convert` 中的 `live2d-motions`（feature `live2d`）。形式版本：2。

## 1. 运行时

Moesekai 的 Live2D 查看器中的 Live2D 运行时，它按游戏的方式显示模型：用 Cubism Core 载入 `.moc3` 文件，用模型的贴图绘制，运行其物理，并在游戏的两个动作层上播放动作。它以 Unity 绑定所用的哈希把动作的曲线绑定到模型参数，按 Unity 的方式求值，从游戏的元数据读取每个动作的淡入时间，并调用名为 `OnLive2DInvokeUserData` 的事件。动作为何保留为游戏的 clip、而不是 Cubism 的 `motion3.json` 文件，参见[Live2D 动作](../live2d-motion.zh-CN.md)。

两种形式只保留游戏所加载的内容。每个模型包中还有一个 `.model3.json` 文件，游戏并不读取它，而是通过模型的构建数据找到模型的各个文件；该文件可作为 [`text`](text.md) 资产取得。

## 2. 内容

### `live2d-model`

| 角色 | 内容 |
|---|---|
| `moc` | `.moc3` 文件，扩展名 `moc3` |
| `physics` | `.physics3.json` 文件，扩展名 `json`。模型没有物理时不存在 |
| `texture/0`、`texture/1`…… | 各贴图，顺序即模型的可绘制对象引用它们的序号，每张与 [`texture`](texture.md) 形式相同 |
| `motion/<名称>` | 模型本身的每个动作，为动作文档（见下），扩展名 `json` |

成员按上表顺序排列。`moc` 与 `physics` 是原样保存的字节。

### `live2d-motions`

| 角色 | 内容 |
|---|---|
| `motion/<名称>` | 每个身体动作，为动作文档，扩展名 `json` |
| `facial/<名称>` | 每个表情动作，为动作文档，扩展名 `json` |

先是身体动作，然后是表情动作，各按游戏列出的顺序。`<名称>` 是游戏查找该动作所用的名称，在同一类中唯一。

### 动作文档

```json
{ "format": "live2d-motion", "version": 1, "fade_in": 0.5, "fade_out": 0.5, "clip": { "format": "clip", … } }
```

| 键 | 值 |
|---|---|
| `format`、`version` | `"live2d-motion"` 与本文档格式的版本 1 |
| `fade_in`、`fade_out` | 动作的淡入与淡出时间，单位为秒，取自其元数据对象。游戏列出该动作时未带元数据则不存在 |
| `clip` | 动作的 clip，为 [clip 文档](clip-documents.zh-CN.md) |

`path` 为 `crc32("Parameters/<id>")`、`attribute` 为 `crc32("Value")` 的曲线驱动模型的参数 `<id>`。`path` 为 `crc32("Parts/<id>")`、`attribute` 为 `crc32("Opacity")` 的曲线驱动部件 `<id>` 的不透明度。在所播放的模型中匹配不到任何对象的曲线不驱动任何东西。

### 在发布中

每种形式在每次发布中都有一份列表（参见[发布文档](publication.zh-CN.md)）。`live2d-model` 列表列出每个模型，`live2d-motions` 列表列出每个动作组，并带有指纹与大小。读取方从资产文档 `sources` 的第一项取得模型或动作组所在的资源包，从其中列出的成员得知要取哪些文件。成员的大小即打开时的下载量，`texture/` 成员的个数即贴图数。

剧情把哪个动作组与模型一起播放，由剧情决定，而不由任一资源包决定：剧情为每个角色指定一个模型包，游戏取该角色动作组中的动作。这种对应关系属于[话](../features.zh-CN.md)资产，它写明所显示的每个角色的模型与动作组。

## 3. 来源

### `live2d-model`

**根对象：** 脚本为 `Assembly-CSharp/Sekai.Live2D.Live2DBuildModelData` 的每个 `MonoBehaviour`（类 114）。一个资源包可能带有多个模型的文件，构建数据指明游戏加载的是哪一个。

**读取的字段：** 根对象的 `Moc3FileName`、`TextureNames`、`PhysicsFileName`、`UserDataFileName`、`AdditionalMotionData` 与 `CategoryRules`。

各文件按名称查找：每个名称拼接到根对象容器路径所在的目录之后，在资源包的容器路径中不区分大小写地查找：

- `moc` 是 `Moc3FileName` 所指的 `TextAsset`，其字节为 `m_Script`；
- `texture/<i>` 是 `TextureNames` 第 *i* 项所指的 `Texture2D`；
- `physics` 是 `PhysicsFileName` 所指的 `TextAsset`，该字段不为空时存在。

`AdditionalMotionData` 的每一项给出一个 `motion/<名称>` 成员，规则与下文的动作组相同。

### `live2d-motions`

**根对象：** 脚本为 `Assembly-CSharp/Sekai.Live2D.Live2DBuildMotionData` 的每个 `MonoBehaviour`（类 114）。

**读取的字段：** 根对象的 `Motions`、`Facials`、`Arms` 与 `Poses`；每个元数据对象的 `FadeInTime` 与 `FadeOutTime`；每个 clip 中其 clip 文档所读的字段。

`Motions` 的每一项给出一个 `motion/<名称>` 成员，`Facials` 的每一项给出一个 `facial/<名称>` 成员。每一项有三个字段：`ClipAssetName` 即 `<名称>`；`Clip` 引用 `AnimationClip`；`Meta` 引用脚本为 `Assembly-CSharp/Sekai.Live2D.Live2DBuildMotionMetaData`、给出淡入淡出时间的 `MonoBehaviour`。

两种形式都不读取游戏数据表。

**资产以外的结果：**

| 情况 | 结果 |
|---|---|
| 所指名的文件，或某一项引用的对象，不在资源包中 | 错误 `missing_object` |
| 所指名的文件不是上述的类、`.moc3` 文件不以 `MOC3` 开头，或同一类中有两项同名 | 错误 `malformed` |
| `UserDataFileName`、`CategoryRules`、`Arms` 或 `Poses` 不为空 | 错误 `unsupported` |
| 某个 clip 无法给出 clip 文档 | [clip 文档](clip-documents.zh-CN.md#3-来源)的相应错误 |

## 4. 容器

`moc` 与 `physics` 是成品字节，原样写出。贴图按 `texture` 形式编码。动作文档是 UTF-8 JSON，无多余空白，键的顺序与上文相同；数的写法与 clip 文档相同。

## 5. 正确性

每个成员或是原样保存的文件，或是 `texture` 形式产出的贴图，或是 clip 文档。因此运行时构建出的模型，就是游戏由同样的文件构建出的模型；动作给出的值就是所存 clip 的值（与 Cubism 自身求值的比较参见[Live2D 动作](../live2d-motion.zh-CN.md)）。

持续集成用合成对象构造模型包与动作包，检查各成员、其顺序、淡入淡出时间以及上述每种情况。在真实资源包上，`.moc3` 与物理成员逐字节、贴图逐像素地与对同一批资源包的独立解包结果比较。随后查看器的运行时把一个动作组的每个动作绑定到同一角色的每个模型，在查看器的帧时钟上播放到结束，一次取自动作文档，一次取自对同一批 clip 的独立解码；每一帧的参数值逐位相等。

## 6. 版本

| 形式 | 版本 | 改动 |
|---|---|---|
| `live2d-model` | 1 | 首个版本 |
| `live2d-motions` | 1 | 首个版本 |
| `live2d-model` | 2 | 动作为版本 2 的 [clip 文档](clip-documents.zh-CN.md) |
| `live2d-motions` | 2 | 动作为版本 2 的 [clip 文档](clip-documents.zh-CN.md) |
