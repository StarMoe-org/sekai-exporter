# Costumes

[中文](costume.zh-CN.md)

The parts the game's 3D costume screen assembles a character from, and the motions it plays on them. Converters: `costume-model` and `costume-motion` in `sekai-moesekai` (feature `costume`).

## 1. Runtime

Moesekai's 3D costume viewer, which shows a character in a costume as the game's costume screen does. For one look it loads a body model, a face model, optionally the face model of another costume for its hair (`Hair` and `Acc` meshes) and an accessory model, merges them into the body by bone name, recolours them with the colour-variation textures of the chosen colour, runs their springs and constraints, and plays the character's costume-screen motions: `motion` and `face` once, then `motion_loop` and `face_loop` repeatedly.

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

## 4. Container

As the model file, component document, texture form and clip document describe.

## 5. Correctness

On real bundles of every kind of part, model files pass the Khronos glTF Validator without errors, and their nodes, accessors and materials, and the component documents, are compared value by value with an independent export of the same prefabs, with every difference explained. Every curve of every costume-screen motion is evaluated at every frame of the viewer's clock and compared bit for bit with an independent decoding of the same clip. The same bundles give the same bytes with one thread or many, with and without the optimizations of [Performance](../performance.md), and natively or on `wasm32`.

## 6. Version

`costume-model`: version 1. `costume-motion`: version 1. A change to any of the above, or to a format they use, changes the version.
