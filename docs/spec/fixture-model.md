# Furniture models

[中文](fixture-model.zh-CN.md)

The `fixture-model` form is one piece of MySekai furniture as a runtime shows it on a housing site: its geometry, its materials and their values, and the collider and navigation data the site reads for it. Converter: `fixture-model` in `sekai-mysekai`. Related pages: [model files](model.md) for the parts this form shares, [texture](texture.md) for each image.

## 1. Runtime

A runtime that reproduces MySekai housing sites, and an editor of its furniture, load each placed piece from its model file, enter its only scene at the piece's placement pose, and:

- read the scene's node `extras` for which source objects are inactive, which renderers are disabled and how they cast shadows, the source identity of each node (which is how a gimmick, a timeline, a door or a particle effect finds the node it names), and the collision record of each node;
- read each material's `extras` to build the material the source author assigned, and load each texture outside the file by its relative URI;
- read the collision document on the scene root, and resolve every collider, navigation modifier and navigation obstacle from it.

It also reads where characters stand to use the piece, its attach points (2.6). It reads them for every piece of a version at once, from the joined [fixture attach points](fixture-attach-points.md) document, so that it can plan where characters go before it loads any model. Apart from that document, it reads nothing outside the file through this form: the collision document and all renderer state travel inside the GLB, and the only files beside it are its images. Which pieces have a model, and which file each uses, it learns from the asset list of the published set.

## 2. Contents

Members:

| Role | Content |
|---|---|
| `model` | The piece as a [model file](model.md), with the differences below |
| `texture/<n>` | The `n`th image of the model file, counted from 0, in the form of the `image` member of the [texture form](texture.md). Roles appear in order of `n` |
| `attach-points` | The attach points of the piece (2.6), a UTF-8 JSON document with the extension `json` |

A piece has a `fixture-model` asset only when its prefab has a `FixtureView` (3.1).

### 2.1 The scene

The file has one scene, which is also its default scene (`scene` is 0). The scene root is a node with `extras.fixtureViewRoot` set to `true`, and its `Transform` subtree is every node of the file. No node is outside the scene.

The game places a piece by finding the one GameObject with a `FixtureView` and setting that GameObject's position and rotation to the placement pose while keeping its scale. That GameObject is the prefab root, so:

- the root's local `translation` and `rotation` in the GLB are zero and identity, and its local `scale` is the authored scale;
- a `Transform` that is parentless in the prefab is a child of the root's `Transform`.

The root's name is the `m_Name` of that GameObject. Siblings can share a name (several joints are called `root`, action points are called `loc_start013` and so on), so names are not identities: the identity of a node is its `sourceObject` (2.2).

### 2.2 Nodes

The nodes follow [model files](model.md): the root first, then its `Transform` subtree in depth-first order, with each node's children in the order the `Transform` stores them. A node has its game object's name, the local position, rotation and scale of its `Transform`, and, with a renderer, its mesh, its skin and its materials, as model files write them. Vertex attributes are `POSITION`, `NORMAL`, `TANGENT`, `COLOR_0` (four normalized bytes) and `TEXCOORD_0` through `TEXCOORD_3`, in the source's own components; a renderer with more materials than submeshes draws its last submesh again with each further material.

The `extras` of a node is the key-value table [model files](model.md) write for a node, plus these keys. A key whose value the reader would read as the default is not written (`active` only when `false`, `enabled` only when `false`).

| Key | Value | Written on |
|---|---|---|
| `fixtureViewRoot` | `true` | The root: the GameObject whose `FixtureView` is the instance root the game places |
| `sourceObject` | `{ "file": <serialized file name>, "gameObjectId": "<signed 64-bit id>", "transformId": "<signed 64-bit id>" }`. Both ids are decimal strings, because the values are outside what a JSON number holds exactly | Every node |
| `sourceComponentIds` | The decimal string of the source component id of each component on the node's game object, in the order the game object stores them | Every node |
| `sourceChildOrder` | The decimal string of each child's transform id, in the order the `Transform` stores them | Every node |
| `shadowCastingMode` | The `Renderer.shadowCastingMode` of the node's renderer (0 `Off`, 1 `On`, 2 `TwoSided`, 3 `ShadowsOnly`) | Every node that a renderer draws on |
| `materialed` | `true` when the node's renderer draws with a material; `false` when its material slot is empty. Absent when the node has no renderer | Every node that a renderer is on |
| `roadCellType` | The road cell type of the node's road cell, a number | A node whose game object is a cell of a `RoadView` |
| `sourceCollision` | The node's collision record (2.3) | Every node whose game object carries a collider, a `NavMeshModifier`, a `NavMeshAgent` or a `NavMeshObstacle` |
| `fixtureCollision` | The collision document (2.3) | The root only |

`sourceObject` is on every node, including a node that is not the placement root: every other document of the piece that names a node names it by this identity (7), and the reader finds the node by walking the scene and comparing `sourceObject`.

### 2.3 Collision

The collision document is the `fixtureCollision` key of the root node's `extras` (2.2). It holds two kinds of data: the collision meshes the colliders use, and each node's record of its colliders and navigation components.

**Document.** A JSON object:

| Key | Value |
|---|---|
| `geometry` | The collision meshes, in ascending order of `geometryId`. Each is `{ "geometryId", "positions": [[x, y, z], …], "triangles": [[a, b, c], …] }`. Positions are in the runtime frame and triangles have the winding of section 4 |

`geometryId` is `<serialized file name>:<decimal path id of the source mesh>`. A source mesh used by several colliders is written once.

**Node record.** `extras.sourceCollision` on every node that the game's navigation build reads for that node:

| Key | Value |
|---|---|
| `gameObjectId`, `transformId` | The same decimal ids as the node's `sourceObject` |
| `activeSelf` | Whether the game object is active (`m_IsActive`) |
| `activeInHierarchy` | Whether the game object and all its ancestors are active |
| `authoredLayer` | The game object's layer (`m_Layer`) |
| `fixtureNavLayer` | `true` when the node is the `FixtureView` game object or below it |
| `colliders` | The node's colliders, in component order (below) |
| `geometryIds` | The distinct `geometryId` of its colliders, sorted |
| `modifiers` | Its `NavMeshModifier` components: `{ "componentId", "enabled", "overrideArea", "area", "ignoreFromBuild", "affectedAgents" }` |
| `hasNavMeshAgent` | `true` when the node has a `NavMeshAgent` |
| `navMeshAgents` | Its `NavMeshAgent` components: `{ "componentId", "enabled", "agentTypeId", "radius", "height", "baseOffset" }` |
| `navMeshObstacles` | Its `NavMeshObstacle` components: `{ "componentId", "enabled", "shape", "center", "extents", "carve", "onlyStationary", "moveThreshold", "stationaryTime" }`. `shape` is `0` (capsule) or `1` (box); `center` is in the runtime frame and `extents` are lengths |

A collider is `{ "componentId", "kind", "enabled", "isTrigger", "includeLayers", "excludeLayers", "layerOverridePriority", "providesContacts", "convex", "cookingOptions", "mesh", "geometryId", "center", "size", "radius", "height", "direction" }`. `kind` is `MeshCollider`, `BoxCollider`, `SphereCollider` or `CapsuleCollider`. For a `MeshCollider`, `mesh` is `{ "fileId", "pathId" }` of the source mesh, `geometryId` names its geometry, `convex` is `m_Convex` and `cookingOptions` is `m_CookingOptions`. For a box, `center` and `size` are set; for a sphere, `center` and `radius`; for a capsule, `center`, `radius`, `height` and `direction`. Keys that a kind does not use are `null`, and so are `includeLayers`, `excludeLayers` (the layer masks' `m_Bits`), `layerOverridePriority` and `providesContacts` when the engine version of the source does not store them. `center` is in the runtime frame; `size`, `radius` and `height` are lengths.

The reader requires these keys: `enabled` and `isTrigger` on every collider; `center` on every box, sphere and capsule; `size` on a box; `radius` on a sphere and a capsule; `height` and `direction` (`0` x, `1` y, `2` z) on a capsule; `convex` and `geometryId` on a mesh collider; `activeInHierarchy` on every record; `enabled`, `shape`, `center`, `extents` and `carve` on every obstacle; `affectedAgents` on every modifier of a node below the root.

**Format.** The form version (6) covers the format of the document and of the file; neither carries a version field or a convention string. Every collision of the piece is resolved: one the form cannot resolve fails the conversion (3.3).

### 2.4 Materials

A material has the fields and the `material.extras.unity` table of [model files](model.md), with the values below, plus the keys 2.4.2 adds. This form writes one material for each material a renderer of the scene uses.

**2.4.1 Fields.** `name` is the source material's `m_Name`; several materials of a file can share a name. `doubleSided` is `false`. `pbrMetallicRoughness` holds `metallicFactor` 0, `roughnessFactor` 1 and `baseColorTexture` — the texture of `_MainTex`, or of `_BaseMap` when `_MainTex` has none — as model files write it. That description is a preview for programs that do not read the extras; the runtime builds the material from `extras` alone, and its blend mode is the one the program of 2.4.4 selects from `floats` and `keywords.valid`. `alphaMode` and `alphaCutoff` are not written.

**2.4.2 `material.extras.unity`.** The keys of model files, kept:

| Key | Value |
|---|---|
| `shader` | The name of the shader (`m_ParsedForm.m_Name` of the `Shader` that `m_Shader` names), or `null` when the material has none; such a material has no `shaderTextureDefaults` and no `shaderPasses`, and the runtime leaves it on the scene's placeholder |
| `renderQueue` | `m_CustomRenderQueue`. The runtime does not read it; the furniture programs compute their queue themselves |
| `keywords` | `{ "valid": […], "invalid": […] }`, from `m_ValidKeywords` and `m_InvalidKeywords`, in stored order |
| `floats`, `ints`, `colors` | Each saved property of `m_SavedProperties` by name, as model files write them, with no property singled out |
| `textures` | Every texture property of `m_SavedProperties`, by name: `{ "texture": <index or null>, "scale": [x, y], "offset": [x, y] }`, with the scale and offset as stored. `texture` is the index of a glTF texture, and is `null` when the property holds no `Texture2D` or its image is not written by this form. As in model files, a property that holds no texture is listed too; the runtime reads the scale and offset of every property |

The keys this form adds to the same table:

| Key | Value |
|---|---|
| `shaderTextureDefaults` | For every texture property the shader declares, the name of its declared default texture. Absent when the material has no shader |
| `shaderPasses` | One row for each pass of the shader, in subshader and pass order: `{ "name", "lightMode", "subShaderIndex", "passIndex", "subShaderTags", "tags", "renderState" }`. `name` and `tags` are the `m_Name` and `m_Tags` of the pass's state, `lightMode` is the value of its `LightMode` tag (tag names compared without regard to case) or `null`, `subShaderTags` are the `m_Tags` of its subshader, and `renderState` is 2.4.3. Absent when the material has no shader |

`fixtureShaderUsage` is not a key: it is the float `_FixtureShaderUsage` of `floats`, like every other property.

**2.4.3 `renderState`.** Each pass's `renderState` is Unity's serialized state of that pass: `culling`, `zTest`, `zWrite` and `blend` with `colMask`, `srcBlend`, `destBlend`, `srcBlendAlpha`, `destBlendAlpha`, `blendOp` and `blendOpAlpha`. A value is either a named value `{ "name": "<material property>", "default": <the value the shader declares for it: the first component of the property's m_DefValue>, "val": <the value compiled into the pass> }`, without `default` when the shader does not declare the property, or a fixed value `{ "name": "<noninit>", "val": <value> }`. A named value is the material's `floats` entry of that name, or `default` when the material does not store it; a fixed value is `val`. A pass state that is absent for any of these is an error (3.3).

**Source layouts.** A material whose shader reads the multi-layer branch of its program (`_ENABLE_MULTI_UV_SCROLL` in `keywords.valid`) draws from `_LayerTex1`, `_LayerTex2` and `_LayerTex3`. They are texture properties in `textures` like any other, and each has the floats `_UVScrollX` and `_UVScrollY` and the float `_ParallaxStrength`; `_LayerTex1` is always used, and the floats `_UseLayerTex2` and `_UseLayerTex3` switch the others on.

**2.4.4 Programs.** The runtime has a material object for each shader name below, and applies it, in place of the scene's placeholder material, to the primitives whose material has that `shader`:

| `shader` | Runtime material |
|---|---|
| `Mysekai/Fixture/Basic`, `Mysekai/Fixture/Rug`, `Mysekai/Fixture/Fence` | The furniture material. It reads `shader`, `floats`, `colors`, `textures`, `keywords.valid`, `shaderTextureDefaults` and `shaderPasses`, and takes its main texture from `baseColorTexture` |
| The character program of the runtime | The character material, built from the same table and from the textures `textures` retains |
| `Mysekai/Fixture/Canvas`, `Mysekai/Site/Tree`, and the program of a transparent block | The furniture surface material, built from the same table and from `shaderPasses` |
| `Mysekai/Fixture/ShadowMesh` | No material: its meshes are hidden |
| `Mysekai/Fixture/Road` | The road material, which also reads `roadCellType` (2.2) |

A material whose state writes no colour (`_ColorMask` 0 and `_ZWrite` 0 in `floats`) draws nothing, and its meshes are hidden. A material of any other program, or with a `null` shader, keeps the placeholder material of the scene.

Materials whose program is the character program draw with the character's texture conventions, including its UV flip, while the furniture programs flip v themselves (4). The texture coordinates are written unflipped for all of them (4); only the flip each program applies differs.

### 2.5 Animations

This version of the form writes no animations: the GLB has no `animations` array, and the file holds the geometry, the materials and the collision of the piece only. The animation clips a piece's gimmicks and timelines play are to be carried by a later form; this page does not define them yet.

### 2.6 Attach points

An attach point is a place where a character stands to use the piece: it walks to the start of the point, and when the point has an end, it moves from there to the end. The game stores the points of a piece as a list on its `FixtureView`, and finds a point by its position in that list. Each point has a start locator and an end locator, which are game objects of the prefab, and the game reads the world position and rotation of each.

The `attach-points` member is a JSON object:

| Key | Value |
|---|---|
| `root` | `{ "sourceObject", "scale": [x, y, z] }`: the `sourceObject` of the root node (2.2), and the root's local scale as the model file writes it |
| `collider` | The mesh collider the `FixtureView` names as its own: `{ "sourceObject", "componentId" }`, the `sourceObject` of the node whose game object carries it and the decimal string of its component id, as its record in `sourceCollision` gives it (2.3). `null` when the `FixtureView` names none |
| `points` | The attach points, in the order the `FixtureView` stores them. A piece without points has an empty list |

A point is `{ "start", "end" }`. `start` is a locator, and `end` is a locator or `null` when the point has no end locator. A locator is:

| Key | Value |
|---|---|
| `name` | The `m_Name` of the locator's game object |
| `sourceObject` | The `sourceObject` of the locator's node |
| `position` | `[x, y, z]`, the `translation` of the locator's node in the model file |
| `rotation` | `[x, y, z, w]`, the `rotation` of the locator's node in the model file |

A locator is a child of the root (3.3), so its position and rotation are in the frame of the root before the root's scale applies, with the change of basis of section 4. A placed piece puts a locator at the placement pose × `scale` of `root` × the locator's `position` and `rotation`. The locator's own scale does not move it, and is not written.

The member records the points as the game stores them, and nothing derived from them. The game takes the slot of a point from the name of its start locator (the part after `loc_start`, split at `_`, whose second part is the slot number, and 0 when there is none or it is not a number); that law is the runtime's, applied to `name`, and a name it does not match is not an error of this form. The `IsUsing` and `CharacterId` fields of a stored point are the game's state while it runs, and are not written.

## 3. Sources

### 3.1 The input

The root is a `GameObject` with a container path, compared without regard to case, of the form:

| Asset | Container path |
|---|---|
| A piece's model | `…/resources/ondemand/mysekai/fixture/<name>/<name>.prefab` |

The root is the GameObject the container lists under that path; `is_root` selects it. A bundle that lists this path is the piece's model package, and its name is `<prefix><name>` (the path above with `/` spelled `__` and without `.prefab`). The asset's name is the container path. Other objects of the package — the imported model files (`.fbx`) the prefab was made from, and the prefabs of other folders — are not roots, and this form writes no asset for them.

`<name>` is the name of the piece's model package, which its directory and its prefab share.

The prefab root has one `MonoBehaviour` whose script class is `FixtureView`, and only the GameObject of that prefab root is a root. A prefab whose `FixtureView` sits below its root is an error (3.3), because placement and the collision graph are defined on the prefab root.

The model file holds the root's `Transform` subtree, the components of its game objects, everything the renderers reach (meshes, materials, shaders, textures, which may be in other bundles), and the collision graph of the same subtree (2.3).

### 3.2 Unity classes read

| Class | Read for |
|---|---|
| `GameObject` | `m_Name`, `m_IsActive`, `m_Layer`, `m_Component` |
| `Transform`, `RectTransform` | `m_LocalPosition`, `m_LocalRotation`, `m_LocalScale`, `m_Children`, `m_Father`, `m_GameObject` |
| `MeshFilter`, `MeshRenderer`, `SkinnedMeshRenderer` | Mesh, materials, `m_Enabled`, `m_Bones`, `m_RootBone`, `m_ShadowCastingMode` |
| `Mesh` | Vertex data, submeshes and their topology, `m_BindPose` |
| `Material` | `m_Name`, `m_Shader`, `m_CustomRenderQueue`, `m_SavedProperties`, `m_ValidKeywords`, `m_InvalidKeywords` |
| `Shader` | `m_ParsedForm` (name, texture property defaults, passes and their render state) |
| `Texture2D` | Image, sampling settings |
| `MonoBehaviour` with script `FixtureView` | The placement root (`fixtureViewRoot`); `_attachPoints`, whose entries hold `StartLoc` and `EndLoc`, and `_collider` for the attach points (2.6) |
| `MonoBehaviour` with script `RoadView` | Road cell types (`roadCellType`) |
| `MeshCollider`, `BoxCollider`, `SphereCollider`, `CapsuleCollider`, `NavMeshModifier`, `NavMeshAgent`, `NavMeshObstacle` | `sourceCollision` and the collision geometry |

A component can name an object in another bundle of the same package set (a material, a shader, a mesh or a texture). That object is read from the bundle that holds it and written into this file, because the GLB is self-contained.

`FenceView` is not read: the parts a fence piece shows carry their own `active` on their nodes, and the connection law that switches them is the runtime's. Every component this page does not list is not read for the form.

### 3.3 Outcomes

| Case | Result |
|---|---|
| A field the page names is missing, has another type, or does not resolve to an object of the class it names | Error `malformed` |
| A stored number the form writes is not finite (a transform, a vertex, a morph target offset, a bind pose, a collision position, a property in `floats`, `ints` or `colors`) | Error `malformed` (as model files) |
| A property the form writes cannot be expressed (`ints` or `colors` of another shape, a stored name that repeats) | Error `malformed` |
| A collision triangle index lies outside its positions, or a collider's mesh has no triangles | Error `malformed` |
| A material's shader does not resolve | The error of the missing object |
| A collider's mesh does not resolve, or is not a `Mesh` | Error `malformed` |
| A point's `StartLoc` is null, or a locator or `_collider` does not resolve to an object of the prefab of the class it names | Error `malformed` |
| A locator's game object is not a child of the root | Error `unsupported`: its pose would have to be composed through the transforms between them |
| The root's prefab has no `FixtureView` GameObject | Error `malformed` (the form has no asset for such a package) |
| A node's mesh data uses a format, a component count, a topology or a vertex channel that [model files](model.md) does not list | The model file's `unsupported` |
| A texture cannot be decoded | The error of the [texture form](texture.md) |

The errors above follow the error model of the exporter: a conversion stops at the first object that does not give the values the form needs.

## 4. Container

GLB: glTF 2.0 with one binary chunk, holding the scene of 2.1, its nodes, its meshes, materials, textures and samplers. The differences from the container of a [model file](model.md) are:

- **Images are outside the file.** `images[].uri` is `texture/<n>.webp`, and `texture/<n>` is the member of the same asset with that role. Every image is an 8-bit WebP in row order from the top of the image. No image is embedded and no image is PNG.
- **No extension declares the WebP images.** Neither `EXT_texture_webp` nor any other extension is listed in `extensionsUsed` or `extensionsRequired` for the images: `texture.source` names the image directly.
- **`KHR_texture_transform` is used only for a base colour texture** whose slot has a scale other than 1 in either axis or an offset other than 0 in either axis. It is listed in `extensionsUsed` only when written. Its scale is the slot's scale and its offset is `(ox, 1 - sy - oy)` for the slot's scale `(sx, sy)` and offset `(ox, oy)`. Every other texture property keeps its scale and offset in `textures` only, and the runtime applies them itself.
- **Texture coordinates are not flipped.** `TEXCOORD_n` is the source component pair unchanged (u, v), with v counted from the bottom of the image, and rows are written in the image from the top. The runtime flips v when it samples. This is the one difference from the UV rule of model files, and it is why this form does not use the model file's flip.
- **The change of basis is model files' change of basis:** right-handed, Y up, X reflected (`S = diag(-1, 1, 1)`) applied to positions, rotations, normals, tangents, triangles, bind poses and the collision geometry, so that the scene and the collision are in one space.
- Memory layout, accessors, index width, joints and weights are those of model files. `POSITION` accessors have `min` and `max`.
- `asset.generator` names the converter and the form version. The `asset` carries no `extras`.

**Version.** Format version: 1.

## 5. Correctness

A file is correct for the runtime when each of these holds:

1. It has one scene, which is the default scene, and exactly one node carries `fixtureViewRoot`; that node is the scene's only root node, and it is the prefab root.
2. Every node carries `sourceObject` with the ids of the source object it was written from, and no two nodes share the same `(file, gameObjectId, transformId)`.
3. The root carries `fixtureCollision`; every `geometryId` a collider names is a geometry of that document; every node that the navigation build reads has `sourceCollision` with the keys of 2.3.
4. Every material has `shader`, `renderQueue`, `keywords`, `floats`, `ints`, `colors` and `textures`, with every texture property in `textures`; every material whose program 2.4.4 gives a runtime material holds every key that program's material code reads from the extras; every texture index in `textures` names a texture, and every texture names an image that is a member of the asset; every image is WebP and decodes.
5. The file has no `animations`; no object of the file carries a version field, a coordinate contract, or a unit or UV convention string.
6. The `attach-points` member lists every entry of `_attachPoints` in stored order; every `sourceObject` in it is the `sourceObject` of a node of the file; each locator's `position` and `rotation` equal its node's `translation` and `rotation`; `root.scale` equals the root node's `scale`; and `collider`, when set, names a collider in the `sourceCollision` of that node.

**Checks.**

- A file passes the Khronos glTF Validator with no errors.
- Node identities, transforms, extras keys, the material table (extras against the source properties), the collision document and the node records are compared value by value with an independent export of the same bundle, and every difference is explained.
- The WebP images are compared by decoded pixels with the source textures.
- For a sample of pieces, each node and collider the game's own navigation build reads from the source is found again from this file by `sourceObject`, and its shape agrees.
- The attach points of every piece of a version are compared with the `_attachPoints` of its `FixtureView`, read independently from the object documents of the same bundles: the count, the order, the locator names and identities, and the poses.
- The same bundle gives the same bytes with one thread and with many, and on native and on the web target.

## 6. Version

Form version: 2. Version 2 adds the `attach-points` member. A change to any of the above, or to a format it uses (model files, texture, the collision record, the material table, the attach points), changes the version of every asset of the form.

## 7. Naming nodes

Every document of a piece that names a node names it by the node's `sourceObject`: `file`, `gameObjectId` and `transformId`, the same three values the node carries (2.2). Sibling names repeat, so a node's name does not identify it.

Which pieces have a model, and which file each uses, a reader learns from the asset list of the published set: the form produces an asset only for a prefab with a `FixtureView`, so every piece listed with a `fixture-model` asset has one.
