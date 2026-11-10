# 贴图

[English](texture.md)

`texture` 形式是贴图对象所存储的图像，写成普通图片。转换器：`sekai-convert` 中的 `texture`（feature `image`）。形式版本：2。

## 1. 运行时

任何显示或处理贴图所含图片的程序：图像查看器、显示卡牌与剧情插画的网页，以及自行从贴图裁切精灵的工具。形式按所存的原样保留颜色值；它不施加贴图的采样设置、色彩空间转换，也不施加使用它的任何材质的着色。

## 2. 内容

`Texture2D` 或 `Cubemap` 给出一个成员：

| 角色 | 内容 |
|---|---|
| `image` | 图像，行从上到下：直通（非预乘）alpha 的 8 位 RGBA，或 HDR 格式的 binary16 线性 RGBA |

图像是第一个切片（slice）的 mip 级别 0。对于 `Cubemap`，这是第一个面（+X）。宽度与高度为 `m_Width` 与 `m_Height`。

`Texture2DArray` 或 `Texture3D` 为每个切片给出一个成员，`slice/0` 到 `slice/<m_Depth - 1>`，各为该切片 mip 级别 0，形式与 `image` 相同。对于 `Texture3D`，切片是沿第三轴的各层，从第一层开始。

每种纹素格式展开为 RGBA 的方式如下：

| 格式（`m_TextureFormat`） | 展开 |
|---|---|
| 1 Alpha8 | `(0, 0, 0, a)` |
| 3 RGB24 | `(r, g, b, 255)` |
| 4 RGBA32 | `(r, g, b, a)` |
| 7 RGB565 | 每个通道重复其高位来加宽：`r << 3 \| r >> 2`、`g << 2 \| g >> 4`、`b << 3 \| b >> 2`；alpha 为 255 |
| 34 ETC_RGB4 | ETC1，alpha 为 255 |
| 29 DXT5Crunched | Crunch 压缩的 BC3。颜色遵循 S3TC 公式：端点为 `c / 31` 与 `c / 63`，插值精确，结果舍入到最近的 8 位值 |
| 41 EAC_R | EAC R11，11 位的值去掉低位收窄为 8 位：`(r, 0, 0, 255)` |
| 45 ETC2_RGB | ETC2 颜色，alpha 为 255 |
| 47 ETC2_RGBA8 | ETC2 颜色加 EAC alpha |
| 63 R8 | `(r, 0, 0, 255)` |
| 66 至 71 ASTC_HDR | HDR 配置下的 ASTC，足迹为 4x4、5x5、6x6、8x8、10x10 与 12x12，值为 binary16 |
| 48–53 ASTC 4×4 至 12×12 | 解码为每通道 8 位的 ASTC LDR，与 ARM 的参考解码器在其 LDR 配置下所做相同 |

贴图数组与 3D 贴图以 `m_Format`（图形格式）指明格式：

| 图形格式（`m_Format`） | 展开 |
|---|---|
| 48 R16G16B16A16_SFloat | binary16 线性 RGBA，所存的值不变 |
| 134 RGBA_ASTC6X6_UNorm | 同贴图格式 50，ASTC 6×6 |

## 3. 来源

**根对象：** 每个 `Texture2D`（类 28）、`Cubemap`（类 89）、`Texture3D`（类 117）与 `Texture2DArray`（类 187）。形式不读取游戏数据表。

**读取的字段：** `m_Width`、`m_Height`、`m_TextureFormat`、`image data` 与 `m_StreamData`（`offset`、`size`、`path`）；贴图数组与 3D 贴图以 `m_Format`、`m_Depth` 与 `m_DataSize` 代替 `m_TextureFormat`。

**图像数据。** `m_StreamData.path` 不为空时，图像数据是路径所指资源文件中的 `size` 字节，起始于 `offset`；该文件是贴图自身资源包中的一个条目，路径的最后一段即条目名。否则图像数据为 `image data` 字段。数据从第一个切片的 mip 级别 0 开始；该图像之后的字节不使用。贴图数组或 3D 贴图的数据为 `m_DataSize` 字节，分成 `m_Depth` 个等大的切片，每个切片从其 mip 级别 0 开始。

**图像以外的结果：**

| 情况 | 结果 |
|---|---|
| 宽度或高度为 0，且没有图像数据也没有流路径 | 无资产。这是动态字体的贴图，由引擎在运行时填充 |
| 宽度或高度为 0，但有图像数据 | 错误 `malformed` |
| 格式不在第 2 节的表中 | 错误 `unsupported` |
| 图像数据短于 mip 级别 0 | 错误 `truncated` |
| 某个 ASTC 块不合法 | 错误 `malformed` |
| LDR ASTC 格式的块使用了 HDR 端点 | 错误 `unsupported` |
| crunch 文件损坏，或尺寸与该贴图不同 | 错误 `malformed` 或 `truncated` |
| 流路径未指明资源包中的任何条目 | 错误 `missing_resource` |
| 贴图数组或 3D 贴图使用的图形格式不在第 2 节的表中 | 错误 `unsupported` |
| 贴图数组或 3D 贴图的图像数据不是 `m_DataSize` 字节，或不能分成 `m_Depth` 个等大的切片 | 错误 `malformed` |

## 4. 容器

成员是已解码内容，由求值器编码。规范文件是无损 WebP，由 `webp-lossless` 编码器（版本 1）以该编码器的固定参数写出。HDR 图像的规范文件是 OpenEXR，使用半精度浮点 RGBA 通道与 ZIP 压缩，由 `exr` 编码器（版本 1）写出，它精确存储每个解码出的值；浏览器无法显示它，因此需要 8 位图像的视图把每个值钳制到 0..=1，乘以 255 并舍入。需要其他容器的视图，例如 PNG，由同一张解码后的图像经其自身的编码器产生。

## 5. 正确性

Unity 自下而上存储贴图的行；形式把它们变为自上而下，即 PNG 与 WebP 的顺序。所存的颜色值保持不变，因此结果是 Unity 上传给 GPU 的同一张图片，先于任何采样或色彩空间转换。

解码器与独立的参考解码器逐字节比对：ASTC 对照 ARM 的 `astcenc` 在其 8 位 LDR 模式下的输出，并在其 HDR 配置下按 binary16 值比对；crunch 对照 Unity 的 crunch 库解包的块以及按 S3TC 公式的精确运算；ETC1、ETC2 与 EAC 对照一个参考 ETC 解码器；未压缩格式对照字节的直接展开。这些检查运行在用 Unity 编辑器构建的合成贴图上（`crates/sekai-codec/tests`），并在真实资源包上由任何持有游戏数据的人用 `tools/compare-texture` 中的脚本运行。`compare_converter.py` 逐张贴图检查整个转换器，包括字段与资源文件。贴图数组的各切片对照同一批参考解码器检查，binary16 切片对照所存的值检查。

## 6. 版本

| 版本 | 变更 |
|---|---|
| 1 | 第一版 |
| 2 | 贴图数组与 3D 贴图，每个切片一个成员 |
