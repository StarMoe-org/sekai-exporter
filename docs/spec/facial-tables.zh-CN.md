# 表情表

[English](facial-tables.md)

表情表说明角色的眼睛贴图与嘴部贴图的哪些格显示每种表情。`facial-table` 形式是游戏所存储的一张此类表。`facial-tables` 形式把一个版本的三张表合并成一份文档。转换器：`facial-table` 与 `facial-tables`。

## 1. 运行时

复现 MySekai 的运行时按名称设置角色的面部：一个眼睛图案与一个嘴部图案。它查找每个名称以找到要显示的贴图格，只让允许眨眼的图案眨眼，并让每个角色以默认面部开始。它在加载时读取合并后的文档一次。

## 2. 内容

两种形式都有一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

`facial-table` 的文档含有表的序列化字段，不含每个 `MonoBehaviour` 都有的字段。

`facial-tables` 的文档含有三个数组，各按其所对应表的顺序排列：

`NPCAvatarEyeData`，眼睛图案：

| 键 | 值 |
|---|---|
| `PatternName` | 图案的名称 |
| `OpenEyeIndex` | 睁眼时的贴图格 |
| `CloseEyeIndex` | 闭眼时的贴图格 |
| `BlinkEnabled` | 图案眨眼时为 1，否则为 0 |
| `TextureIndex` | 这些格所在眼睛贴图的序号。在其表中没有此列的版本中为 0，因为这类版本只有一张眼睛贴图 |

`NPCAvatarLipSyncData`，嘴部图案：

| 键 | 值 |
|---|---|
| `Name` | 图案的名称 |
| `OpenLipSyncIndex` | 张嘴时的贴图格 |
| `MiddleLipSyncIndex` | 半张嘴时的贴图格，图案没有此格时为 -1 |
| `CloseLipSyncIndex` | 闭嘴时的贴图格 |

`NPCAvatarDefaultFacialData`，默认面部：

| 键 | 值 |
|---|---|
| `CharacterUnitId` | 角色单元 ID |
| `EyePatternName` | 其眼睛图案的名称 |
| `MouthPatternName` | 其嘴部图案的名称 |

## 3. 来源

**`facial-table` 的对象：** 程序集 `Assembly-CSharp` 中脚本为 `Sekai.Mysekai.NPCAvatarEyeDataList`、`Sekai.Mysekai.NPCAvatarLipSyncDataList` 与 `Sekai.Mysekai.NPCAvatarDefaultFacialDataList` 的每个 `MonoBehaviour`。

**`facial-tables` 的资产：** 包 `mysekai/character_settings` 中的 `facial-table` 资产 `npcavatareyedata`、`npcavatarlipsyncdata` 与 `npcavatardefaultfacialdata`。它们的列表位于字段 `EyeDataList`、`LipSyncDataList` 与 `NPCAvatarDefaultFacialData` 中。包中没有 `facial-table` 资产的版本没有 `facial-tables` 资产。一张表缺失而另一张存在，或某行缺少除 `TextureIndex` 以外的列，均为错误。

## 4. 容器

UTF-8 JSON，不带字节顺序标记，紧凑。值为所存的值。

## 5. 正确性

每个值都复制自其所指的表中，因此对象被精确读取且列即上表所列时，文档正确。按带类型的列读取一行，使改名或缺失的列成为错误，而不是缺失的值。针对合成文档的单元测试检查合并与每种错误。真实版本与对同一批对象的独立读取逐行比对。

## 6. 版本

`facial-table` 1，`facial-tables` 1。
