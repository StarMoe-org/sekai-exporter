# Alone actions

[中文](alone-actions.zh-CN.md)

The `alone-actions` form is what each MySekai character does while it is on its own, as one JSON document. Converter: `alone-actions`.

## 1. Runtime

A runtime that reproduces MySekai plays the program of a character for as long as the character is on its own. The document gives the program as data, so that the runtime needs no Lua interpreter.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document holds `version` (3) and `units`, an object from the ID of the character unit, as a string, to a unit:

| Key | Value |
|---|---|
| `asset` | The name of the script, with its extension |
| `scenarios` | The conditional blocks of the loop, in source order |
| `tail` | `{ "steps": [...] }`, what runs after the blocks on every pass, also when no block ran. It holds at least one wait of a positive time |
| `program` | How the blocks are evaluated |

A scenario holds `id`, `trigger` and `steps`. The `id` is the name of the motion slot its gate tests, otherwise `gate@n` with `n` the place of the block in the loop, and `rand<n` for a random branch.

A trigger with `kind` `"timeGated"` holds:

| Key | Value |
|---|---|
| `timeLimitName`, `timeLimitSeconds` | The constant of the time window and its value. The gate holds while fewer than this many wall-clock seconds have passed since the script started |
| `probabilityName`, `probability` | The constant of the chance and its value. The gate draws an integer from 0 to 99 of its own and holds when the draw is below `probability * 100` |
| `motionSlotValue` | The slot the gate tests, or `null`. The gate holds when the slot was never remembered or was remembered more than `slotMemorySeconds` before the time captured at the start of the pass |
| `slotMemorySeconds` | The time a remembered slot blocks its gate |

A trigger with `kind` `"randomBranch"` holds `low` and `high`, the draw of the pass falling in `low <= draw < high`, and `weight`, the share of draws in that range.

A program holds:

| Key | Value |
|---|---|
| `kind` | `"sequentialIfs"`: every block is tried in turn on each pass. `"randomBranch"`: the one block whose range holds the draw runs |
| `clock` | `"wallSeconds"` |
| `loopRandomMin`, `loopRandomMax` | 0 and 99, the range of the draw at the start of each pass |
| `captureLoopTime` | Whether each pass captures the time after its draw. True for `"sequentialIfs"` |
| `blocks` | One `{ "scenario", "rememberSlot" }` per block in evaluation order. `scenario` is the index into `scenarios`. `rememberSlot` is the slot that the block writes with the captured time after its steps, or `null` |

A step holds `op`, `t`, the sum of the waits before it in its sequence of steps, and the keys of its operation. The operations and their keys are those of the `communication-actions` form, section 2, without `who`: a script acts only on its own character.

## 3. Sources

**Scripts:** the `text` assets of the bundle `mysekai/character_alone_action`. `defines.lua` defines the constants that arguments name, `lib.lua` the default crossfade `animation_blend_time`, and a script of its own holds each program.

**Game data table:** `mysekaiCharacterTalkSoloActions`. Each row gives a character unit (`gameCharacterUnitId`) and the script of its program (`lua`).

A script is local definitions followed by one `while (is_end(character) == false) do ... end`. The body opens with `rand = math.random(0, 99)`, as a local or not, then has one of two shapes:

- `local nowTime = os.time()`, which may also come before the draw, then a run of `if` statements with one arm whose condition is `hasTimeElapsed(limit) and shouldExecuteWithProbability(probability)`, optionally `and canSelectMotion(slot, nowTime)`. The last statement of a block may be `lastSelectedTimes[slot] = nowTime` for the slot it tests. Numbers come from the local definitions of the script, which must also define `hasTimeElapsed`, `shouldExecuteWithProbability`, `canSelectMotion`, `startTime` and `lastSelectedTimes` as section 2 describes them.
- One `if` whose conditions are `rand < n` in rising order.

Every other statement in the body is a call of `change_npc_eye`, `change_npc_mouth`, `change_animation`, `emoticon`, `hide_emoticon` or `alone_wait_time` on the character of the row.

**Outcomes other than a document:** a version without scripts in the bundle has no asset. A script that does not parse, that departs from the shapes above, that acts on another character or that names a key its constant table does not define makes the asset an error with the code `malformed`. So does a table with two programs for one character unit.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters. The asset is named `mysekai-runtime/alone-actions`.

## 5. Correctness

Tests on synthetic scripts cover both shapes of loop, the constants a gate names, the slots a block remembers and every outcome above that is an error. The asset of a real version must have a program for every row of the table.

## 6. Version

Form version 1.
