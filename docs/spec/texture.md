# Texture

[中文](texture.zh-CN.md)

The `texture` form is the image a texture object stores, as a plain picture. Converter: `texture` in `sekai-convert` (feature `image`). Form version: 2.

## 1. Runtime

Any program that shows or processes the picture a texture holds: image viewers, web pages that display card and story art, and tools that crop sprites from a texture themselves. The form keeps the stored colour values as they are; it does not apply the texture's sampling settings, colour space conversion or the shading of any material that uses it.

## 2. Contents

A `Texture2D` or a `Cubemap` gives one member:

| Role | Content |
|---|---|
| `image` | The image, rows from top to bottom: 8-bit RGBA with straight (not premultiplied) alpha or, for an HDR format, linear RGBA in binary16 |

The image is mip level 0 of the first slice. For a `Cubemap` this is the first face (+X). The width and height are `m_Width` and `m_Height`.

A `Texture2DArray` or a `Texture3D` gives one member per slice, `slice/0` to `slice/<m_Depth - 1>`, each mip level 0 of that slice in the same forms as `image`. For a `Texture3D` the slices are the layers along the third axis, from the first.

Each texel format expands to RGBA as follows:

| Format (`m_TextureFormat`) | Expansion |
|---|---|
| 1 Alpha8 | `(0, 0, 0, a)` |
| 3 RGB24 | `(r, g, b, 255)` |
| 4 RGBA32 | `(r, g, b, a)` |
| 7 RGB565 | Each channel widened by repeating its high bits: `r << 3 \| r >> 2`, `g << 2 \| g >> 4`, `b << 3 \| b >> 2`; alpha 255 |
| 34 ETC_RGB4 | ETC1, alpha 255 |
| 29 DXT5Crunched | Crunch-compressed BC3. Colours follow the S3TC formulas: endpoints are `c / 31` and `c / 63`, interpolation is exact, and the result is rounded to the nearest 8-bit value |
| 41 EAC_R | EAC R11, the 11-bit value narrowed to 8 bits by dropping its low bits: `(r, 0, 0, 255)` |
| 45 ETC2_RGB | ETC2 colour, alpha 255 |
| 47 ETC2_RGBA8 | ETC2 colour with EAC alpha |
| 63 R8 | `(r, 0, 0, 255)` |
| 66 to 71 ASTC_HDR | ASTC in the HDR profile, footprints 4x4, 5x5, 6x6, 8x8, 10x10 and 12x12, as binary16 values |
| 48–53 ASTC 4×4 to 12×12 | ASTC LDR decoded to 8 bits per channel, as ARM's reference decoder does in its LDR profile |

Texture arrays and 3D textures name their format with `m_Format`, a graphics format:

| Graphics format (`m_Format`) | Expansion |
|---|---|
| 48 R16G16B16A16_SFloat | Linear RGBA in binary16, the stored values unchanged |
| 134 RGBA_ASTC6X6_UNorm | As texture format 50, ASTC 6×6 |

## 3. Sources

**Root objects:** every `Texture2D` (class 28), `Cubemap` (class 89), `Texture3D` (class 117) and `Texture2DArray` (class 187). The form does not read game data tables.

**Fields read:** `m_Width`, `m_Height`, `m_TextureFormat`, `image data` and `m_StreamData` (`offset`, `size`, `path`); for texture arrays and 3D textures `m_Format`, `m_Depth` and `m_DataSize` in place of `m_TextureFormat`.

**Image data.** When `m_StreamData.path` is not empty, the image data is `size` bytes at `offset` in the resource file the path names; the file is an entry of the texture's own bundle, and the path's last component is the entry name. Otherwise the image data is the `image data` field. The data starts with mip level 0 of the first slice; the bytes after that image are not used. The data of a texture array or a 3D texture is `m_DataSize` bytes, split into `m_Depth` slices of equal size, each starting with its mip level 0.

**Outcomes other than an image:**

| Case | Result |
|---|---|
| Width or height is 0, and there is no image data and no stream path | No asset. This is the texture of a dynamic font, which the engine fills at run time |
| Width or height is 0, but image data is present | Error `malformed` |
| The format is not in the table in section 2 | Error `unsupported` |
| The image data is shorter than mip level 0 | Error `truncated` |
| An ASTC block is illegal | Error `malformed` |
| A block of an LDR ASTC format uses HDR endpoints | Error `unsupported` |
| A crunch file is damaged, or has other dimensions than the texture | Error `malformed` or `truncated` |
| The stream path names no entry of the bundle | Error `missing_resource` |
| A texture array or 3D texture uses a graphics format not in the table in section 2 | Error `unsupported` |
| The image data of a texture array or 3D texture is not `m_DataSize` bytes, or does not split into `m_Depth` equal slices | Error `malformed` |

## 4. Container

The member is decoded content, which the evaluator encodes. The canonical file is lossless WebP, written by the `webp-lossless` encoder (version 1) with the fixed parameters of that encoder. The canonical file of an HDR image is OpenEXR with half-float RGBA channels and ZIP compression, written by the `exr` encoder (version 1), which stores every decoded value exactly; browsers cannot show it, so a view that needs an 8-bit image clamps each value to 0..=1, multiplies it by 255 and rounds it. Views that need another container, such as PNG, produce it from the same decoded image with their own encoder.

## 5. Correctness

Unity stores texture rows from bottom to top; the form turns them to top to bottom, the order of PNG and WebP. The stored colour values are kept unchanged, so the result is the same picture Unity uploads to the GPU, before any sampling or colour space conversion.

The decoders are checked byte for byte against independent reference decoders: ASTC against ARM's `astcenc` in its 8-bit LDR mode and, as binary16 values, in its HDR profile, crunch against the blocks unpacked by Unity's crunch library and exact arithmetic from the S3TC formulas, ETC1, ETC2 and EAC against a reference ETC decoder, and the uncompressed formats against a direct expansion of the bytes. The checks run on synthetic textures built with the Unity editor (`crates/sekai-codec/tests`) and, by anyone who has the game data, on real bundles with the scripts in `tools/compare-texture`. `compare_converter.py` checks the whole converter, including the fields and the resource files, texture by texture. The slices of texture arrays are checked against the same reference decoders, and binary16 slices against the stored values.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
| 2 | Texture arrays and 3D textures, one member per slice |
