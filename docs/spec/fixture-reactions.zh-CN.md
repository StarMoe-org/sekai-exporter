# 家具反应

[English](fixture-reactions.md)

`fixture-reactions` 形式是 MySekai 中对每件家具作出反应的角色列表，每个数据集写成一份 JSON 文档。转换器：`fixture-reactions`。

## 1. 运行时

复现 MySekai 的运行时在件家具的详情中显示对它的每一组作出反应的角色。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

文档含有 `fixtures`，即按存储顺序排列的数据集各行。一行含有：

| 键 | 值 |
|---|---|
| `fixtureId` | 家具的 ID |
| `groups` | 对它作出反应的各组，按存储顺序排列。一个组是按存储顺序排列的角色单元 ID 列表 |

## 3. 来源

脚本为 `Sekai.Mysekai.FixtureReactionDataSet` 的 `MonoBehaviour`。`FixturerRactions` 保存各行，一行保存 `FixtureId` 与 `ReactionCharacter`，`ReactionCharacter` 的每一项以 `CharacterUnitIds` 保存该组。

**文档以外的结果：** 对象没有的字段或类型不符的字段，使资产成为错误码为 `malformed` 的错误。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。资产取其容器路径的名称。

## 5. 正确性

针对合成对象的测试覆盖各行与各组的顺序以及类型错误的字段。真实版本的资产必须为每个所存的行保存一行。

## 6. 版本

形式版本 1。
