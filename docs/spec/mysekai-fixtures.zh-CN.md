# 家具

[English](mysekai-fixtures.md)

`mysekai-fixtures` 形式是 MySekai 的家具，写成一份 JSON 文档。转换器：`mysekai-fixtures`。

## 1. 运行时

复现 MySekai 的运行时在住居站点上放置、收纳、排序与列出家具。为此它逐件读取：这件家具是什么，放置的家具为角色提供什么，它如何叠放与收纳，可以放在哪里，有多大，花费多少，有哪些颜色格，放置时播放什么声音，其名称与描述，以及其家具类别。文档把这些事实写成游戏数据表每行一条。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

文档含有 `version`（1）与 `fixtures`。

`fixtures` 是家具条目的数组，游戏数据表每行一条，按 `id` 升序排列。每个条目含有下列键：

| 键 | 值 |
|---|---|
| `assetbundleName` | 这件家具的资源包名称。其模型包为该名称加固定前缀 `mysekai__fixture__`。一个模型可以由多个条目指明 |
| `colorIds` | 这件家具的颜色格：其其他颜色中大于 0 的贴图 ID 去重并按升序排列，然后是 1，它始终是一个格 |
| `description` | 风味文本 |
| `firstPutCost` | 首次放置的布局代价，在其表中没有此列的版本中为 `null` |
| `fixtureType` | `system`、`custom`、`plant`、`house_plant`、`surface_appearance`、`gate`、`normal` 或 `canvas` |
| `fixtureTypeValue` | `fixtureType` 的编号：`system` 0，`custom` 1，`plant` 2，`house_plant` 3，`surface_appearance` 4，`gate` 5，`normal` 6，`canvas` 7 |
| `gridDepth` | 这件家具在家具格上的深度，以格计 |
| `gridHeight` | 这件家具在地面之上的高度，以格计 |
| `gridWidth` | 这件家具在家具格上的宽度，以格计 |
| `handleType` | 这件家具的把手，按书写原样，例如 `none`、`block` 或 `light`。它决定收纳与叠放规则，`block_transparent` 把手使这件家具的地面格保持可通行 |
| `id` | 该行的 id |
| `layoutType` | 这件家具的可设置布局，按书写原样，例如 `floor`、`rug`、`road`、`wall`、`wall_appearance` 或 `floor_appearance` |
| `mainGenreId` | 这件家具的家具类别，为主类别表的一个 id |
| `name` | 显示名称 |
| `playerActionType` | `no_action`、`loop`、`timeline` 或 `one_shot`：放置的家具为使用它的角色提供什么。`loop` 或 `one_shot` 的家具运行一个机关，`timeline` 的家具运行一次角色互动 |
| `playerActionTypeValue` | `playerActionType` 的编号：`no_action` 0，`loop` 1，`timeline` 2，`one_shot` 3 |
| `putSoundId` | 放置声音表中指明放置时播放的 cue 的行的 id |
| `putType` | 这件家具的放置类型，按书写原样，例如 `none`、`put_base` 或 `put_target`。它决定哪些家具放在一起 |
| `secondPutCost` | 首次之后放置的布局代价，在其表中没有此列的版本中为 `null` |
| `subGenreId` | 这件家具的子类别，为子类别表的一个 id，或 `null` |

读取方先依据 `layoutType`、再依据 `handleType` 判断墙、窗玻璃或钟是墙面家具，道路是道路家具。墙与地板外观共用一个模型，因此按包识别家具的运行时读取各 id 的条目，而不是一个包一条。

## 3. 来源

**游戏数据表：** `mysekaiFixtures`。每行给出列 `id`、`name`、`flavorText`、`gridSize`（带有 `width`、`depth` 与 `height` 的对象）、`mysekaiFixtureType`、`mysekaiFixturePlayerActionType`、`mysekaiFixturePutType`、`mysekaiFixtureHandleType`、`mysekaiFixtureMainGenreId`、`mysekaiSettableLayoutType`、`mysekaiFixtureAnotherColors`（带有 `textureId` 的对象）、`mysekaiFixturePutSoundId`、`assetbundleName`、`firstPutCost` 与 `secondPutCost`。`mysekaiFixtureSubGenreId` 为整数或 `null`。

条目的键按如下方式写出：

| 条目键 | 列 |
|---|---|
| `id`、`assetbundleName`、`name`、`putSoundId`、`firstPutCost`、`secondPutCost` | 同名的列 |
| `description` | `flavorText` |
| `fixtureType` | `mysekaiFixtureType` |
| `playerActionType` | `mysekaiFixturePlayerActionType` |
| `putType` | `mysekaiFixturePutType` |
| `handleType` | `mysekaiFixtureHandleType` |
| `mainGenreId`、`subGenreId` | `mysekaiFixtureMainGenreId`、`mysekaiFixtureSubGenreId` |
| `gridWidth`、`gridDepth`、`gridHeight` | `gridSize.width`、`gridSize.depth`、`gridSize.height` |
| `layoutType` | `mysekaiSettableLayoutType` |
| `colorIds` | `mysekaiFixtureAnotherColors` 每一项的 `textureId` |

`fixtureTypeValue` 与 `playerActionTypeValue` 不是列：各自是其词在第 2 节所列的编号，因此这两个键不会互相矛盾，而两个列表之外的词会成为错误，而不是一件悄无声息什么都提供不了的家具。

**文档以外的结果：** 没有该表的版本使本次运行以表查找的错误失败。某行缺少它必须给出的列、缺少格的大小、含有两个列表之外的词，或含有其他类型的值，使资产成为错误码为 `malformed` 的错误。同一 id 的两行也是如此。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。资产命名为 `mysekai-runtime/mysekai-fixtures`。

## 5. 正确性

针对合成游戏数据行的测试覆盖各列的映射、每种类型与每种动作的词与编号、颜色格、按 id 排序、某行必须给出的每个键，以及上述每一种错误结果。

条目的每个键都由复现游戏的运行时读取：`id` 与 `assetbundleName` 标识这件家具；`name`、`description`、`mainGenreId` 与 `subGenreId` 绘制与筛选家具列表；`fixtureType` 与 `playerActionType` 及其编号选择放置家具的动作；`putType` 与 `handleType` 是叠放与收纳的规则；`gridWidth`、`gridDepth` 与 `gridHeight` 决定这件家具在格上的大小；`layoutType` 放置墙、地毯与道路；`putSoundId` 指明放置的 cue；`firstPutCost` 与 `secondPutCost` 为布局代价；`colorIds` 指明这件家具提供的颜色格。该行的 `mysekaiSettableSiteType`、`mysekaiFixtureTagGroup`、`pronunciation`、`seq` 及其他列描述游戏数据，但不写出。

## 6. 版本

形式版本 1。
