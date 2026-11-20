# 虚拟演唱会时间轴

[English](virtual-live-timeline.md)

`virtual-live-timeline` 形式是游戏以时间轴（Timeline）存储的虚拟演唱会的一段——谁登场、移动、注视、说话，以及灯光、声音、观众和舞台物件——写成一个 JSON 文档，内含该时间轴及其引用的每个对象。转换器：`sekai-moesekai` 中的 `virtual-live-timeline`（feature `virtual-live`）。形式版本：1。

## 1. 运行时

[`virtual-live-script`](virtual-live-script.zh-CN.md) 形式的读者与播放器：展示或检索虚拟演唱会中所说所做的读者，以及重放它的播放器。虚拟演唱会的一段要么存为剧本，由那个形式给出；要么存为时间轴，由本形式给出。本形式按游戏存储的样子给出时间轴：它的轨道、轨道的片段以及片段所指的对象。它不播放时间轴，不把其中的名字解析为文件，也不与列出虚拟演唱会的游戏数据表关联。

## 2. 内容

| 角色 | 内容 |
|---|---|
| `timeline` | 文档（见下），扩展名 `json` |
| `bytes/<sha256>` | 大于 4096 字节的字节载荷，扩展名 `bin`，同 [`scenario-prefab`](scenario-prefab.zh-CN.md#2-内容) 形式 |

```json
{ "format": "virtual-live-timeline", "version": 1, "objects": [
  { "file": "CAB-…", "id": "5829576412538874031", "class": "MonoBehaviour", "class_id": 114,
    "script": "Unity.Timeline/UnityEngine.Timeline.TimelineAsset", "name": "mc_1",
    "fields": { "m_GameObject": null, "m_Enabled": 1, "m_Script": { "$ref": 1 }, "m_Name": "mc_1",
                "m_Tracks": [{ "$ref": 2 }], "…": "…" } },
  { "file": "CAB-…", "id": "-657477701292886207", "class": "MonoScript", "class_id": 115,
    "name": "Unity.Timeline/UnityEngine.Timeline.TimelineAsset" },
  { "…": "…", "script": "Unity.Timeline/UnityEngine.Timeline.GroupTrack",
    "fields": { "…": "…", "m_Parent": { "$ref": 0 }, "m_Children": [{ "$ref": 4 }] } },
  { "…": "…", "script": "Assembly-CSharp/Sekai.MCTimeline.MCTimelineCharacterTalkTrack",
    "fields": { "…": "…", "m_Clips": [{ "m_Start": 14.533333333333333, "m_Asset": { "$ref": 6 }, "m_Duration": 2.8, "…": "…" }],
                "CharacterId": 28 } },
  { "…": "…", "script": "Assembly-CSharp/Sekai.MCTimeline.CharacterTalkClip",
    "fields": { "…": "…", "CueName": "voice_mc_…", "Serif": "…", "Character": null } }
] }
```

文档的写法与 `scenario-prefab` 文档相同，键与字段值都一样，`format` 为 `"virtual-live-timeline"`，版本 1：`objects` 先列时间轴，再按遍历（§3）到达的顺序列出它到达的每个对象；指向文档中对象的引用写作 `{"$ref": n}`，`n` 为其在 `objects` 中的位置，空引用写作 `null`；脚本只列名字。没有对象带自己的成员，因此没有对象有 `member` 键，除脚本外的每个对象都保留其全部存储字段。

对象里存的就是引擎与游戏存储的内容，本形式不加解释：

- 时间轴在 `m_Tracks` 中保存顶层轨道。轨道在 `m_Clips` 中保存片段，在 `m_Children` 中保存它所编组的轨道，并在 `m_Parent` 中指向包含它的轨道或时间轴。
- `m_Clips` 的每个片段给出以秒计的开始与时长（`m_Start`、`m_Duration`）、混合与显示名，并在 `m_Asset` 中指向保存片段数据的对象。
- 主持（MC）的轨道与片段对象是 `Sekai.MCTimeline` 命名空间的脚本。某个角色的轨道带有其 `CharacterId`。`CharacterTalkClip` 保存一句台词（`Serif`）及其语音的 cue（`CueName`）；其他片段保存动作与表情、位置与朝向、角色注视之处、灯光、观众、欢呼、音效、舞台物件和应援棒颜色。
- 片段指向时间轴的 `VirtualLivePerformerBundleBuildData`，其中保存片段按键名引用的站位。

## 3. 来源

**根对象：**脚本为 `Unity.Timeline/UnityEngine.Timeline.TimelineAsset`、且容器把它列在包含 `/virtual_live/mc/timeline/` 的路径下（不区分大小写比较）的每个 `MonoBehaviour`（class 114）。本形式不读游戏数据表。

**遍历：**同 [`scenario-prefab`](scenario-prefab.zh-CN.md#3-来源) 形式。从时间轴开始，遍历按引用首次出现的顺序访问每个对象一次：对象的字段按存储顺序读取，每个能解析为某个资源包中对象的引用把该对象加到末尾，已列出的除外。遍历不跟随着色器与脚本的引用。

**读取的字段：**遍历到达的每个对象的全部存储字段；着色器与脚本只读名字。

**文档以外的结果：**

| 情况 | 结果 |
|---|---|
| 引用指向该版本中不存在的对象 | 解析它时的错误 `missing_object` |

## 4. 容器

文档为 UTF-8 JSON，无多余空白，键的顺序同 `scenario-prefab` 文档。遍历按固定顺序访问对象，因此同一时间轴总是给出相同字节。没有编码参数。

## 5. 正确性

文档保存对象的存储值而不加解释，因此相对于游戏所读的内容是完整的。

资源包构建时为每个容器条目记录加载它会加载的对象：条目的 `preloadIndex` 与 `preloadSize` 在资源包 `m_PreloadTable` 中选出的一段。对时间轴而言，遍历给出的恰好就是这一段：时间轴、它的轨道、轨道的片段对象、构建数据和脚本。对一个版本的每条时间轴比较这两个集合，可以用构建结果检查遍历；把每个文档的每个值与 UnityPy 通过类型树从同一对象读出的字段比较（浮点数按 32 位浮点比较，引用按其解析到的 path ID 比较），可以检查取值。

`crates/sekai-moesekai` 的测试（`tests/virtual_live.rs`）用内存中构造的对象检查：

- 根规则：任意大小写的 `/virtual_live/mc/timeline/` 下按脚本认定的时间轴，而不是其他位置的时间轴、其他脚本或其他类；
- 一条带编组轨道、台词轨道、其片段及片段所指构建数据的时间轴的文档：遍历顺序、以位置表示的引用、空引用，以及只列名字的脚本；
- 指向缺失对象的引用给出 `missing_object`。

## 6. 版本

| 版本 | 变更 |
|---|---|
| 1 | 第一版 |
