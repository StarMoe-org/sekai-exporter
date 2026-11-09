# Live2D

[中文](live2d.zh-CN.md)

Two forms carry the game's Live2D content:

- **`live2d-model`** is one model as the game loads it: its `.moc3` file, its textures, its physics settings and the motions that belong to the model itself. Converter: `live2d-model` in `sekai-convert` (feature `live2d`). Form version: 1.
- **`live2d-motions`** is one set of motions as the game loads it: the body motions and the facial motions a character plays on any of its models. Converter: `live2d-motions` in `sekai-convert` (feature `live2d`). Form version: 1.

## 1. Runtime

The Live2D runtime of Moesekai's Live2D viewer, which reproduces how the game shows a model: it loads the `.moc3` file with Cubism Core, draws it with the model's textures, runs its physics, and plays motions on the game's two motion layers. It binds the curves of a motion to the model's parameters by the hashes Unity binds them by, evaluates them as Unity does, takes each motion's fade time from the game's metadata, and calls the events named `OnLive2DInvokeUserData`. Why motions are kept as the game's clips, and not as Cubism `motion3.json` files, is explained in [Live2D motions](../live2d-motion.md).

The forms keep what the game loads and nothing else. The `.model3.json` file each model bundle also carries is not read by the game, which finds the files of a model through its build data; it is available as a [`text`](text.md) asset.

## 2. Contents

### `live2d-model`

| Role | Content |
|---|---|
| `moc` | The `.moc3` file, extension `moc3` |
| `physics` | The `.physics3.json` file, extension `json`. Absent when the model has no physics |
| `texture/0`, `texture/1`, … | The textures in the order the model's drawables index them, each as in the [`texture`](texture.md) form |
| `motion/<name>` | Each motion of the model itself, as a motion document (below), extension `json` |

The members are in the order of the table. `moc` and `physics` are the stored bytes, unchanged.

### `live2d-motions`

| Role | Content |
|---|---|
| `motion/<name>` | Each body motion, as a motion document, extension `json` |
| `facial/<name>` | Each facial motion, as a motion document, extension `json` |

Body motions come first, then facial motions, each in the order the game lists them. `<name>` is the name by which the game looks the motion up, and is unique within its kind.

### Motion documents

```json
{ "format": "live2d-motion", "version": 1, "fade_in": 0.5, "fade_out": 0.5, "clip": { "format": "clip", … } }
```

| Key | Value |
|---|---|
| `format`, `version` | `"live2d-motion"` and the version of this document format, 1 |
| `fade_in`, `fade_out` | The motion's fade times in seconds, from its metadata object. Absent when the game lists the motion without one |
| `clip` | The motion's clip as a [clip document](clip-documents.md) |

A curve whose `path` is `crc32("Parameters/<id>")` and whose `attribute` is `crc32("Value")` drives the parameter `<id>` of the model. A curve whose `path` is `crc32("Parts/<id>")` and whose `attribute` is `crc32("Opacity")` drives the opacity of part `<id>`. A curve that matches nothing in the model it is played on drives nothing.

### In the publication

Each form has a listing in every publication (see [Publication documents](publication.md)). The `live2d-model` listing names every model and the `live2d-motions` listing every motion set, with their fingerprints and sizes. A reader finds the bundle of a model or a motion set as the bundle of the first entry of `sources` in its asset document, and the files to fetch as the members listed there. The size of a member says how much opening it downloads, and the number of `texture/` members is the number of textures.

Which motion set a story plays with a model is decided by the story, not by either bundle: the story names a model bundle for each character, and the game takes the motions of the character's motion set. That pairing belongs to the [episode](../features.md) assets, which name the model and the motion set of every character they show.

## 3. Sources

### `live2d-model`

**Root objects:** every `MonoBehaviour` (class 114) whose script is `Assembly-CSharp/Sekai.Live2D.Live2DBuildModelData`. A bundle can carry the files of more than one model; the build data names the one the game loads.

**Fields read:** of the root, `Moc3FileName`, `TextureNames`, `PhysicsFileName`, `UserDataFileName`, `AdditionalMotionData` and `CategoryRules`.

The files are found by name. Each name is joined to the directory of the root's container path, and the result is looked up among the container paths of the bundle without regard to case:

- `moc` is the `TextAsset` named by `Moc3FileName`, and its bytes are `m_Script`;
- `texture/<i>` is the `Texture2D` named by entry *i* of `TextureNames`;
- `physics` is the `TextAsset` named by `PhysicsFileName`, when that is not empty.

Each entry of `AdditionalMotionData` gives a `motion/<name>` member, as for a motion set below.

### `live2d-motions`

**Root objects:** every `MonoBehaviour` (class 114) whose script is `Assembly-CSharp/Sekai.Live2D.Live2DBuildMotionData`.

**Fields read:** of the root, `Motions`, `Facials`, `Arms` and `Poses`; of each metadata object, `FadeInTime` and `FadeOutTime`; of each clip, the fields its clip document reads.

Each entry of `Motions` gives a `motion/<name>` member and each entry of `Facials` a `facial/<name>` member. An entry has three fields: `ClipAssetName` is `<name>`, `Clip` refers to the `AnimationClip`, and `Meta` refers to the `MonoBehaviour` whose script is `Assembly-CSharp/Sekai.Live2D.Live2DBuildMotionMetaData` and which gives the fade times.

Neither form reads game data tables.

**Outcomes other than an asset:**

| Case | Result |
|---|---|
| A named file, or an object an entry refers to, is not in the bundle | Error `missing_object` |
| A named file is not of the class above, the `.moc3` file does not begin with `MOC3`, or two entries of one kind have the same name | Error `malformed` |
| `UserDataFileName`, `CategoryRules`, `Arms` or `Poses` is not empty | Error `unsupported` |
| A clip gives no clip document | The error of the [clip document](clip-documents.md#3-sources) |

## 4. Container

`moc` and `physics` are finished bytes, written as they are. Textures are encoded as in the `texture` form. Motion documents are UTF-8 JSON without insignificant whitespace, with keys in the order above; numbers are written as in clip documents.

## 5. Correctness

Each member is either a stored file unchanged, a texture produced by the `texture` form, or a clip document. The model the runtime builds is therefore the model the game builds from the same files, and the values a motion gives are those of the stored clip (see [Live2D motions](../live2d-motion.md) for how this was compared with Cubism's own evaluation).

Continuous integration builds model and motion bundles from synthetic objects and checks the members, their order, the fade times and every case above. On real bundles, the `.moc3` and physics members are compared byte for byte, and the textures pixel by pixel, with the files an independent unpacking of the same bundles gives. The runtime of the viewer then binds every motion of a set to each model of the same character and plays it to its end on the viewer's frame clock, once from the motion documents and once from an independent decoding of the same clips; the parameter values at every frame are equal bit for bit.

## 6. Version

| Form | Version | Change |
|---|---|---|
| `live2d-model` | 1 | First version |
| `live2d-motions` | 1 | First version |
