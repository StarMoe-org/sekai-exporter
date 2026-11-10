# 角色

[English](characters.md)

`characters` 形式是 MySekai 的角色名单，写成一份 JSON 文档。转换器：`characters`。

## 1. 运行时

复现 MySekai 的运行时以名称与单元颜色显示每个角色，播放其待机动作与行走动作，以其自身的速度移动它，在步与步之间暂停它，并启动其独处动作脚本。它在加载时读取文档一次。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

文档含有 `version`（1）与 `characters`，后者以十进制的角色单元 ID 为键，按单元升序排列。每一项含有：

| 键 | 值 |
|---|---|
| `unitId` | 角色单元 ID，与键相同 |
| `name` | 显示名称：先姓氏后名字，二者都可以不存在 |
| `gameCharacter` | `id`，即游戏角色 ID，以及 `gender`，按游戏数据中的写法 |
| `identity` | `gameCharacterId`，与 `gameCharacter.id` 相同；`unit`，角色所属的团体；`colorCode`，角色的颜色，写作 `#rrggbb` |
| `locomotion` | `idleMotion` 与 `walkMotion`，动作库中动作族系的名称；`pauseSeconds`，步与步之间的暂停；`walkSpeedMetersPerSecond`，行走速度 |
| `soloAction` | 角色独处动作脚本的名称，游戏数据未绑定时为 `null` |

## 3. 来源

**游戏数据表：**

- `mysekaiCharacterTalkMotions`：每个角色一行，据此决定名单。`gameCharacterUnitId`、`idleMotion`、`walkMotion`、`pauseMilliSeconds` 与 `walkSpeed`。
- `gameCharacterUnits`：`id` 为该单元的那一行。`gameCharacterId`、`unit` 与 `colorCode`。
- `gameCharacters`：`id` 为该游戏角色的那一行。`firstName`、`givenName` 与 `gender`。
- `mysekaiCharacterTalkSoloActions`：`gameCharacterUnitId` 为该单元的那一行（如果有）。`lua`。

`pauseSeconds` 为 `pauseMilliSeconds` 除以 1000。`walkSpeedMetersPerSecond` 为 `walkSpeed` 除以 1000：游戏存储的速度比其移动的每秒米数大一千倍。

某一行所引用的单元或游戏角色没有自己的一行时，为错误 `malformed`。

## 4. 容器

UTF-8 JSON，不带字节顺序标记，紧凑，对象的键按上表给出的顺序排列，角色按单元升序排列。

## 5. 正确性

所存的值与转换后的值只通过上述除法相关联。针对合成行的单元测试检查连接、除法、名称、没有独处动作脚本的角色，以及两种缺失的引用。真实版本与一份由同一批表独立产生的文档逐键比对。

## 6. 版本

1。
