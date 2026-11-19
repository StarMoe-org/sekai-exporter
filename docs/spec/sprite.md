# Sprite

[中文](sprite.zh-CN.md)

The `sprite` form is the region of a texture a `Sprite` shows, with the layout that places it. Converter: `sprite` in `sekai-convert` (feature `image`). Form version: 2.

## 1. Runtime

Any program that shows or lays out the pictures of a user interface: image viewers, web pages that show icons and backgrounds, and tools that rebuild nine-slice frames or position sprites by their pivot. The form keeps the stored colour values as they are; it does not draw the sprite's mesh, apply the material or scale the picture by pixels per unit.

## 2. Contents

Two members:

| Role | Content |
|---|---|
| `image` | The pixels of the sprite's rectangle `m_Rect`, rows from top to bottom, in the texture's own form: 8-bit RGBA with straight alpha or, for an HDR texture, linear RGBA in binary16 |
| `sprite` | A JSON document with the layout of the sprite |

The `sprite` document is one object with these keys, in this order, with every number written as the shortest decimal that reads back to the stored 32-bit float:

| Key | Value |
|---|---|
| `rect` | `{"x", "y", "width", "height"}`: `m_Rect`, the sprite's rectangle in the texture, in pixels from the bottom left; the image is cut from it |
| `texture_rect` | `{"x", "y", "width", "height"}`: `m_RD.textureRect`, the part of the texture that holds the sprite's mesh |
| `texture_rect_offset` | `{"x", "y"}`: `m_RD.textureRectOffset`, where the texture rectangle starts within `rect` |
| `pivot` | `{"x", "y"}`: `m_Pivot`, as a fraction of `rect` |
| `pixels_per_unit` | `m_PixelsToUnits` |
| `border` | `{"left", "bottom", "right", "top"}`: `m_Border` (`x`, `y`, `z`, `w`), the nine-slice borders in pixels |

The document has no whitespace.

## 3. Sources

**Root objects:** every `Sprite` (class 213). The form does not read game data tables.

**Fields read:** `m_Rect`, `m_Pivot`, `m_PixelsToUnits`, `m_Border`, `m_SpriteAtlas`, `m_RenderDataKey`, and from the render data (`m_RD`, or the entry of the atlas under `m_RenderDataKey` when the atlas lists one): `texture`, `alphaTexture`, `textureRect`, `textureRectOffset`, `settingsRaw`; for a sprite of an atlas that packs tight outlines, also `m_VertexData` and `m_IndexBuffer` of `m_RD`. From the sprite atlas: `m_RenderDataMap`. From the texture: the fields the `texture` form reads.

**Image.** The `texture` of the render data is decoded as the `texture` form decodes a `Texture2D`. Each edge of `m_Rect`, or of `textureRect` when the render data comes from the atlas, is rounded to the nearest whole pixel, ties to even: the left edge is `x`, the right edge `x + width`, the bottom edge `y` and the top edge `y + height`, the sums taken in double precision. The image is the pixels between those edges, with `y` counted from the bottom row of the texture.

**Trimmed sprites.** An atlas that packs rectangles (bit 1 of `settingsRaw` set) may trim the transparent margins of a sprite: its `textureRect` is then smaller than `m_Rect`, or its `textureRectOffset` is not zero. The image of such a sprite has the size of `m_Rect`, each side rounded to the nearest whole pixel, ties to even. It is transparent black except for the pixels cut from `textureRect`, whose bottom left corner lies at `textureRectOffset`, rounded the same way, from the bottom left of the image.

**Sprites of tight atlases.** An atlas that packs tight outlines (bit 1 of `settingsRaw` clear) places the box around the mesh of a sprite, `textureRect`, where it fits beside the outlines of other sprites, so the box may also hold their pixels. When such a sprite is trimmed, or mirrored (packing rotation 1, 2 or 3 in bits 2 to 5 of `settingsRaw`: horizontally, vertically, or both), its image has the size of `m_Rect`, each side rounded as above. It is transparent black except for each pixel whose centre lies in or on a triangle of the sprite's own mesh (`m_RD`: the positions of channel 0 of `m_VertexData`, three 16-bit indices of `m_IndexBuffer` per triangle). A position `p` of the mesh lies at `p * m_PixelsToUnits + m_Pivot * (m_Rect.width, m_Rect.height)` pixels from the bottom left of the sprite. Such a pixel is the texture pixel under its centre `c`: along each axis, `textureRect.x + (c.x - textureRectOffset.x)`, or `textureRect.x + textureRect.width - (c.x - textureRectOffset.x)` when the atlas mirrors that axis, rounded down, and likewise for `y`. A triangle without area covers no pixel.

**Outcomes other than an image:**

| Case | Result |
|---|---|
| The render data comes from an atlas that packs rectangles and has a packing rotation | Error `unsupported` |
| The render data comes from an atlas that packs tight outlines and turns the sprite a quarter turn (packing rotation 4 or more) | Error `unsupported` |
| The region of a trimmed sprite reaches outside its image | Error `malformed` |
| The mesh of a sprite of a tight atlas has no positions, is not a list of triangles, names a vertex it does not have, or covers a pixel whose texture pixel lies outside the texture | Error `malformed` |
| The render data is `m_RD` and `settingsRaw` has the packed bit (bit 0) set | Error `unsupported` |
| The render data has an `alphaTexture` that is not null | Error `unsupported` |
| The rounded rectangle is empty or reaches outside the texture | Error `malformed` |
| The texture holds no image | Error `malformed` |
| A float of the layout is not finite | Error `malformed` |
| The texture cannot be decoded | The error of the `texture` form |

Except for a trimmed or mirrored sprite of a tight atlas, the pixels of the rectangle that the sprite's mesh does not cover are kept as stored: the form cuts the rectangle and does not mask it by the mesh.

## 4. Container

The `image` member is decoded content, which the evaluator encodes as the `texture` form encodes its images: PNG for 8-bit images and OpenEXR for HDR images. The `sprite` member is finished bytes, a JSON file.

## 5. Correctness

The image is the rectangle the sprite was defined with, so it keeps the size of the picture and the place of its content, including transparent margins. A rectangle-packing atlas trims only margins whose pixels are fully transparent, so putting the region back at its offset in a transparent image gives the picture of the sprite; colour values the margins held under zero alpha are not kept. In a tight atlas the box around a mesh is not the sprite's alone, and the pixels that belong to the sprite are those its mesh covers, which are the pixels a renderer draws when it draws the mesh: a rasterizer shades a pixel when its centre lies in a triangle. A pixel whose centre lies exactly on an edge is kept, so the triangles of a mesh leave no gap between them and the result does not depend on their order or winding. `m_RD.textureRect` is the smaller box around the mesh, in fractional pixels when the mesh is tight; the layout records it for programs that draw the mesh. The rectangle is rounded to whole pixels the same way an image library crops a box given in fractional pixels.

The crop and the layout are checked on real bundles against UnityPy: the image against the same rectangle cut from the texture decoded by the reference decoders of `tools/compare-texture`, and every value of the layout against the fields UnityPy reads. The image of a sprite of a tight atlas, mirrored or not, is checked against the image UnityPy draws by the mesh, which matches it pixel for pixel where it lies in the image.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
| 2 | The image is the rectangle `m_Rect`. Revision 1 converts sprites that an atlas trimmed or mirrored, which version 2 refused |
