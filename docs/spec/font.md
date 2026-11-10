# Font

[中文](font.zh-CN.md)

The `font` form is the font file a `Font` object stores. Converter: `font` in `sekai-convert` (feature `text`). Form version: 1.

## 1. Runtime

Any program that renders text with a font file: operating systems, browsers through `@font-face`, and font tools.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `font` | The bytes of `m_FontData`, unchanged |

The extension follows the first four bytes of the data:

| Signature | Extension |
|---|---|
| `00 01 00 00` or `true` (TrueType outlines) | `ttf` |
| `OTTO` (CFF outlines) | `otf` |

## 3. Sources

**Root objects:** every `Font` (class 128). The form does not read game data tables.

**Fields read:** `m_FontData`.

**Outcomes other than a file:**

| Case | Result |
|---|---|
| `m_FontData` is empty | No asset. The font names fonts installed on the device and stores no file |
| The signature is not in the table in section 2 | Error `unsupported` |
| `m_FontData` is missing or is not a byte array | Error `malformed` |

## 4. Container

The member is finished bytes, written as they are: an OpenType file with TrueType or CFF outlines. There are no encoding parameters.

## 5. Correctness

The member is `m_FontData` as stored, so it is the font file Unity imported. It is checked by comparing the SHA-256 of every member, and its extension, with the font data that UnityPy reads for the same object, on real bundles.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
