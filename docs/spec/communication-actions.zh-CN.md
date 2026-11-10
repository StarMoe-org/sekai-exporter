# 交流动作

[English](communication-actions.md)

`communication-actions` 形式是每个 MySekai 角色与他人一起时所做的行为，写成一份 JSON 文档。转换器：`communication-actions`。

## 1. 运行时

复现 MySekai 的运行时，在角色参与交流期间播放该角色的程序。文档以数据形式给出程序，因此运行时无需 Lua 解释器。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

文档含有 `version`（1）与 `units`，后者从角色单元的 ID（字符串形式）映射到 `{ "program": ... }`。

一个程序含有：

| 键 | 值 |
|---|---|
| `kind` | `"whileCommunication"` |
| `random` | 循环每轮都抽取一个数时为 `{ "min": 0, "max": 99 }`，否则为 `null` |
| `branches` | 按顺序尝试的分支。只运行第一个条件成立的分支 |
| `steps` | 每轮在分支之后运行的步骤。其中至少有一个时长为正的等待 |

一个分支含有 `steps` 与 `condition`，`condition` 带有 `randomBelow`（抽取结果必须小于的数）与 `participant`（必须参与的角色单元，或 `null`）。

一个步骤含有 `op`、`t`（其步骤序列中在它之前的等待之和），以及除等待以外的 `who`，即它所作用的角色单元：

| `op` | 其他键 |
|---|---|
| `eye`、`mouth` | `pattern`；`alias`，即命名它的常量 |
| `animation` | `motion`；`alias`；按书写原样的 `speed`，省略时为 `null`；`playbackSpeed`，即播放速率，`speed` 为 0 或省略时为 1；`playEndMotion`；`blend`，即交叉淡入淡出，以秒计；给出时为 `phase`，取 `S`、`L`、`E`、`O` 之一，以及 `phaseSource`，即命名它的常量 |
| `emoticon` | `name`；按书写原样的 `time` 与 `notPlaySe`，省略时为 `null`；`hostNotPlaySe`，即 `time` 在 Lua 中是否为真；书写时给出 `showSeconds` |
| `hideEmoticon` | 无。它跟随 `showSeconds` 不为 `null` 的某个 `emoticon`，同时发生 |
| `wait` | `seconds` |

## 3. 来源

**脚本：** 资源包 `mysekai/some_character_communication` 中的 `text` 资产。`defines.lua` 定义参数所引用的常量，`lib.lua` 定义默认的交叉淡入淡出 `animation_blend_time`，每个程序由它自己的一个脚本承载。

**游戏数据表：** `mysekaiCharacterTalkTogetherCommunications`。每行给出一个角色单元（`gameCharacterUnitId`）及其程序的脚本（`lua`）。

一个程序是一个 `while enable_communication() do ... end`。其循环体可以以 `rand = math.random(0, 99)` 和一个 `if` 开头，该 `if` 的条件为 `is_communicating(character) and rand < n` 或 `rand < n`。其他每条语句都是对 `change_npc_eye`、`change_npc_mouth`、`change_animation`、`emoticon`、`hide_emoticon` 或 `wait_time` 的调用。

**文档以外的结果：** 资源包中没有脚本的版本没有资产。无法解析的脚本、含有其他语句的脚本，或引用了其常量表未定义的键的脚本，使资产成为错误码为 `malformed` 的错误。同一角色单元有两个程序的表也是如此。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。资产命名为 `mysekai-runtime/communication-actions`。

## 5. 正确性

针对合成脚本的测试覆盖每一种步骤、普通循环与随机循环、各步骤的时刻，以及上述每一种错误结果。真实版本的资产必须为表中的每一行都有一个程序。

## 6. 版本

形式版本 1。
