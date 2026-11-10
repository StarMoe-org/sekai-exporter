# 独处动作

[English](alone-actions.md)

`alone-actions` 形式是每个 MySekai 角色独自一人时所做的行为，写成一份 JSON 文档。转换器：`alone-actions`。

## 1. 运行时

复现 MySekai 的运行时，在角色独自一人期间播放该角色的程序。文档以数据形式给出程序，因此运行时无需 Lua 解释器。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

文档含有 `version`（3）与 `units`，后者从角色单元的 ID（字符串形式）映射到一个单元：

| 键 | 值 |
|---|---|
| `asset` | 脚本的名称，含扩展名 |
| `scenarios` | 循环中的条件块，按源码顺序 |
| `tail` | `{ "steps": [...] }`，每轮在条件块之后运行的内容，没有条件块运行时也运行。其中至少有一个时长为正的等待 |
| `program` | 条件块的求值方式 |

一个场景含有 `id`、`trigger` 与 `steps`。`id` 是其门控所检查的动作槽位名称，否则为 `gate@n`（`n` 为条件块在循环中的位置），随机分支则为 `rand<n`。

`kind` 为 `"timeGated"` 的触发器含有：

| 键 | 值 |
|---|---|
| `timeLimitName`、`timeLimitSeconds` | 时间窗口的常量及其值。自脚本启动以来经过的实际时间少于这么多秒时，门控成立 |
| `probabilityName`、`probability` | 概率的常量及其值。门控自行抽取一个 0 到 99 的整数，抽取结果小于 `probability * 100` 时成立 |
| `motionSlotValue` | 门控所检查的槽位，或 `null`。槽位从未被记住，或在本轮开始时所取的时间之前超过 `slotMemorySeconds` 才被记住时，门控成立 |
| `slotMemorySeconds` | 被记住的槽位阻止其门控成立的时长 |

`kind` 为 `"randomBranch"` 的触发器含有 `low` 与 `high`，即本轮抽取结果落在 `low <= draw < high` 内，以及 `weight`，即该范围内的抽取所占的比例。

一个程序含有：

| 键 | 值 |
|---|---|
| `kind` | `"sequentialIfs"`：每轮依次尝试每个条件块。`"randomBranch"`：运行其所处范围含抽取结果的那一个条件块 |
| `clock` | `"wallSeconds"` |
| `loopRandomMin`、`loopRandomMax` | 0 与 99，即每轮开始时抽取的范围 |
| `captureLoopTime` | 每轮是否在抽取之后取时间。`"sequentialIfs"` 为 true |
| `blocks` | 按求值顺序为每个条件块一项 `{ "scenario", "rememberSlot" }`。`scenario` 是 `scenarios` 中的索引。`rememberSlot` 是条件块在其步骤之后以所取时间写入的槽位，或 `null` |

一个步骤含有 `op`、`t`（其步骤序列中在它之前的等待之和），以及其操作的各键。操作及其键与 `communication-actions` 形式第 2 节相同，但没有 `who`：脚本只作用于自己的角色。

## 3. 来源

**脚本：** 资源包 `mysekai/character_alone_action` 中的 `text` 资产。`defines.lua` 定义参数所引用的常量，`lib.lua` 定义默认的交叉淡入淡出 `animation_blend_time`，每个程序由它自己的一个脚本承载。

**游戏数据表：** `mysekaiCharacterTalkSoloActions`。每行给出一个角色单元（`gameCharacterUnitId`）及其程序的脚本（`lua`）。

一个脚本是若干局部定义，其后是一个 `while (is_end(character) == false) do ... end`。循环体以 `rand = math.random(0, 99)` 开头（无论是否声明为局部变量），随后是下列两种形态之一：

- `local nowTime = os.time()`（也可以位于抽取之前），然后是一串 `if` 语句，其中一支的条件为 `hasTimeElapsed(limit) and shouldExecuteWithProbability(probability)`，可选地再接 `and canSelectMotion(slot, nowTime)`。条件块的最后一条语句可以是对其所检查槽位的 `lastSelectedTimes[slot] = nowTime`。数来自脚本的局部定义，这些定义还必须按第 2 节所述定义 `hasTimeElapsed`、`shouldExecuteWithProbability`、`canSelectMotion`、`startTime` 与 `lastSelectedTimes`。
- 一个 `if`，其条件按升序依次为 `rand < n`。

循环体中的其他每条语句都是对行中角色调用 `change_npc_eye`、`change_npc_mouth`、`change_animation`、`emoticon`、`hide_emoticon` 或 `alone_wait_time`。

**文档以外的结果：** 资源包中没有脚本的版本没有资产。无法解析的脚本、偏离上述形态的脚本、作用于其他角色的脚本，或引用了其常量表未定义的键的脚本，使资产成为错误码为 `malformed` 的错误。同一角色单元有两个程序的表也是如此。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。资产命名为 `mysekai-runtime/alone-actions`。

## 5. 正确性

针对合成脚本的测试覆盖两种循环形态、门控所引用的常量、条件块记住的槽位，以及上述每一种错误结果。真实版本的资产必须为表中的每一行都有一个程序。

## 6. 版本

形式版本 1。
