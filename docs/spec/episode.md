# Episode

[中文](episode.zh-CN.md)

The `episode` form is everything one story episode needs, as a JSON document that names the assets it uses. Converter: `episode` in `sekai-moesekai` (feature `episode`). Form version: 2.

## 1. Runtime

Story players and exporters that play an episode the way the game does: they load the scene data, show the Live2D models of its characters and play their motions, show its backgrounds and movies, and play its music, sound effects and voices. The form resolves every name the scene data holds into an asset of the same version. It holds no content of those assets besides the lengths, loops and volumes of the sounds.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `episode` | A JSON document, extension `json` |

The asset is named `episode/<type>/<key>/<episode>`, where `<type>`, `<key>` and `<episode>` are those of the selector below.

| Key | Value |
|---|---|
| `format`, `version` | `"episode"` and the version of this document format, 1 |
| `story` | Which episode this is (below) |
| `scenario` | The name of the [`scenario`](scenario.md) asset |
| `characters` | Each character of the scene data, keyed by `Character2dId` (below) |
| `backgrounds` | Background key → name of the [`texture`](texture.md) asset. The key is the background name, or `<name>/<file>` when the scene data names a file other than the background's own |
| `bgm`, `se`, `voices` | Cue name → sound (below). A voice is keyed by its `VoiceId`, matched exactly, trailing spaces included |
| `effects` | Effect name → the name of the bundle that holds the effect |
| `movies` | Movie name → names of the [`video`](video.md) assets of its bundle, in name order |
| `music_videos` | The music video IDs the episode plays, in order of first use |
| `area_talk` | Present for area conversations (below) |
| `warnings` | What could not be resolved: `{ "kind": "…", "detail": "…" }` |

Every map is ordered by key. Lists keep the order of first appearance in the scene data.

### `story`

| Key | Value |
|---|---|
| `type` | `unit`, `event`, `card`, `special` or `area` |
| `selector` | `<type>:<key>/<episode>` |
| `story_id` | Chapter ID, event ID, card ID, special story ID or area ID |
| `episode_no` | The episode number. For a card, 1 for the first part and 2 for the second. For an area conversation, the action set ID |
| `episode_id` | The ID of the table row |
| `title` | The title, empty for an area conversation |
| `scenario_id` | The scene data the table row names |

`<key>` is the `assetbundleName` of a chapter, the event ID, the card ID, the special story ID or the area ID. `<episode>` is the episode number, `first` or `second` for a card (the row ID for a card episode without a part), and the action set ID for an area conversation.

### Characters

| Key | Value |
|---|---|
| `asset_name` | `assetName` of the `character2ds` row, absent when there is none |
| `motion_set` | The name of the [`live2d-motions`](live2d.md) asset, absent when unresolved |
| `costumes` | In order of first appearance; the first is the costume the character starts in |
| `area_costumes` | Area conversations only, absent otherwise (below) |

A costume is `{ "costume_type": "…", "model": "…", "motions": { … } }`. `model` is the name of the [`live2d-model`](live2d.md) asset, absent when it is not in the version. `motions` maps each motion and facial name the episode uses for the character to `{ "asset": "…", "role": "…" }`, the asset and the member that holds it. A name that is not in `motions` keeps the previous motion on that layer.

### Sounds

```json
{ "asset": "sound/scenario/bgm/bgm00000/bgm00000", "cue": "bgm00000", "waveforms": [ … ] }
```

`asset` is the name of the [`audio`](audio.md) asset, `cue` the cue in it, and `waveforms` and `arrangement` the entries of the cue in its [cue table](audio.md#2-contents), `arrangement` absent on a cue without one.

### Area conversations

`area_talk` is `{ "action_set_id": …, "area_id": …, "background": "…", "bgm": "…" }`. `background` is a key of `backgrounds`, absent when the area has no image and the conversation plays on black. `bgm` is a key of `bgm`, absent when unresolved: the scene data sets no music, and the area's music plays under the conversation.

Which model a character wears in an area conversation depends on the date, so the document lists every candidate and the reader chooses. `area_costumes` holds one entry per `costume2ds` row of the character and the costume group its action set names, in table order:

```json
{ "from": "120100", "to": "122523", "model": "live2d/model/…", "motions": { … } }
```

`from` and `to` are `MMDDHH`. The reader takes the first entry whose window holds the date: when `from` is not after `to`, `from ≤ now ≤ to`; otherwise `now ≥ from`. If that entry has a `model`, every costume of the character uses it with the entry's `motions`. If it has none, or no entry matches, the character keeps the costume of the scene data.

### Warnings

| Kind | Meaning |
|---|---|
| `asset_missing` | The version holds no asset that a reference needs |
| `cue_not_found` | No searched cue sheet has the cue |
| `model_missing` | The version holds no model for a costume |
| `motion_set_unresolved` | The character's motion set is unknown or not in the version |
| `motion_not_found` | A motion or facial name is in none of the character's models nor its motion set |
| `table_mismatch` | The scene data uses a `Character2dId` that `character2ds` lacks |
| `unknown_character` | A motion is given for a character that is not in `AppearCharacters` |
| `unknown_effect_type` | An `EffectType` above 44, which the game skips |

## 3. Sources

**Tables:** `unitStories`, `eventStories`, `cardEpisodes`, `cards`, `specialStories`, `character2ds`, `actionSets`, `costume2ds` and `areaPlaylists`. One asset is built per episode row, and per `actionSets` row with a `scenarioId`. A row whose scene data is not in the version gives no asset.

**Assets read:** the [`scenario`](scenario.md) asset, the `cues` member of `audio` assets, the documents of `live2d-model` and `live2d-motions` assets, and [`action-set`](action-set.md) and [`scenario-background`](scenario-background.md) assets. Assets are found by bundle and form. The stem of an asset is the last segment of its name without an `@` suffix, compared without case.

**Scene data.** Bundle names always come from the table's `scenarioId`, never from the `ScenarioId` inside the scene data. It is found by its stem in the first of these bundles that holds it:

| Type | Bundles |
|---|---|
| `unit` | `scenario/unitstory/<chapter assetbundleName>` |
| `event` | `event_story/<assetbundleName>/scenario` |
| `card` | `character/member/<card assetbundleName>`, then `character/member_scenario/<card assetbundleName>` |
| `special` | `scenario/tutorial_story` for `op_02` and `scenario/actionset/group0` for `op_02area`; then `scenario/special/<episode assetbundleName>`, then `scenario/special/<story assetbundleName>` |
| `area` | `scenario/actionset/group<action set ID / 100>` |

**References in the scene data**, walked in array order:

- Characters come from `AppearCharacters`. Costumes come from there and from the `CostumeType` of `FirstLayout` and `LayoutData`; an empty `CostumeType` in `AppearCharacters` becomes `<id, three digits>_casual`, with ID 0 taken as 1. Motion and facial names come from `FirstLayout`, `LayoutData` and `TalkData[].Motions[]`.
- Backgrounds come from `FirstBackground`, `NeedBundleNames` entries under `scenario/background/`, and `EffectType` 7 (`StringVal`, with `StringValSub` as the file when set).
- Movies come from `NeedBundleNames` entries under `scenario/movie/` and `EffectType` 19.
- Voices come from `TalkData[].Voices[]` and `EffectType` 24 (`StringValSub`).
- Music comes from `FirstBgm` and `SoundData[].Bgm`, and sound effects from `SoundData[].Se` with `SeBundleName`.
- Effects come from `EffectType` 15 and 16 (`StringVal` is the name, `StringValSub` the bundle), `EffectType` 22 with `hologram` (`scenario/effect/hologram`) and `IncludeSoundDataBundleNames` (the last segment is the name). `EffectType` 15 without a bundle is an `asset_missing` warning. The same name in two bundles is also keyed by the second bundle.
- Music videos come from `EpisodeMusicVideoId` and `EffectType` 37 (`IntVal`).

**Models and motions.** The model of a costume is the `live2d-model` asset of `live2d/model/<CostumeType>`. The motion set is the `live2d-motions` asset of `live2d/motion/<assetName>_motion_base`, with `assetName` from the `character2ds` row of the character, IDs of at most 1 taken as 1. A name is looked up without case: first the `motion/` members of the model, then the `motion/` and then the `facial/` members of the motion set.

**Backgrounds.** A background is the `texture` asset of `scenario/background/<name>` whose stem is its file, or the only texture of the bundle. Each `scenario-background` asset of those bundles adds its movies, named without `scenario/movie/`.

**Sounds.** The first asset of the first listed bundle whose cue table has the cue, matched exactly; an asset whose stem is the `scenarioId` is tried first within a bundle.

| Sound | Bundles searched |
|---|---|
| Music | `sound/scenario/bgm/<name>` |
| Voice | `sound/<family>/voice/<scenarioId>`, then for a `partvoice_` cue the bundle of its character, then every part voice bundle that holds the cue, in name order |
| Sound effect | `SeBundleName` when set, then by the cue name (below) |

`<family>` follows the bundle of the scene data: `card_scenario` under `character/member/` and `character/member_scenario/`, `actionset` under `scenario/actionset/`, `tutorial_scenario` for `scenario/tutorial_story`, and `scenario` otherwise. The bundle of a part voice is `sound/scenario/voice/part_voice_<key>` when `<key>` starts with `v2_` or `clb`, and `sound/scenario/part_voice/<key>` otherwise, where `<key>` is `<assetName>_<unit>` of the character who speaks. Part voice bundles are those under the two prefixes.

| Sound effect cue | Bundles |
|---|---|
| `seNNNNN` | `sound/scenario/se/se_pack00001` up to number 528, then `sound/scenario/se/se_pack00001_b` |
| `seNNNNN_b`, `se_walk_*`, `se_run_*` | `sound/scenario/se/se_pack00001_b` |
| `se_event_<name>_NNN` | `event_story/event_<name>/scenario_se`, then `event_story/<name>/scenario_se` |
| Any other | The event's own `event_story/<assetbundleName>/scenario_se`, then both packs |

**Area conversations.** The background is `bg_area_<area ID>` in `area/common_bg`. The music is the bundle `sound/area_bgm/<assetbundleName>` and cue `bgmName` of the area's first `areaPlaylists` row, or `sound/area_bgm/normal_area` and `BGM_AREA_2` when the area has no row or the row names neither. The costume groups come from the `action-set` asset of `actionset/group<action set ID / 100>` whose stem is `scriptId`; the first detail of a character decides. An entry names the model of `live2dAssetbundleName`, when that is not empty.

**Outcomes other than a document:**

| Case | Result |
|---|---|
| The scene data is not a JSON object | Error `malformed` |
| A reference cannot be resolved | A warning in the document |

## 4. Container

UTF-8 JSON without insignificant whitespace, with keys in the order above. Absent values are omitted. Numbers are written as in the [cue table](audio.md#4-container).

## 5. Correctness

The rules of section 3 are those the game's client applies to find the resources of a story. Dates are left to the reader, so the same tables and assets always give the same bytes. Continuous integration builds episodes from synthetic tables and assets and checks each rule, the order of searches and each warning. On real data, every name in a document is checked against the assets it names, every cue against the cue table of its asset, every motion against the members of its asset, and every reference of the scene data is resolved or warned about; the conversion is run on one thread and on several and the bytes compared.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
| 2 | Sounds carry the arrangement of their cue |
