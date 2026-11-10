# Scenario

[中文](scenario.zh-CN.md)

The `scenario` form is the scene data of a story — its lines, layouts, effects, sounds and the resources it names — as a JSON document. Converter: `scenario` in `sekai-moesekai` (feature `scenario`). Form version: 1.

## 1. Runtime

Story readers and players on the web, and tools that search, translate or analyse the stories. The form gives the scene data as the game stores it; it does not resolve the names it holds into files, play the story, or join it with the game data tables that list episodes.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `scenario` | A JSON document, extension `json` |

The document is one object holding every serialized field of the scene data object, in stored order, except the four fields every `MonoBehaviour` has: `m_GameObject`, `m_Enabled`, `m_Script` and `m_Name` (the name repeats `ScenarioId`). Field names are written as stored, such as `ScenarioId`, `AppearCharacters`, `Snippets`, `TalkData`, `LayoutData`, `SpecialEffectData`, `SoundData` and `NeedBundleNames`.

Values are written as follows:

| Stored value | JSON |
|---|---|
| Structure | Object with the fields in stored order |
| Array | Array |
| Boolean | `true` or `false` |
| Integer | Number, all digits |
| Floating-point number | Number: the shortest decimal that reads back to the same 32-bit float when the value is one, otherwise the shortest that reads back to the same 64-bit float |
| String | String; `"`, `\` and control characters are escaped, and every other character is written as it is in UTF-8 |

The document has no whitespace outside strings. Resources that the story uses are named in it by the strings the game stores — bundle names, voice IDs, background and music names — and are not included in it.

## 3. Sources

**Root objects:** every `MonoBehaviour` (class 114) whose script is `Assembly-CSharp/Sekai.ScenarioSceneData`. The form does not read game data tables.

**Fields read:** all serialized fields of the root object.

**Outcomes other than a document:**

| Case | Result |
|---|---|
| A field holds a reference, a map, a byte array or a string that is not UTF-8 | Error `unsupported` |
| A floating-point field is not finite | Error `malformed` |

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters.

## 5. Correctness

The document holds the stored values of the object without interpretation, so it is complete with respect to what the game reads, and the same object always gives the same bytes: the field order is the stored order and every number has one written form. It is checked on real bundles by comparing every value of every document with the fields UnityPy reads from the same object through its type tree, floating-point numbers as 32-bit floats, and by converting all documents on one thread and on several and comparing the bytes.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
