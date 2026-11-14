# Scenario prefab

[中文](scenario-prefab.zh-CN.md)

The `scenario-prefab` form is an effect or a prefab background of a story with every object it refers to, as a JSON document of their stored fields, with the textures as images and the animation clips also as clip documents. Converter: `scenario-prefab` in `sekai-moesekai` (feature `episode`). Form version: 1.

## 1. Runtime

Story players and exporters that draw the effects and the prefab backgrounds of an episode the way the game does: they instantiate the prefab, place its transforms and rect transforms, draw its images, raw images and sprite renderers with their materials, simulate its particle systems from their modules, run its animators with their controllers and clips, and show the movie view of a background. They read the objects as the engine stores them, so the form keeps their fields and does not interpret them.

An asset starts at the object the game loads. An episode names an effect by its bundle and its name (see [`episode`](episode.md)), and the game loads the prefab of that name from the bundle: each prefab of an effect bundle is an asset. A prefab background is loaded through its build data, a `ScenarioBackgroundBuildData` whose `BackgroundPrefab` and `ForegroundPrefab` are the prefabs the game instantiates: the build data is the asset, so it holds both prefabs and the other fields the build data stores.

## 2. Contents

| Role | Content |
|---|---|
| `prefab` | The document (below), extension `json` |
| `texture/<n>` | The image of the texture that is object `n` of the document, as in the [`texture`](texture.md) form |
| `clip/<n>` | The animation clip that is object `n`, as a [clip document](clip-documents.md), extension `json`, when clip documents represent the clip |
| `bytes/<sha256>` | A byte payload larger than 4096 bytes (below), extension `bin` |

The `prefab` member comes first, then the textures and the clips in object order, then the payloads in the order the document first names them.

```json
{ "format": "scenario-prefab", "version": 1, "objects": [
  { "file": "CAB-…", "id": "-4407209310344673407", "class": "GameObject", "class_id": 1, "name": "kirakira_01",
    "fields": { "m_Component": [{ "component": { "$ref": 1 } }], "m_Name": "kirakira_01", "…": "…" } },
  { "file": "CAB-…", "id": "-3998864310059250282", "class": "Shader", "class_id": 48, "name": "Sekai/Particles/Additive" },
  { "file": "CAB-…", "id": "8811209310344673407", "class": "Texture2D", "class_id": 28, "name": "tex_kirakira", "member": "texture/2",
    "fields": { "m_Width": 256, "…": "…" } }
] }
```

| Key | Value |
|---|---|
| `format`, `version` | `"scenario-prefab"` and the version of this document format, 1 |
| `objects` | The root first, then every object the walk (§3) reaches, in the order it reaches them |

Each object:

| Key | Value |
|---|---|
| `file` | The serialized file that holds the object, such as `CAB-0123456789abcdef` |
| `id` | The path ID, as a decimal string |
| `class`, `class_id` | The class name and the class ID |
| `script` | For an object that refers to a script: `"<assembly>/<namespace>.<class>"`. Absent otherwise |
| `name` | The top-level `m_Name` when it is a nonempty string; for a shader, the name in its parsed form; for a script, its full name as in `script` |
| `member` | For a `Texture2D` that holds an image, `texture/<n>`; for an animation clip that clip documents represent, `clip/<n>`. Absent otherwise |
| `fields` | The stored fields, written as below. Absent for a shader and a script |

Shaders and scripts are listed by name only: a renderer outside the engine finds them by name and does not run the stored program. An animation clip keeps its stored fields, and its clip document is a member when clip documents represent it: a clip they report as `unsupported`, such as one with integer curves or an event with an object parameter, has its fields only. A `Texture2D` keeps its fields but `image data` and `m_StreamData`, whose image is the member.

Field values are written as in [object documents](object-documents.md#24-field-values), with these differences, since the reader sees values rather than their stored types:

| Value | Written as |
|---|---|
| Integer | A number, or a decimal string when it is beyond ±2^53 |
| Floating-point number | The shortest decimal that reads back to the same 32-bit float when the value is one, otherwise the shortest that reads back to the same 64-bit float; `"NaN"`, `"Infinity"` and `"-Infinity"` as strings |
| Reference to an object of the document | `{"$ref": n}`, `n` its position in `objects` |
| Reference to an object of the engine's own resources | `{"$ref": {"external": "<file>", "id": "<path ID>"}}`, such as the built-in quad of `Library/unity default resources` |
| Null reference | `null` |
| Byte payload | `{"$bytes": {"size": n, "sha256": "…", "base64": "…"}}` up to 4096 bytes, and `{"$bytes": {"size": n, "sha256": "…", "member": "bytes/<sha256>"}}` above |
| A top-level `m_StreamData` (`offset`, `size`, `path`) or `m_Resource` (`m_Offset`, `m_Size`, `m_Source`) with a nonempty path | The byte payload of the bytes it names in the resource file of the object's bundle. These are the fields through which textures, meshes, audio clips and video clips store data outside the object |

## 3. Sources

**Root objects:**

- every `MonoBehaviour` (class 114) whose script is `Assembly-CSharp/Sekai.ScenarioBackgroundBuildData`, the build data of a prefab background;
- every `GameObject` (class 1) that the container lists under a path that contains `/scenario/effect/` and ends in `.prefab`, compared without regard to case: the prefabs of the effect bundles `scenario/effect/<name>`.

A background prefab is reached from its build data and is not a root of its own.

**Walk.** Starting at the root, the walk visits each object once, in the order references are first met: the fields of an object are read in stored order, and every reference that resolves to an object of a bundle adds that object at the end, unless it is listed already. A reference to an object of the engine's own resources adds nothing. The walk does not follow the references of a shader or a script, which are listed by name.

**Fields read:** all stored fields of every object the walk reaches, except those of shaders and scripts beyond their names.

**Outcomes other than a document:**

| Case | Result |
|---|---|
| A reference names an object that the version does not hold | The error of resolving it, `missing_object` |
| A texture cannot be decoded, or a clip is malformed | That error, as in the `texture` form and in clip documents |
| An `m_StreamData` or `m_Resource` names bytes outside its resource file | Error `malformed` |

## 4. Container

The document is UTF-8 JSON without insignificant whitespace, with the keys in the order above. The walk visits objects in a fixed order, so the same prefab always gives the same bytes. Images are encoded as in the `texture` form.

## 5. Correctness

The document lists the objects the game loads when it instantiates the prefab, with the values it stores, so a reader that interprets them as the engine does draws what the game draws. Textures and clips are those of the `texture` form and of clip documents, which have their own checks.

The build of a bundle records, for each container entry, the objects that loading it loads: the slice of the bundle's `m_PreloadTable` that the entry's `preloadIndex` and `preloadSize` select. The walk gives the same set when it stops at objects of other bundles, which the engine loads with the bundles that hold them, and leaves out engine objects and the references of shaders. Comparing the two sets for every asset of a version checks the walk against the build; the reproduction is to export the `scenario/effect/` bundles with object documents and compare each asset's `objects` with the preload slice of its root.

In the tests of `crates/sekai-moesekai` (`tests/scenario_prefab.rs`), objects built in memory check:

- the root rule: build data by script, effect prefabs by path, and neither background prefabs nor other paths;
- the document of an effect with a transform, a sprite renderer, a material, a texture, an animator, its controller and a clip;
- that a background starts at its build data and reaches its prefab through `BackgroundPrefab`;
- that the walk lists each object once in first-reference order, including a cycle;
- that a clip that clip documents do not represent keeps its fields and its references, without a member;
- that shaders and scripts are listed by name only and their references are not followed;
- that engine references, null references, large and small byte payloads, both kinds of streamed data, integers beyond ±2^53 and the floating-point cases are written as above;
- that the same prefab gives the same bytes;
- that a reference to a missing object gives `missing_object`, and streamed bytes outside the resource file `malformed`.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
