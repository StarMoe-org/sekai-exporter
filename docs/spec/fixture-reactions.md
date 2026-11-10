# Fixture reactions

[中文](fixture-reactions.zh-CN.md)

The `fixture-reactions` form is the list of the characters who react to each piece of furniture in MySekai, as one JSON document per data set. Converter: `fixture-reactions`.

## 1. Runtime

A runtime that reproduces MySekai shows, in the details of a piece of furniture, each group of characters that reacts to it.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document holds `fixtures`, the rows of the data set in stored order. A row holds:

| Key | Value |
|---|---|
| `fixtureId` | The ID of the piece of furniture |
| `groups` | The groups that react to it, in stored order. A group is a list of character unit IDs, in stored order |

## 3. Sources

A `MonoBehaviour` of the script `Sekai.Mysekai.FixtureReactionDataSet`. `FixturerRactions` holds the rows, a row holds `FixtureId` and `ReactionCharacter`, and each entry of `ReactionCharacter` holds the group as `CharacterUnitIds`.

**Outcomes other than a document:** a field that the object does not hold or that has another type makes the asset an error with the code `malformed`.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters. The asset takes the name of its container path.

## 5. Correctness

Tests on synthetic objects cover the order of the rows and groups and a field of the wrong type. The asset of a real version must hold one row per stored row.

## 6. Version

Form version 1.
