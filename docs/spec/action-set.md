# Action set

The `action-set` form is the area actors of an action set and the group of 2D costumes each of them wears, as a JSON document. Converter: `action-set` in `sekai-moesekai` (feature `episode`). Form version: 1.

## 1. Runtime

The table stage that builds the `episode` assets, when it builds an episode of an area conversation: it reads the action set of the conversation's area to choose the costume group of every area character the conversation shows. The form gives the stored lists and nothing that the game derives from them; it does not resolve a costume group into costumes, models or motions.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `action-set` | A JSON document, extension `json` |

```json
{ "format": "action-set", "version": 1, "action_set_id": 7001, "area_id": 12,
  "details": [
    { "area_character_id": 3, "costume2d_group_id": 31 },
    { "area_character_id": 1, "costume2d_group_id": -2 }
  ] }
```

| Key | Value |
|---|---|
| `format`, `version` | `"action-set"` and the version of this document format, 1 |
| `action_set_id` | The action set ID of the object |
| `area_id` | The area ID of the object |
| `details` | The area actors and their costume groups, in the order the object stores them |
| `area_character_id` | The area actor |
| `costume2d_group_id` | The group of 2D costumes the area actor wears |

Numbers are written as signed integers in decimal.

## 3. Sources

**Root objects:** every `MonoBehaviour` (class 114) whose script is `Assembly-CSharp/Sekai.ActionSetData`. A bundle can hold more than one action set; each is a root of its own.

**Fields read:** of the root, `ActionSetId`, `AreaId` and `Details`; of each entry of `Details`, `AreaCharacterId` and `Costume2dGroupId`. Every other field of the object is not read.

The form does not read game data tables and does not refer to other objects.

**Outcomes other than a document:**

| Case | Result |
|---|---|
| `ActionSetId`, `AreaId`, `AreaCharacterId` or `Costume2dGroupId` is absent or is not an integer of the number type | Error `malformed` |
| `Details` or an entry of it is absent, `Details` is not an array, or an entry is not a structure | Error `malformed` |

The message of an error names the field, such as `Details[1].Costume2dGroupId`.

## 4. Container

The member is finished bytes: UTF-8 JSON without insignificant whitespace, with the keys in the order above and the entries of `details` in stored order.

## 5. Correctness

The document holds the stored values without interpretation, so an episode that reads a costume group from it reads the group the game reads for the same object, and the same object always gives the same bytes.

In the tests of `crates/sekai-format`, each document type is written to a fixed sequence of bytes and read back from them. In the tests of `crates/sekai-moesekai` (`tests/story_data.rs`), objects built in memory check the root rule, the fields and their order in `details`, an empty `Details`, and an error that is `malformed` and names the field for each absent field and for a field of another type.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
