# Characters

The `characters` form is the roster of the characters of MySekai, as one JSON document. Converter: `characters`.

## 1. Runtime

A runtime that reproduces MySekai shows each character under a name and a unit colour, plays its idle and walk motions, moves it at its own speed, pauses it between steps and starts its alone-action script. It reads the document once at load.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document holds `version` (1) and `characters`, an object keyed by the decimal character unit id, in ascending unit order. An entry holds:

| Key | Value |
|---|---|
| `unitId` | The character unit id, the same as the key |
| `name` | The display name: the family name followed by the given name, either of which may be absent |
| `gameCharacter` | `id`, the game character id, and `gender`, as written in the game data |
| `identity` | `gameCharacterId`, the same as `gameCharacter.id`; `unit`, the group the character belongs to; `colorCode`, the colour of the character as `#rrggbb` |
| `locomotion` | `idleMotion` and `walkMotion`, the names of the motion families in the motion library; `pauseSeconds`, the pause between steps; `walkSpeedMetersPerSecond`, the walking speed |
| `soloAction` | The name of the alone-action script of the character, or `null` when the game data binds none |

## 3. Sources

**Game data tables:**

- `mysekaiCharacterTalkMotions`: one row per character, which decides the roster. `gameCharacterUnitId`, `idleMotion`, `walkMotion`, `pauseMilliSeconds` and `walkSpeed`.
- `gameCharacterUnits`: the row whose `id` is the unit. `gameCharacterId`, `unit` and `colorCode`.
- `gameCharacters`: the row whose `id` is that game character. `firstName`, `givenName` and `gender`.
- `mysekaiCharacterTalkSoloActions`: the row whose `gameCharacterUnitId` is the unit, if any. `lua`.

`pauseSeconds` is `pauseMilliSeconds` divided by 1000. `walkSpeedMetersPerSecond` is `walkSpeed` divided by 1000: the game stores the speed a thousand times larger than the metres per second it moves by.

A unit or game character that a row refers to and that has no row of its own is the error `malformed`.

## 4. Container

UTF-8 JSON without a byte order mark, compact, with object keys in the order the tables above give and characters in ascending unit order.

## 5. Correctness

The stored and the converted values are related only by the division above. Unit tests on synthetic rows check the join, the division, the name, a character without an alone-action script and both missing references. A real version is compared key by key with a document produced independently from the same tables.

## 6. Version

1.
