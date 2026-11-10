# Communication actions

[中文](communication-actions.zh-CN.md)

The `communication-actions` form is what each MySekai character does while it is together with others, as one JSON document. Converter: `communication-actions`.

## 1. Runtime

A runtime that reproduces MySekai plays the program of a character for as long as the character takes part in a communication. The document gives the program as data, so that the runtime needs no Lua interpreter.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document holds `version` (1) and `units`, an object from the ID of the character unit, as a string, to `{ "program": ... }`.

A program holds:

| Key | Value |
|---|---|
| `kind` | `"whileCommunication"` |
| `random` | `{ "min": 0, "max": 99 }` when each pass of the loop draws a number, otherwise `null` |
| `branches` | The branches tried in order. Only the first one whose condition holds runs |
| `steps` | The steps every pass runs after the branches. They hold at least one wait of a positive time |

A branch holds `steps` and `condition`, with `randomBelow`, the number the draw must be below, and `participant`, the character unit that must take part, or `null`.

A step holds `op`, `t`, the sum of the waits before it in its sequence of steps, and, except for a wait, `who`, the character unit it acts on:

| `op` | Further keys |
|---|---|
| `eye`, `mouth` | `pattern`; `alias`, the constant that named it |
| `animation` | `motion`; `alias`; `speed` as written, `null` when left out; `playbackSpeed`, the rate to play at, 1 when `speed` is 0 or left out; `playEndMotion`; `blend`, the crossfade in seconds; when given, `phase`, one of `S`, `L`, `E` and `O`, and `phaseSource`, the constant that named it |
| `emoticon` | `name`; `time` and `notPlaySe` as written, `null` when left out; `hostNotPlaySe`, whether `time` is true to Lua; `showSeconds` when written |
| `hideEmoticon` | None. It follows an `emoticon` whose `showSeconds` is not `null`, at the same time |
| `wait` | `seconds` |

## 3. Sources

**Scripts:** the `text` assets of the bundle `mysekai/some_character_communication`. `defines.lua` defines the constants that arguments name, `lib.lua` the default crossfade `animation_blend_time`, and a script of its own holds each program.

**Game data table:** `mysekaiCharacterTalkTogetherCommunications`. Each row gives a character unit (`gameCharacterUnitId`) and the script of its program (`lua`).

A program is one `while enable_communication() do ... end`. Its body may start with `rand = math.random(0, 99)` and an `if` of conditions `is_communicating(character) and rand < n` or `rand < n`. Every other statement is a call of `change_npc_eye`, `change_npc_mouth`, `change_animation`, `emoticon`, `hide_emoticon` or `wait_time`.

**Outcomes other than a document:** a version without scripts in the bundle has no asset. A script that does not parse, that has any other statement, or that names a key its constant table does not define makes the asset an error with the code `malformed`. So does a table with two programs for one character unit.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters. The asset is named `mysekai-runtime/communication-actions`.

## 5. Correctness

Tests on synthetic scripts cover each kind of step, the plain and the random loop, the times of the steps and every outcome above that is an error. The asset of a real version must have a program for every row of the table.

## 6. Version

Form version 1.
