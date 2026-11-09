# Fixture talks

The `fixture-talks` form is what characters say at a piece of furniture in MySekai, as one JSON document. Converter: `fixture-talks`.

## 1. Runtime

A runtime that reproduces MySekai chooses a talk at a piece of furniture by the piece, the characters, the site group and the term, then shows its tweet and plays the steps of its script in order.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document holds `version` (1) and `talks`, the talks in table order. A talk holds:

| Key | Value |
|---|---|
| `talkId` | The ID of the talk |
| `lua` | The name of its script, without extension |
| `form` | 1 when the table of furniture actions without a talk lists one of its pairs, otherwise 2 |
| `siteGroupId`, `termId`, `conditionGroupId` | The site group, the term and the condition group of the talk |
| `fixtureIds` | The pieces of furniture the talk is anchored to |
| `unitIds` | The characters of the talk, in the order of the slots of its group |
| `pairs` | Every `[fixture, unit]` of those two lists |
| `tweet` | As in the `talks` form |
| `voices`, `steps` | As in the `talks` form, section 2 |

## 3. Sources

The scripts and tables are those of the `talks` form, section 3, and `mysekaiCharacterTalkNoTalkMysekaiFixtureActions`, with rows of `mysekaiFixtureId` and `gameCharacterUnitId`.

A talk is in the document when it has at least one pair. Its pieces of furniture are the values of its conditions of the type `mysekai_fixture_id`.

**Outcomes other than a document:** those of the `talks` form, section 3.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters. The asset is named `mysekai-runtime/fixture-talks`.

## 5. Correctness

Tests on synthetic tables and scripts cover the selection of talks, the pairs and the two values of `form`. The asset of a real version must have a script for every selected row.

## 6. Version

Form version 1.
