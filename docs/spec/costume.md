# Costumes

[中文](costume.zh-CN.md)

The parts the game's 3D costume screen assembles a character from, and the motions it plays on them. Converters: `costume-model`, `costume-motion`, `costume-role` and `costume` in `sekai-moesekai` (feature `costume`). The last two are table converters: they are built from game data tables and from the assets of the version, after the assets of the bundles.

## 1. Runtime

Moesekai's 3D costume viewer, which shows a character in a costume as the game's costume screen does. For one look it loads a body model, a face model, optionally the face model of another costume for its hair (`Hair` and `Acc` meshes) and an accessory model, merges them into the body by bone name, recolours them with the colour-variation textures of the chosen colour, runs their springs and constraints, and plays the character's costume-screen motions: `motion` and `face` once, then `motion_loop` and `face_loop` repeatedly.

The viewer lists the roles, each a character in a unit, from the `costume-role` assets. A role's asset lists the parts the role can wear and which heads and hairs go together. A `costume` asset gives, for each role and colour of one costume, the parts of the look and the assets that show them.

## 2. Contents

### costume-model

A part of a costume: a body, a face with its hair, or a head accessory, as the prefab the game instantiates.

| Role | Content |
|---|---|
| `model` | The prefab as a [model file](model.md) |
| `components` | Its springs, colliders and constraints as a [component document](component-documents.md). Absent when the prefab has none |
| `texture/<n>` | The `n`th image of the model file, counted from 0, in the form of the `image` member of the [texture form](texture.md). Roles appear in order of `n` |

The component document lists the components of these classes:

| Class | Script |
|---|---|
| `MonoBehaviour` | `ThirdParty.SpringBone/UTJ.SpringManager` |
| `MonoBehaviour` | `Assembly-CSharp/Sekai.SekaiSpringBone` |
| `MonoBehaviour` | `ThirdParty.SpringBone/UTJ.SpringSphereCollider` |
| `MonoBehaviour` | `ThirdParty.SpringBone/UTJ.SpringCapsuleCollider` |
| `AimConstraint`, `ParentConstraint`, `PositionConstraint`, `RotationConstraint` | — |

The colour-variation textures of a part, whose names end in `_C` (colour), `_S` (shadow) and `_H` (value, read as linear), are assets of the [texture form](texture.md) in their own bundles. They replace the textures of the `_MainTex`, `_ShadowTex` and `_ValueTex` properties of the part's materials.

### costume-motion

One motion of the costume screen.

| Role | Content |
|---|---|
| `motion` | The clip as a [clip document](clip-documents.md) |

Its `Transform` curves bind to the nodes of the body model, by the CRC-32 of the node's path as [component documents](component-documents.md) write it. Its blend shape curves bind to the blend shape channels of the face model by the CRC-32 of the channel name; their path is that of the face model's node that has the renderer.

### costume-role

One role: a character in a unit. The asset is named `costume/role/<characterId>-<unit>` and has one member:

| Role | Content |
|---|---|
| `role` | A JSON document, extension `json` |

| Key | Value |
|---|---|
| `format`, `version` | `"costume-role"` and the version of this document format, 1 |
| `role` | The role (below) |
| `parts` | Every part the role can wear (below), ordered by `costume3dId` |
| `notAvailable` | The pairs `[head, hair]` of `costume3dId`s that are not worn together, ordered |
| `defaultHairs` | Head `costume3dId` → the `costume3dId` of the hair worn with it, for the heads among `parts`. Keys are decimal strings, ordered by number |

The role:

| Key | Value |
|---|---|
| `characterId`, `unit` | The character and the unit |
| `figure`, `breastSize` | The character's figure and breast size |
| `skin` | `default`, `shadow1` and `shadow2`: the skin colour codes |
| `defaultBody`, `defaultHead`, `defaultHair` | The `costume3dId`s of the role's default costume |
| `motions` | Clip name → the name of the `costume-motion` asset, for each of `motion`, `face`, `motion_loop` and `face_loop` that the role's motion bundle holds. Absent when no motion bundle is in the version |

A part:

| Key | Value |
|---|---|
| `costume3dId` | The part's ID |
| `slot` | `body`, `hair` or `head` |
| `costumeNumber` | The costume the part belongs to. Absent for the parts of default and unit costumes |
| `colorId` | The colour |
| `colorName`, `name` | The colour's name and the costume's name. Absent when the tables have none |
| `headType` | For a head: `head_only`, `head_and_hair`, `head_all`, `head_front` or `head_back`. Absent otherwise |
| `publishedAt` | When the part was released, in milliseconds since the Unix epoch. Absent when the tables do not say |
| `thumbnail` | The name of the part's thumbnail image. Absent when there is none |
| `model` | What shows the part (below). Absent when the part has no model in the version |
| `missing` | What could not be resolved, as sentences. Absent when nothing is missing |

`model` holds `asset`, the name of a `costume-model` asset: the body for a body, the face with its hair for a hair or for a head of the type `head_and_hair`, and the accessory for any other head. A body or an accessory has `colors` when its colour has textures of its own: `color`, `shadow` and `value` → the names of the `texture` assets whose names end in `_C`, `_S` and `_H`. An accessory has `attach`, the name of the child of the face's `Head` node it hangs from.

### costume

One costume. The asset is named `costume/<costumeNumber>` and has one member:

| Role | Content |
|---|---|
| `costume` | A JSON document, extension `json` |

| Key | Value |
|---|---|
| `format`, `version` | `"costume"` and the version of this document format, 1 |
| `costumeNumber` | The costume |
| `name` | The costume's name. Absent when the tables have none |
| `variants` | One look for each role that has parts of the costume and each colour of it, ordered by character, unit and colour |

A look:

| Key | Value |
|---|---|
| `role` | The name of the role's `costume-role` asset |
| `characterId`, `unit`, `colorId` | Whose look, in which colour |
| `body`, `head`, `hair` | The `costume3dId`s of the parts worn |
| `alternativeHeads` | The `costume3dId`s of the costume's other heads in this colour or in colour 1, ordered |
| `models` | `body`, the body's `model` as a part has it; `face`, the name of the `costume-model` asset of the face worn; and `accessory`, the accessory's `model`, absent when the head is of the type `head_and_hair`. A key is absent when its model is not in the version |
| `missing` | What could not be resolved, as sentences. Absent when nothing is missing. A viewer shows no look that has missing parts |

## 3. Sources

### costume-model

The root is a `GameObject` with a container path, compared without regard to case, of the form:

| Part | Container path |
|---|---|
| Body | `…/live_pv/model/character/body/…/body.prefab` or `…/live_pv/model/characterv2/body/…/body.prefab` |
| Face and hair | `…/live_pv/model/character/face/…/face.prefab` or `…/live_pv/model/characterv2/face/…/face.prefab` |
| Head accessory | `…/live_pv/model/character/head_optional/…/optional.prefab` or `…/live_pv/model/characterv2/head_optional/…/optional.prefab` |

Other prefabs of these bundles, and the imported model files (`.fbx`) the prefabs are made from, are not roots. The model file and the component document are read from the root as their pages describe, and their outcomes other than an asset are those of the two pages.

### costume-motion

The root is an `AnimationClip` with a container path of the form `…/character/motion/costume_setting/…/<name>.anim`, compared without regard to case. Its outcomes other than an asset are those of [clip documents](clip-documents.md).

### costume-role and costume

The tables `character3ds`, `costume3ds`, `costume3dModels`, `costume3dModelDefaultHairs`, `costume3dModelNotAvailablePatterns`, `gameCharacters` and `gameCharacterUnits`, and `costume3dGroups` where it exists. A field whose value is `null` is read as absent. When some row of `costume3ds` has no `characterId`, every row takes the `characterId`, `name` and `publishedAt` it lacks from the row of `costume3dGroups` with its group ID; a row that still has no `characterId` is left out.

| What | Rule |
|---|---|
| Roles | Each pair of `characterId` and `unit` in `character3ds`. The row with the lowest `id` gives the default costume |
| A role's parts | The rows of `costume3dModels` of the role's unit whose `costume3ds` row has the role's character. `partType` gives the slot |
| Costume | A part whose `costume3dGroupId` is 1000 or more belongs to costume `costume3dGroupId / 1000`, rounded down |
| Colours of a costume | The colours of the role's body parts of the costume, or of all its parts of the costume when it has no body among them |
| Look | For each colour, each slot takes the role's part of the costume in that colour, else in colour 1, else the one with the lowest `costume3dId`: the body; the head among the accessories, or among the heads of the type `head_and_hair` when the costume has no accessory; and the hair. A slot without parts of the costume takes the role's default body or head; the hair then is the head's default hair, else the role's default hair |
| `notAvailable`, `defaultHairs` | The rows of `costume3dModelNotAvailablePatterns` and `costume3dModelDefaultHairs` of the role's unit, as head and hair |
| Skin | The `gameCharacterUnits` row of the character and unit, else the character's first row |
| Motion bundle | `character/motion/costume_setting/<id>_00`, with `<id>` the two-digit ID of the role's `gameCharacterUnits` row, then of the character's first row; then `character/motion/costume_setting/ladies` for the figure `ladies` and `mens` otherwise. The first that holds `costume-motion` assets |

A model bundle below is named with the prefix `live_pv/model/characterv2/`, or, when that bundle is not in the version, `live_pv/model/character/`. A bundle is in the version when it holds exactly one asset of the form needed: `costume-model`, or for colours one `texture` asset for each suffix present.

| Model | Bundle name after the prefix |
|---|---|
| Body | `body/<assetbundleName>/<figure>`, with `_<breastSize>` added unless the breast size is `none` |
| Body colours | `color_variation/body/<assetbundleName>/<figure>/<colorAssetbundleName>` |
| Face | `face/<model><suffix>`, where `<model>` is the `assetbundleName` of the hair, or of the head of the type `head_and_hair`. The suffix is `_<headType>` when the head worn is of the type `head_all`, `head_front` or `head_back` and the model name contains `0000` or `0001`, and empty otherwise. When that bundle is not in the version, `0000` in the model name is replaced with `0001` and the name is tried again |
| Accessory | `head_optional/<assetbundleName>`. `attach` is the model's `part`, or else the last segment of `assetbundleName` |
| Accessory colours | `color_variation/head_optional/<assetbundleName>/<colorAssetbundleName>` |

A face with a suffix that is not in the version is shown without the suffix, and the look records that in `missing`. A model or colour bundle that is not in the version, a hair without a model, and a head and hair pair listed in `notAvailable` are recorded in `missing`; no other part takes their place.

**Outcomes other than an asset:** a table that cannot be read fails the run with the code `table_unreadable`, and one that cannot be parsed with `malformed`.

## 4. Container

As the model file, component document, texture form and clip document describe.

## 5. Correctness

On real bundles of every kind of part, model files pass the Khronos glTF Validator without errors, and their nodes, accessors and materials, and the component documents, are compared value by value with an independent export of the same prefabs, with every difference explained. Every curve of every costume-screen motion is evaluated at every frame of the viewer's clock and compared bit for bit with an independent decoding of the same clip. The same bundles give the same bytes with one thread or many, with and without the optimizations of [Performance](../performance.md), and natively or on `wasm32`.

Tests on synthetic tables and assets cover the roles and their defaults, the colours and parts of a look with their fallbacks, faces with and without a suffix, complete heads, accessories with colours and attach points, the motion fallbacks, the filling of compact rows, and the `missing` records. On real tables of every region, the roles, costumes and looks are compared with an independent resolution of the same tables, with every difference explained.

## 6. Version

`costume-model`, `costume-motion`, `costume-role` and `costume`: version 1. A change to any of the above, or to a format they use, changes the version.
