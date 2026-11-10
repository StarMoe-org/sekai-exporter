# 精灵

[English](sprite.md)

`sprite` 形式是 `Sprite` 所显示的贴图区域，以及放置它的布局。转换器：`sekai-convert` 中的 `sprite`（feature `image`）。形式版本：2。

## 1. 运行时

任何显示或布局用户界面图片的程序：图像查看器、显示图标与背景的网页，以及重建九宫格边框或按轴心定位精灵的工具。形式按所存的原样保留颜色值；它不绘制精灵的网格，不施加材质，也不按每单位像素缩放图片。

## 2. 内容

两个成员：

| 角色 | 内容 |
|---|---|
| `image` | 精灵矩形 `m_Rect` 的像素，行从上到下，采用贴图自身的形式：直通 alpha 的 8 位 RGBA，或 HDR 贴图的 binary16 线性 RGBA |
| `sprite` | 带有精灵布局的 JSON 文档 |

`sprite` 文档是一个对象，带有下列键，按此顺序排列，每个数都写为能读回所存 32 位浮点数的最短十进制形式：

| 键 | 值 |
|---|---|
| `rect` | `{"x", "y", "width", "height"}`：`m_Rect`，精灵在贴图中的矩形，以左下角为原点按像素计；图像从其中裁出 |
| `texture_rect` | `{"x", "y", "width", "height"}`：`m_RD.textureRect`，贴图中含有精灵网格的部分 |
| `texture_rect_offset` | `{"x", "y"}`：`m_RD.textureRectOffset`，贴图矩形在 `rect` 之内开始的位置 |
| `pivot` | `{"x", "y"}`：`m_Pivot`，以 `rect` 的分数表示 |
| `pixels_per_unit` | `m_PixelsToUnits` |
| `border` | `{"left", "bottom", "right", "top"}`：`m_Border`（`x`、`y`、`z`、`w`），九宫格边框，以像素计 |

文档没有空白。

## 3. 来源

**根对象：** 每个 `Sprite`（类 213）。形式不读取游戏数据表。

**读取的字段：** `m_Rect`、`m_Pivot`、`m_PixelsToUnits`、`m_Border`、`m_SpriteAtlas`、`m_RenderDataKey`，以及渲染数据（`m_RD`，或图集按 `m_RenderDataKey` 列出的那一项，前提是图集列出了它）中的：`texture`、`alphaTexture`、`textureRect`、`textureRectOffset`、`settingsRaw`。从精灵图集取 `m_RenderDataMap`。从贴图取 `texture` 形式所读取的字段。

**图像。** 渲染数据的 `texture` 按 `texture` 形式解码 `Texture2D` 的方式解码。`m_Rect` 的每条边（渲染数据来自图集时为 `textureRect` 的边）舍入到最近的整像素，相同时取偶数：左边缘为 `x`，右边缘为 `x + width`，下边缘为 `y`，上边缘为 `y + height`，求和以双精度进行。图像为这些边缘之间的像素，`y` 从贴图的最下一行计数。

**图像以外的结果：**

| 情况 | 结果 |
|---|---|
| 渲染数据来自图集，且被旋转（`settingsRaw` 的第 2 至 5 位）、`textureRectOffset` 不为零，或 `textureRect` 的大小与 `m_Rect` 不同 | 错误 `unsupported` |
| 渲染数据为 `m_RD`，且 `settingsRaw` 设置了打包位（第 0 位） | 错误 `unsupported` |
| 渲染数据的 `alphaTexture` 不为 null | 错误 `unsupported` |
| 舍入后的矩形为空或越出贴图 | 错误 `malformed` |
| 贴图不含图像 | 错误 `malformed` |
| 布局的某个浮点数不是有限数 | 错误 `malformed` |
| 贴图无法解码 | `texture` 形式的错误 |

精灵的网格未覆盖的矩形像素按存储保留：形式裁出矩形，而不按网格做遮罩。

## 4. 容器

`image` 成员是已解码内容，由求值器按 `texture` 形式编码其图像的方式编码：8 位图像用无损 WebP，HDR 图像用 OpenEXR。`sprite` 成员是成品字节，即 JSON 文件。

## 5. 正确性

图像即定义精灵时所用的矩形，因此它保持图片的大小及其中内容的位置，包括透明边距。`m_RD.textureRect` 是网格周围更小的框，网格紧凑时以小数像素计；布局记录它，供绘制网格的程序使用。矩形舍入到整像素的方式，与图像库裁切以小数像素给出的框相同。

在真实资源包上，裁切与布局对照 UnityPy 检查：图像对照从 `tools/compare-texture` 的参考解码器解码的贴图中裁出的同一矩形，布局的每个值对照 UnityPy 读取的字段。

## 6. 版本

| 版本 | 变更 |
|---|---|
| 1 | 第一版 |
| 2 | 图像为矩形 `m_Rect` |
