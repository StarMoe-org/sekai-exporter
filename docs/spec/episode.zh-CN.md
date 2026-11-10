# 话

[English](episode.md)

`episode` 形式是剧情一话所需的全部内容，写成一份指明所用资产的 JSON 文档。转换器：`sekai-moesekai` 中的 `episode`（feature `episode`）。形式版本：2。

## 1. 运行时

按游戏的方式播放一话的剧情播放器与导出器：它们加载场景数据，显示其中角色的 Live2D 模型并播放其动作，显示其背景与影片，并播放其音乐、音效与语音。形式把场景数据所含的每个名称解析为同一版本的资产。除声音的长度、循环与音量以外，它不含这些资产的任何内容。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `episode` | JSON 文档，扩展名 `json` |

资产命名为 `episode/<type>/<key>/<episode>`，其中 `<type>`、`<key>` 与 `<episode>` 取自下文的选择器。

| 键 | 值 |
|---|---|
| `format`、`version` | `"episode"` 与本文档格式的版本，1 |
| `story` | 这是哪一话（见下文） |
| `scenario` | [`scenario`](scenario.zh-CN.md) 资产的名称 |
| `characters` | 场景数据中的每个角色，以 `Character2dId` 为键（见下文） |
| `backgrounds` | 背景键 → [`texture`](texture.zh-CN.md) 资产的名称。键为背景名称，场景数据所指的文件不是背景自身时则为 `<名称>/<文件>` |
| `bgm`、`se`、`voices` | cue 名称 → 声音（见下文）。语音以其 `VoiceId` 为键，精确匹配，含末尾空格 |
| `effects` | 效果名称 → 含有该效果的资源包名称 |
| `movies` | 影片名称 → 其资源包中 [`video`](video.zh-CN.md) 资产的名称，按名称顺序排列 |
| `music_videos` | 该话播放的音乐视频 ID，按首次使用的顺序排列 |
| `area_talk` | 区域对话时出现（见下文） |
| `warnings` | 无法解析的内容：`{ "kind": "…", "detail": "…" }` |

每个映射都按键排序。列表保持其在场景数据中首次出现的顺序。

### `story`

| 键 | 值 |
|---|---|
| `type` | `unit`、`event`、`card`、`special` 或 `area` |
| `selector` | `<type>:<key>/<episode>` |
| `story_id` | 章节 ID、活动 ID、卡牌 ID、特别剧情 ID 或区域 ID |
| `episode_no` | 话的编号。卡牌为 1 表示第一部分，2 表示第二部分。区域对话为动作集 ID |
| `episode_id` | 表中该行的 ID |
| `title` | 标题，区域对话为空 |
| `scenario_id` | 表中该行所指的场景数据 |

`<key>` 为章节的 `assetbundleName`、活动 ID、卡牌 ID、特别剧情 ID 或区域 ID。`<episode>` 为话的编号，卡牌为 `first` 或 `second`（没有分段的卡牌话为行 ID），区域对话为动作集 ID。

### 角色

| 键 | 值 |
|---|---|
| `asset_name` | `character2ds` 行的 `assetName`，没有该行时不存在 |
| `motion_set` | [`live2d-motions`](live2d.zh-CN.md) 资产的名称，无法解析时不存在 |
| `costumes` | 按首次出现的顺序排列；第一个是角色初始所穿的服装 |
| `area_costumes` | 仅区域对话，其他情况不存在（见下文） |

一份服装为 `{ "costume_type": "…", "model": "…", "motions": { … } }`。`model` 为 [`live2d-model`](live2d.zh-CN.md) 资产的名称，该版本没有时为不存在。`motions` 把该话为该角色所用的每个动作名称与表情名称映射到 `{ "asset": "…", "role": "…" }`，即资产以及持有它的成员。不在 `motions` 中的名称使该层保持先前的动作。

### 声音

```json
{ "asset": "sound/scenario/bgm/bgm00000/bgm00000", "cue": "bgm00000", "waveforms": [ … ] }
```

`asset` 为 [`audio`](audio.zh-CN.md) 资产的名称，`cue` 为其中的 cue，`waveforms` 与 `arrangement` 为 cue 在其 [cue 表](audio.zh-CN.md#2-内容)中的条目，没有编配的 cue 不存在 `arrangement`。

### 区域对话

`area_talk` 为 `{ "action_set_id": …, "area_id": …, "background": "…", "bgm": "…" }`。`background` 是 `backgrounds` 的一个键，区域没有图像、对话在黑色背景上播放时不存在。`bgm` 是 `bgm` 的一个键，无法解析时不存在：场景数据不设音乐，区域的音乐在对话之下播放。

角色在区域对话中穿哪个模型取决于日期，因此文档列出每个候选，由读取方选择。`area_costumes` 为该角色每个 `costume2ds` 行与其动作集所指的服装组各一项，按表的顺序排列：

```json
{ "from": "120100", "to": "122523", "model": "live2d/model/…", "motions": { … } }
```

`from` 与 `to` 为 `MMDDHH`。读取方取第一个其窗口包含该日期的条目：`from` 不晚于 `to` 时为 `from ≤ now ≤ to`；否则为 `now ≥ from`。该条目若有 `model`，角色的每套服装都用它及其 `motions`。若没有 `model`，或没有条目匹配，角色保持场景数据中的服装。

### 警告

| 种类 | 含义 |
|---|---|
| `asset_missing` | 该版本没有某个引用所需的资产 |
| `cue_not_found` | 所搜索的 cue sheet 都没有该 cue |
| `model_missing` | 该版本没有某套服装的模型 |
| `motion_set_unresolved` | 角色的动作组未知或不在该版本中 |
| `motion_not_found` | 某个动作名称或表情名称不在角色的任何模型或其动作组中 |
| `table_mismatch` | 场景数据所用的某个 `Character2dId` 在 `character2ds` 中缺失 |
| `unknown_character` | 为不在 `AppearCharacters` 中的角色给出了动作 |
| `unknown_effect_type` | 高于 44 的 `EffectType`，游戏会跳过它 |

## 3. 来源

**数据表：** `unitStories`、`eventStories`、`cardEpisodes`、`cards`、`specialStories`、`character2ds`、`actionSets`、`costume2ds` 与 `areaPlaylists`。每一话一行构建一个资产，每个带有 `scenarioId` 的 `actionSets` 行也是如此。场景数据不在该版本中的行不产出资产。

**读取的资产：** [`scenario`](scenario.zh-CN.md) 资产、`audio` 资产的 `cues` 成员、`live2d-model` 与 `live2d-motions` 资产的文档，以及 [`action-set`](action-set.zh-CN.md) 与 [`scenario-background`](scenario-background.zh-CN.md) 资产。资产按资源包与形式查找。资产的词干为其名称的最后一段去掉 `@` 后缀，比较时不区分大小写。

**场景数据。** 资源包名称总是取自表的 `scenarioId`，绝不取自场景数据内部的 `ScenarioId`。场景数据按其词干在下列资源包中查找，取第一个含有它的资源包：

| 类型 | 资源包 |
|---|---|
| `unit` | `scenario/unitstory/<章节 assetbundleName>` |
| `event` | `event_story/<assetbundleName>/scenario` |
| `card` | `character/member/<卡牌 assetbundleName>`，然后是 `character/member_scenario/<卡牌 assetbundleName>` |
| `special` | `op_02` 为 `scenario/tutorial_story`，`op_02area` 为 `scenario/actionset/group0`；然后是 `scenario/special/<话 assetbundleName>`，然后是 `scenario/special/<剧情 assetbundleName>` |
| `area` | `scenario/actionset/group<动作集 ID / 100>` |

**场景数据中的引用**，按数组顺序遍历：

- 角色来自 `AppearCharacters`。服装来自该处以及 `FirstLayout` 与 `LayoutData` 的 `CostumeType`；`AppearCharacters` 中为空的 `CostumeType` 变为 `<id, 三位数字>_casual`，ID 0 视为 1。动作名称与表情名称来自 `FirstLayout`、`LayoutData` 与 `TalkData[].Motions[]`。
- 背景来自 `FirstBackground`、`scenario/background/` 之下的 `NeedBundleNames` 条目，以及 `EffectType` 7（`StringVal`，设置了 `StringValSub` 时以其为文件）。
- 影片来自 `scenario/movie/` 之下的 `NeedBundleNames` 条目与 `EffectType` 19。
- 语音来自 `TalkData[].Voices[]` 与 `EffectType` 24（`StringValSub`）。
- 音乐来自 `FirstBgm` 与 `SoundData[].Bgm`，音效来自带有 `SeBundleName` 的 `SoundData[].Se`。
- 效果来自 `EffectType` 15 与 16（`StringVal` 为名称，`StringValSub` 为资源包）、带 `hologram`（`scenario/effect/hologram`）与 `IncludeSoundDataBundleNames` 的 `EffectType` 22（最后一段为名称）。没有资源包的 `EffectType` 15 是 `asset_missing` 警告。两个资源包中的同名效果也以第二个资源包为键。
- 音乐视频来自 `EpisodeMusicVideoId` 与 `EffectType` 37（`IntVal`）。

**模型与动作。** 服装的模型是 `live2d/model/<CostumeType>` 的 `live2d-model` 资产。动作组是 `live2d/motion/<assetName>_motion_base` 的 `live2d-motions` 资产，`assetName` 取自角色的 `character2ds` 行，至多为 1 的 ID 视为 1。名称查找不区分大小写：先查模型的 `motion/` 成员，再查动作组的 `motion/` 成员，然后是动作组的 `facial/` 成员。

**背景。** 背景是 `scenario/background/<名称>` 中词干为其文件的 `texture` 资产，或资源包中唯一的贴图。这些资源包的每份 `scenario-background` 资产都会加上其影片，名称去掉 `scenario/movie/`。

**声音。** 按顺序所列的资源包中，第一个其 cue 表具有该 cue 的资产中的第一个资产，精确匹配；在同一个资源包内，词干为 `scenarioId` 的资产优先尝试。

| 声音 | 搜索的资源包 |
|---|---|
| 音乐 | `sound/scenario/bgm/<名称>` |
| 语音 | `sound/<family>/voice/<scenarioId>`，然后对于 `partvoice_` cue，查其角色所在的资源包，然后按名称顺序查每个含有该 cue 的部分语音资源包 |
| 音效 | 已设置时为 `SeBundleName`，然后按 cue 名称查（见下文） |

`<family>` 随场景数据所在的资源包而定：`character/member/` 与 `character/member_scenario/` 之下为 `card_scenario`，`scenario/actionset/` 之下为 `actionset`，`scenario/tutorial_story` 为 `tutorial_scenario`，其他情况为 `scenario`。部分语音的资源包在 `<key>` 以 `v2_` 或 `clb` 开头时为 `sound/scenario/voice/part_voice_<key>`，否则为 `sound/scenario/part_voice/<key>`，其中 `<key>` 为说话角色的 `<assetName>_<unit>`。部分语音资源包即这两个前缀之下的资源包。

| 音效 cue | 资源包 |
|---|---|
| `seNNNNN` | `sound/scenario/se/se_pack00001` 直至编号 528，然后是 `sound/scenario/se/se_pack00001_b` |
| `seNNNNN_b`、`se_walk_*`、`se_run_*` | `sound/scenario/se/se_pack00001_b` |
| `se_event_<名称>_NNN` | `event_story/event_<名称>/scenario_se`，然后是 `event_story/<名称>/scenario_se` |
| 其他 | 活动自身的 `event_story/<assetbundleName>/scenario_se`，然后是两个资源包 |

**区域对话。** 背景为 `area/common_bg` 中的 `bg_area_<区域 ID>`。音乐为区域第一条 `areaPlaylists` 行的资源包 `sound/area_bgm/<assetbundleName>` 与 cue `bgmName`，区域没有行或该行两者都未指明时为 `sound/area_bgm/normal_area` 与 `BGM_AREA_2`。服装组来自 `actionset/group<动作集 ID / 100>` 中词干为 `scriptId` 的 `action-set` 资产；角色的第一个明细决定结果。条目在 `live2dAssetbundleName` 不为空时指明其模型。

**文档以外的结果：**

| 情况 | 结果 |
|---|---|
| 场景数据不是 JSON 对象 | 错误 `malformed` |
| 某个引用无法解析 | 文档中的一条警告 |

## 4. 容器

UTF-8 JSON，无多余空白，键的顺序如上，缺省的值省略。数的写法与 [cue 表](audio.zh-CN.md#4-容器)相同。

## 5. 正确性

第 3 节的规则即游戏客户端寻找剧情的资源时所用的规则。日期留给读取方决定，因此相同的表与资产总是给出相同的字节。持续集成用合成的表与资产构建话，检查每条规则、搜索的顺序以及每条警告。在真实数据上，文档中的每个名称都与其所指的资产核对，每个 cue 都与其资产的 cue 表核对，每个动作都与其资产的成员核对，场景数据的每个引用要么被解析，要么给出警告；转换在单线程与多线程下各运行一次，并比较字节。

## 6. 版本

| 版本 | 变更 |
|---|---|
| 1 | 第一版 |
| 2 | 声音带有其 cue 的编配 |
