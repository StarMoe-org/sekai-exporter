# Facial tables

[中文](facial-tables.zh-CN.md)

The facial tables say which cells of a character's eye and mouth textures show each expression. The `facial-table` form is one such table as the game stores it. The `facial-tables` form joins the three tables of a version into one document. Converters: `facial-table` and `facial-tables`.

## 1. Runtime

A runtime that reproduces MySekai sets a character's face by name: an eye pattern and a mouth pattern. It looks each name up to find the texture cells to show, blinks only the patterns that allow it, and starts each character on its default face. It reads the joined document once at load.

## 2. Contents

Both forms have one member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document of `facial-table` holds the serialized fields of the table, without the fields every `MonoBehaviour` has.

The document of `facial-tables` holds three arrays, each in the order of its table:

`NPCAvatarEyeData`, the eye patterns:

| Key | Value |
|---|---|
| `PatternName` | The name of the pattern |
| `OpenEyeIndex` | The texture cell of the open eye |
| `CloseEyeIndex` | The texture cell of the closed eye |
| `BlinkEnabled` | 1 if the pattern blinks, otherwise 0 |
| `TextureIndex` | The index of the eye texture the cells are on. 0 in a version whose table has no such column, because such a version has one eye texture |

`NPCAvatarLipSyncData`, the mouth patterns:

| Key | Value |
|---|---|
| `Name` | The name of the pattern |
| `OpenLipSyncIndex` | The texture cell of the open mouth |
| `MiddleLipSyncIndex` | The texture cell of the half open mouth, or -1 if the pattern has none |
| `CloseLipSyncIndex` | The texture cell of the closed mouth |

`NPCAvatarDefaultFacialData`, the default faces:

| Key | Value |
|---|---|
| `CharacterUnitId` | The character unit id |
| `EyePatternName` | The name of its eye pattern |
| `MouthPatternName` | The name of its mouth pattern |

## 3. Sources

**Objects of `facial-table`:** every `MonoBehaviour` of the scripts `Sekai.Mysekai.NPCAvatarEyeDataList`, `Sekai.Mysekai.NPCAvatarLipSyncDataList` and `Sekai.Mysekai.NPCAvatarDefaultFacialDataList` of the assembly `Assembly-CSharp`.

**Assets of `facial-tables`:** the `facial-table` assets `npcavatareyedata`, `npcavatarlipsyncdata` and `npcavatardefaultfacialdata` of the package `mysekai/character_settings`. Their lists are in the fields `EyeDataList`, `LipSyncDataList` and `NPCAvatarDefaultFacialData`. A version whose package holds no `facial-table` asset has no `facial-tables` asset. One table missing while another is present, or a column other than `TextureIndex` missing from a row, is an error.

## 4. Container

UTF-8 JSON without a byte order mark, compact. Values are as stored.

## 5. Correctness

Every value is copied from the table it names, so the document is correct when the object was read exactly and the columns are the ones above. Reading a row by its typed columns makes a renamed or missing column an error instead of an absent value. Unit tests on synthetic documents check the join and each error. A real version is compared row by row with an independent read of the same objects.

## 6. Version

`facial-table` 1, `facial-tables` 1.
