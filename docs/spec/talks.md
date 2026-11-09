# Talks

The `talks` form is what characters say in MySekai: the talks of each character, the talks several characters have together, the party scripts and the tutorial scripts, as one JSON document. Converter: `talks`.

## 1. Runtime

A runtime that reproduces MySekai chooses a talk by the character, the site group, the term and the value of each condition. It then shows the tweet of the talk and plays the steps of its script in order, with the voice cues the script names.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document holds `version` (1), `units`, `commonSomeCharacter`, `birthdayParty` and `tutorial`.

**`units`** is an object from the ID of a character unit, as a string, to `{ "talks": [...] }`, the talks of that character in table order. A talk holds:

| Key | Value |
|---|---|
| `talkId` | The ID of the talk |
| `lua` | The name of its script, without extension |
| `siteGroupId`, `termId` | The site group and the term of the talk |
| `conditions` | The type of each condition of the talk, in the order of its condition group |
| `conditionValues` | One `{ "conditionType", "conditionTypeValue" }` per condition, in the same order. The value is `null` when the condition holds none |
| `tweet` | `{ "id", "text", "motion", "eye", "mouth" }`, or `null` when the talk has none. `motion`, `eye` and `mouth` are `null` when the tweet names none |
| `voices` | The voice cues of the script, in order |
| `steps` | The steps of the script, in order |

**`commonSomeCharacter`** holds `talks`, the talks of two to five characters in table order. Each holds the keys of a talk, and `conditionGroupId`, `unitIds`, the characters of the talk in the order of the slots of the group, and `someCharacterTalks`, the rows of `mysekaiCharacterTalkSomeCharacterTalks` of the talk.

**`birthdayParty`** holds `status` (`"present"`), `scenarios`, the rows of `birthdayPartyScenarios` in table order with their keys `id`, `birthdayPartyId`, `seq` and `lua`, and `talks`, an object from the name of each script the rows name to its `voices` and `steps`.

**`tutorial`** holds `status` (`"complete"`), `rows`, the rows of `mysekaiTutorialTalks` in table order with their keys `id`, `assetbundleName`, `lua` and, when the row has one, `mysekaiCharacterTalkTweetId`, and `talks` as above.

A step holds `op`, the name of the call, and the keys of the table below. A key marked *when passed* is present when the script passes that argument, and a key marked *when given* when it passes anything but `nil`. A key without a mark is always present, and is `null` when the script leaves the argument out or passes `nil`. An `alias` is the constant that named the argument, or the argument itself when it is a literal.

| `op` | Keys |
|---|---|
| `label` | `name` *when passed* |
| `text` | `text` *when passed* |
| `wait_click`, `show_talk_window`, `hide_talk_window` | None |
| `wait_time`, `wait_time_on_auto_mode` | `seconds`, `auto`, both *when passed* |
| `change_npc_eye`, `change_npc_mouth` | `who`, `pattern`, `alias`, all *when passed*; `delaySeconds` |
| `change_fixture_character_eye`, `change_fixture_character_mouth` | `fixture`, `pattern`, `alias`, all *when passed*; `delaySeconds` |
| `change_animation` | `who`, `motion`, `alias`, all *when passed*; `delay`; `animationSpeed`; `playEndMotion`, the truth of the argument, `blend` and `phase`, all *when given* |
| `play_animation` | `who`, `motion`, `alias`, all *when passed*; `delay`; `animationSpeed`; `blend` *when given* |
| `voice` | `channel`, `cue`, `who`, all *when passed* |
| `fixture_voice` | `cue`, `fixture`, both *when passed* |
| `look_at_body` | `who`, `target`, `duration`, all *when passed* |
| `look_at_to_npc` | `fixture`, `who`, `duration`, all *when passed* |
| `look_at_fixture` | `fixture`, `who`, both *when passed* |
| `emoticon` | `who`, `name`, `alias`, `showSeconds`, all *when passed* |
| `show_fixture_emoticon` | `fixture`, `name`, `alias`, `showSeconds`, all *when passed* |
| `hide_emoticon` | `who` *when passed* |
| `change_fixture_timeline`, `play_fixture_gimmick`, `stop_fixture_gimmick` | `fixture`, `name`, `value`, all *when passed* |

`voices` holds the `cue` of every `voice` and `fixture_voice` step.

## 3. Sources

**Scripts:** the `text` assets of the bundle `mysekai/talk/scenario/talk`, named by the `lua` of a row. `defines.lua` of the bundle `mysekai/talk/scenario/lib` defines the constants that arguments name. A key that its constant table does not define stays the token the script wrote, as value and as alias.

**Game data tables:**

| Table | Use |
|---|---|
| `mysekaiCharacterTalks` | The talks |
| `mysekaiGameCharacterUnitGroups` | The characters of a talk: the non-zero `gameCharacterUnitId1` to `gameCharacterUnitId5` |
| `mysekaiCharacterTalkConditionGroups`, `mysekaiCharacterTalkConditions` | The conditions of a talk and their types and values |
| `mysekaiCharacterTalkPreActions`, `mysekaiCharacterTalkTweets` | The tweet of a talk. A tweet ID of 0 names none |
| `mysekaiCharacterTalkSomeCharacterTalks` | The rows that make a talk of several characters a common one |
| `birthdayPartyScenarios`, `mysekaiTutorialTalks` | The party and tutorial scripts |

A talk belongs to `units` when its group holds one character and none of its conditions is of the type `mysekai_fixture_id`, `mysekai_fixture_tag_id` or `after_set_fixture`. A talk belongs to `commonSomeCharacter` when its group holds two to five characters, none of its conditions is of the type `mysekai_fixture_id`, and the table of links has a row for it. Other talks are not in the document.

**Outcomes other than a document:** a version without either bundle has no asset. A script that does not parse or is not found, a statement that is not a call of an operation above, a unit group, condition group, condition or tweet that its table does not hold makes the asset an error with the code `malformed`.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters. The asset is named `mysekai-runtime/talks`.

## 5. Correctness

Tests on synthetic tables and scripts cover the keys of each kind of operation, the selection of talks, the joins of the tables and every outcome above that is an error. The asset of a real version must have a script for every selected row.

## 6. Version

Form version 1.
