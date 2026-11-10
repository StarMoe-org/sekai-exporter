# 家具对话

[English](fixture-talks.md)

`fixture-talks` 形式是 MySekai 中角色在家具处所说的话，写成一份 JSON 文档。转换器：`fixture-talks`。

## 1. 运行时

复现 MySekai 的运行时依据家具、角色、站点组与期限在家具处选择一段对话，然后显示其推文，并按顺序播放其脚本的步骤。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

文档含有 `version`（1）、`talks`（按表的顺序排列的对话），以及 `missingScripts`（因该版本未提供而被略去的对话的脚本名称，已排序）。一段对话含有：

| 键 | 值 |
|---|---|
| `talkId` | 对话的 ID |
| `lua` | 其脚本的名称，不含扩展名 |
| `form` | 无对话的家具动作表列出了其某一对时为 1，否则为 2 |
| `siteGroupId`、`termId`、`conditionGroupId` | 对话的站点组、期限与条件组 |
| `fixtureIds` | 对话所锚定的家具 |
| `unitIds` | 对话的角色，按其组中各槽位的顺序排列 |
| `pairs` | 这两个列表的每一对 `[家具, 角色]` |
| `tweet` | 与 `talks` 形式相同 |
| `voices`、`steps` | 与 `talks` 形式第 2 节相同 |

## 3. 来源

脚本与表为 `talks` 形式第 3 节所述，以及 `mysekaiCharacterTalkNoTalkMysekaiFixtureActions`，其行为 `mysekaiFixtureId` 与 `gameCharacterUnitId`。

对话至少有一对时进入文档。其家具为类型 `mysekai_fixture_id` 的条件的值。资源包中没有其脚本的对话无法播放，因此被略去，其脚本列入 `missingScripts`。

**文档以外的结果：** 即 `talks` 形式第 3 节所述的那些。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。资产命名为 `mysekai-runtime/fixture-talks`。

## 5. 正确性

针对合成表与脚本的测试覆盖对话的选取、各对、`form` 的两个取值，以及因缺少脚本而被略去的对话。在真实版本上，`missingScripts` 只列出资源包中不存在的名称。

## 6. 版本

形式版本 1。
