# 动作集

[English](action-set.md)

`action-set` 形式是一个动作集的区域出场角色，以及每个角色所穿的一组 2D 服装，写成 JSON 文档。转换器：`sekai-moesekai` 中的 `action-set`（feature `episode`）。形式版本：1。

## 1. 运行时

构建 `episode` 资产的数据表阶段，在它构建一段区域对话的一话时：它读取该对话所属区域的动作集，为对话显示的每个区域角色选择服装组。形式给出所存的列表，而不给出游戏由它们推出的任何内容；它不把服装组解析为服装、模型或动作。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `action-set` | JSON 文档，扩展名 `json` |

```json
{ "format": "action-set", "version": 1, "action_set_id": 7001, "area_id": 12,
  "details": [
    { "area_character_id": 3, "costume2d_group_id": 31 },
    { "area_character_id": 1, "costume2d_group_id": -2 }
  ] }
```

| 键 | 值 |
|---|---|
| `format`、`version` | `"action-set"` 与本文档格式的版本，1 |
| `action_set_id` | 对象的动作集 ID |
| `area_id` | 对象的区域 ID |
| `details` | 区域角色及其服装组，按对象存储它们的顺序 |
| `area_character_id` | 区域角色 |
| `costume2d_group_id` | 区域角色所穿的 2D 服装组 |

数写为十进制的有符号整数。

## 3. 来源

**根对象：** 脚本为 `Assembly-CSharp/Sekai.ActionSetData` 的每个 `MonoBehaviour`（类 114）。一个资源包可以带有多个动作集；每个各自为根。

**读取的字段：** 根对象的 `ActionSetId`、`AreaId` 与 `Details`；`Details` 每一项的 `AreaCharacterId` 与 `Costume2dGroupId`。对象的其他字段都不读取。

形式不读取游戏数据表，也不引用其他对象。

**文档以外的结果：**

| 情况 | 结果 |
|---|---|
| `ActionSetId`、`AreaId`、`AreaCharacterId` 或 `Costume2dGroupId` 不存在，或不是数字类型的整数 | 错误 `malformed` |
| `Details` 或其某一项不存在、`Details` 不是数组，或某一项不是结构 | 错误 `malformed` |

错误信息写明字段，例如 `Details[1].Costume2dGroupId`。

## 4. 容器

成员是成品字节：UTF-8 JSON，无多余空白，键的顺序与上文相同，`details` 的各项按存储顺序排列。

## 5. 正确性

文档不加解释地保存所存的值，因此从它读取服装组的一话，读到的是游戏对同一对象所读的同一个组，且同一对象总是给出相同的字节。

在 `crates/sekai-format` 的测试中，每种文档类型都写成一串固定的字节，再从这些字节读回。在 `crates/sekai-moesekai` 的测试（`tests/story_data.rs`）中，在内存中构建的对象检查根规则、字段及其在 `details` 中的顺序、空的 `Details`，以及对每个缺失字段和某个类型不符的字段给出 `malformed` 并写明该字段的错误。

## 6. 版本

| 版本 | 变更 |
|---|---|
| 1 | 第一版 |
