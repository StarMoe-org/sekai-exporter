# 对话

[English](talks.md)

`talks` 形式是 MySekai 中角色所说的话：每个角色的对话、多个角色一起进行的对话、派对脚本与教程脚本，合并成一份 JSON 文档。转换器：`talks`。

## 1. 运行时

复现 MySekai 的运行时依据角色、站点组、期限以及每个条件的值选择一段对话。然后显示该对话的推文，按顺序播放其脚本的步骤，并带上脚本所命名的语音 cue。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

文档含有 `version`（1）、`units`、`commonSomeCharacter`、`birthdayParty`、`tutorial` 与 `missingScripts`，后者为文档各行所命名而该版本未提供的脚本名称，已排序。

**`units`** 是一个对象，从角色单元的 ID（字符串形式）映射到 `{ "talks": [...] }`，即该角色的对话，按表的顺序排列。一段对话含有：

| 键 | 值 |
|---|---|
| `talkId` | 对话的 ID |
| `lua` | 其脚本的名称，不含扩展名 |
| `siteGroupId`、`termId` | 对话的站点组与期限 |
| `conditions` | 对话每个条件的类型，按其条件组的顺序排列 |
| `conditionValues` | 每个条件一项 `{ "conditionType", "conditionTypeValue" }`，顺序相同。条件不含值时，值为 `null` |
| `tweet` | `{ "id", "text", "motion", "eye", "mouth" }`，对话没有推文时为 `null`。推文未指明时，`motion`、`eye` 与 `mouth` 为 `null` |
| `voices` | 脚本的语音 cue，按顺序排列 |
| `steps` | 脚本的步骤，按顺序排列 |

**`commonSomeCharacter`** 含有 `talks`，即两个到五个角色的对话，按表的顺序排列。每段对话含有对话的各键，以及 `conditionGroupId`、`unitIds`（对话的角色，按其组中各槽位的顺序排列）与 `someCharacterTalks`（该对话在 `mysekaiCharacterTalkSomeCharacterTalks` 中的行）。

**`birthdayParty`** 含有 `status`（`"present"`）、`scenarios`（`birthdayPartyScenarios` 的各行，按表的顺序排列，带键 `id`、`birthdayPartyId`、`seq` 与 `lua`），以及 `talks`（从各行所命名的每个脚本的名称映射到其 `voices` 与 `steps`）。

**`tutorial`** 含有 `status`（`"complete"`）、`rows`（`mysekaiTutorialTalks` 的各行，按表的顺序排列，带键 `id`、`assetbundleName`、`lua`，以及该行有时带有的 `mysekaiCharacterTalkTweetId`），以及如上所述的 `talks`。

一个步骤含有 `op`（调用的名称）以及下表各键。标为*传参时*的键在脚本传入该参数时出现，标为*给出时*的键在脚本传入除 `nil` 之外的任何内容时出现。没有标记的键总是出现，脚本省略该参数或传入 `nil` 时为 `null`。`alias` 是命名该参数的常量，参数为字面量时即参数本身。

| `op` | 键 |
|---|---|
| `label` | `name` *传参时* |
| `text` | `text` *传参时* |
| `wait_click`、`show_talk_window`、`hide_talk_window` | 无 |
| `wait_time`、`wait_time_on_auto_mode` | `seconds`、`auto`，均*传参时* |
| `change_npc_eye`、`change_npc_mouth` | `who`、`pattern`、`alias`，均*传参时*；`delaySeconds` |
| `change_fixture_character_eye`、`change_fixture_character_mouth` | `fixture`、`pattern`、`alias`，均*传参时*；`delaySeconds` |
| `change_animation` | `who`、`motion`、`alias`，均*传参时*；`delay`；`animationSpeed`；`playEndMotion`（参数的真值），`blend` 与 `phase`，均*给出时* |
| `play_animation` | `who`、`motion`、`alias`，均*传参时*；`delay`；`animationSpeed`；`blend` *给出时* |
| `voice` | `channel`、`cue`、`who`，均*传参时* |
| `fixture_voice` | `cue`、`fixture`，均*传参时* |
| `look_at_body` | `who`、`target`、`duration`，均*传参时* |
| `look_at_to_npc` | `fixture`、`who`、`duration`，均*传参时* |
| `look_at_fixture` | `fixture`、`who`，均*传参时* |
| `emoticon` | `who`、`name`、`alias`、`showSeconds`，均*传参时* |
| `show_fixture_emoticon` | `fixture`、`name`、`alias`、`showSeconds`，均*传参时* |
| `hide_emoticon` | `who` *传参时* |
| `change_fixture_timeline`、`play_fixture_gimmick`、`stop_fixture_gimmick` | `fixture`、`name`、`value`，均*传参时* |

`voices` 保存每个 `voice` 与 `fixture_voice` 步骤的 `cue`。

## 3. 来源

**脚本：** 资源包 `mysekai/talk/scenario/talk` 中的 `text` 资产，按某行的 `lua` 命名。资源包 `mysekai/talk/scenario/lib` 中的 `defines.lua` 定义参数所引用的常量。其常量表未定义的键，作为值与别名都保留脚本所写的记号。

**游戏数据表：**

| 表 | 用途 |
|---|---|
| `mysekaiCharacterTalks` | 对话 |
| `mysekaiGameCharacterUnitGroups` | 对话的角色：不为零的 `gameCharacterUnitId1` 至 `gameCharacterUnitId5` |
| `mysekaiCharacterTalkConditionGroups`、`mysekaiCharacterTalkConditions` | 对话的各条件及其类型与值 |
| `mysekaiCharacterTalkPreActions`、`mysekaiCharacterTalkTweets` | 对话的推文。推文 ID 为 0 表示没有 |
| `mysekaiCharacterTalkSomeCharacterTalks` | 使多个角色的对话成为共同对话的行 |
| `birthdayPartyScenarios`、`mysekaiTutorialTalks` | 派对脚本与教程脚本 |

对话的组只有一个角色，且其条件都不是 `mysekai_fixture_id`、`mysekai_fixture_tag_id` 或 `after_set_fixture` 类型时，对话属于 `units`。对话的组有两个到五个角色、其条件都不是 `mysekai_fixture_id` 类型，且链接表中有它的一行时，对话属于 `commonSomeCharacter`。其他对话不在文档中。游戏数据可能早于它所命名的脚本进入某个版本：对话、派对行或教程行在资源包中没有其脚本时无法播放，因此被略去，其脚本列入 `missingScripts`。

**文档以外的结果：** 两个资源包都缺失的版本没有资产。无法解析的脚本、不是上述某个操作调用的语句，或其表未持有的角色组、条件组、条件或推文，使资产成为错误码为 `malformed` 的错误。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。资产命名为 `mysekai-runtime/talks`。

## 5. 正确性

针对合成表与脚本的测试覆盖每种操作调用的键、对话的选取、各表的连接、因缺少脚本而被略去的行，以及上述每一种错误结果。在真实版本上，`missingScripts` 只列出资源包中不存在的名称。

## 6. 版本

形式版本 1。
