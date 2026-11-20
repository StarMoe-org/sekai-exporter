# Virtual live timeline

[中文](virtual-live-timeline.zh-CN.md)

The `virtual-live-timeline` form is a part of a virtual live that the game stores as a timeline — who appears, moves, looks and speaks, with the lights, sound, audience and stage objects — as a JSON document of the timeline and every object it refers to. Converter: `virtual-live-timeline` in `sekai-moesekai` (feature `virtual-live`). Form version: 1.

## 1. Runtime

The readers and players of the [`virtual-live-script`](virtual-live-script.md) form: readers that show or search what is said and done in a virtual live, and players that replay it. A part of a virtual live is stored either as a script, which that form gives, or as a timeline, which this form gives. The form gives the timeline as the game stores it: its tracks, their clips and the objects the clips name. It does not play the timeline, resolve the names it holds into files, or join it with the game data tables that list the virtual lives.

## 2. Contents

| Role | Content |
|---|---|
| `timeline` | The document (below), extension `json` |
| `bytes/<sha256>` | A byte payload larger than 4096 bytes, extension `bin`, as in the [`scenario-prefab`](scenario-prefab.md#2-contents) form |

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

The document is written as the `scenario-prefab` document is, keys and field values alike, with `format` `"virtual-live-timeline"` and version 1: `objects` lists the timeline first, then every object the walk (§3) reaches, in the order it reaches them; a reference to an object of the document is `{"$ref": n}`, `n` its position in `objects`, and a null reference is `null`; scripts are listed by name only. No object has a member of its own, so no object has the `member` key, and every object other than a script keeps all of its stored fields.

What the objects hold is what the engine and the game store; the form does not interpret it:

- The timeline holds its top tracks in `m_Tracks`. A track holds its clips in `m_Clips` and the tracks it groups in `m_Children`, and names the track or timeline that holds it in `m_Parent`.
- Each clip of `m_Clips` gives its start and duration in seconds (`m_Start`, `m_Duration`), its blends and its display name, and names the object that holds its data in `m_Asset`.
- The tracks and clip objects of the master of ceremonies are scripts of the `Sekai.MCTimeline` namespace. The tracks of a character carry its `CharacterId`. A `CharacterTalkClip` holds a line (`Serif`) and the cue of its voice (`CueName`); other clips hold motions and facial expressions, positions and directions, where a character looks, lights, the audience, cheers, sound effects, stage objects and penlight colours.
- The clips name the `VirtualLivePerformerBundleBuildData` of the timeline, which holds the standing positions that clips name by key.

## 3. Sources

**Root objects:** every `MonoBehaviour` (class 114) whose script is `Unity.Timeline/UnityEngine.Timeline.TimelineAsset` and that a container lists under a path that contains `/virtual_live/mc/timeline/`, compared without regard to case. The form does not read game data tables.

**Walk:** as in the [`scenario-prefab`](scenario-prefab.md#3-sources) form. Starting at the timeline, the walk visits each object once, in the order references are first met: the fields of an object are read in stored order, and every reference that resolves to an object of a bundle adds that object at the end, unless it is listed already. The walk does not follow the references of a shader or a script.

**Fields read:** all stored fields of every object the walk reaches, except those of shaders and scripts beyond their names.

**Outcomes other than a document:**

| Case | Result |
|---|---|
| A reference names an object that the version does not hold | The error of resolving it, `missing_object` |

## 4. Container

The document is UTF-8 JSON without insignificant whitespace, with the keys in the order of the `scenario-prefab` document. The walk visits objects in a fixed order, so the same timeline always gives the same bytes. There are no encoding parameters.

## 5. Correctness

The document holds the stored values of the objects without interpretation, so it is complete with respect to what the game reads.

The build of a bundle records, for each container entry, the objects that loading it loads: the slice of the bundle's `m_PreloadTable` that the entry's `preloadIndex` and `preloadSize` select. For a timeline, the walk gives exactly that slice: the timeline, its tracks, their clip objects, the build data and the scripts. Comparing the two sets for every timeline of a version checks the walk against the build, and comparing every value of every document with the fields UnityPy reads from the same objects through their type trees, floating-point numbers as 32-bit floats and references by the path IDs they resolve to, checks the values.

In the tests of `crates/sekai-moesekai` (`tests/virtual_live.rs`), objects built in memory check:

- the root rule: timelines by script under `/virtual_live/mc/timeline/` in any case, and neither timelines elsewhere, other scripts nor other classes;
- the document of a timeline with a group track, a talk track, its clip and the build data the clip names: the order of the walk, references as positions, null references, and the script listed by name;
- that a reference to a missing object gives `missing_object`.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
