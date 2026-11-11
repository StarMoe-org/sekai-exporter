# Model files

[中文](model.zh-CN.md)

A model file is a prefab hierarchy as a glTF 2.0 binary file: its nodes, meshes, skins, blend shapes and materials, in glTF's coordinates. Asset forms that carry 3D models, such as the [costume model](costume.md#costume-model), use it as a member and add its images as further members. It is not an asset form of its own.

## 1. Runtime

Moesekai's 3D costume viewer, through three.js and its glTF loader. It finds parts of a model by node name, binds the meshes of one part to the bones of another by bone name, drives blend shapes by channel name, reads each material's textures by Unity texture property, and reads the vertex colour as the outline width.

## 2. Contents

The file holds:

- one node for each game object below and including the root, in depth-first order with the root first. A node has the game object's name, the local position, rotation and scale of its `Transform`, and its children in the order the `Transform` stores them. The root is the only node of its scene;
- one glTF mesh for each combination of a mesh, its materials and, when a skin uses it, the joint of each bone slot, drawn by the node of each renderer that draws that combination. The renderers are `MeshRenderer`, with the mesh of the `MeshFilter` of the same game object, and `SkinnedMeshRenderer`. A submesh with triangles is a primitive. A renderer with more materials than submeshes draws its last submesh again with each further material, so each one adds a primitive;
- every vertex channel the mesh stores: `POSITION`, `NORMAL`, `TANGENT`, `COLOR_0`, `TEXCOORD_0` and up, and, for a skinned renderer, `JOINTS_0` and `WEIGHTS_0`;
- one skin for each skinned renderer, as described below. A `SkinnedMeshRenderer` with no bones is not skinned;
- one morph target for each blend shape channel, with the position offsets of the channel's frame and, when the mesh has normals and tangents, their offsets, as sparse accessors. An offset the frame does not store (`hasNormals` or `hasTangents` false) is zero;
- one material for each material the renderers use, and one texture, one sampler and one image for each `Texture2D` the materials use.

A form may write several prefabs into one file, such as the scenes of a place. Each prefab is then a scene of its own, named after its root's game object, in the order the form gives, and the form chooses the scene shown first (`scene`). The nodes of each prefab follow those of the prefabs before it, and its renderers bind to bones of the same prefab only. Meshes, materials, textures and images are shared between the scenes. A file of one prefab has one scene without a name.

A node whose game object is inactive has `extras.active` set to `false`. A node whose renderer is disabled has `extras.enabled` set to `false`. A skin whose `m_RootBone` is a node of the model has that node in `extras.rootBone`. A renderer whose mesh is an object of the engine's own resources (`Library/unity default resources` or `Resources/unity_builtin_extra`), such as its quad, which no bundle holds, draws no mesh in the file, and its node has `extras.engineMesh`: `{ "file": <that file>, "pathId": "<decimal path id>" }`.

A mesh has one bind pose for each bone slot, and its vertices weigh on slots. When the mesh stores a bone name hash for each slot (`m_BoneNameHashes`), the hash is the CRC-32 of the bone's path below the prefab root, written as [component documents](component-documents.md) write node paths. Each slot binds to a node of the model:

1. when the mesh stores a hash for each slot, the first node in depth-first order whose path has the slot's hash;
2. otherwise, or when no node has that hash, the node of the slot's entry of `m_Bones`, when it is set and a node of the model.

A node binds to one slot only: binding by hash comes first, in slot order, then binding by `m_Bones`, also in slot order, and a node that is already bound is passed over. Bones are bound by their hash first because the hash records the bone the mesh was made for: the bind pose of the slot is the inverse of that bone's transform, while the renderer's `m_Bones` may list other bones.

The skin has one joint for each bound slot, in slot order, with the slot's bind pose as its inverse bind matrix. A slot that binds no node has no joint. A skinned renderer with a vertex that weighs on such a slot cannot be posed by the nodes of the model: it draws no mesh in the file, adds no material, and its node has `extras.unbound` set to `true`. `JOINTS_0` gives each vertex the joints of its slots.

The node of a `SkinnedMeshRenderer` whose mesh has blend shapes has `weights`: for each channel, its entry of `m_BlendShapeWeights`, or 0 when the list is shorter, divided by the channel's full weight.

A vertex whose mesh stores bone indices without weights is bound to its first bone index with weight 1. A joint index whose weight is 0 is written as 0.

`mesh.extras` names the morph targets:

| Key | Value |
|---|---|
| `targetNames` | The name of each target: the name of its blend shape channel |
| `fullWeights` | For each target, the channel weight at which its frame applies fully, in Unity's scale |

A morph target's glTF weight is the Unity channel weight divided by its full weight.

`material.extras.unity` holds the material as stored:

| Key | Value |
|---|---|
| `shader` | The name of the shader (`m_ParsedForm.m_Name`), or `null` when the material has none |
| `renderQueue` | `m_CustomRenderQueue` |
| `keywords` | `{ "valid": […], "invalid": […] }`, from `m_ValidKeywords` and `m_InvalidKeywords` |
| `textures` | Each texture property of `m_SavedProperties`, by name: `{ "texture": <index or null>, "scale": [x, y], "offset": [x, y] }`. `texture` is the index of a glTF texture, and is `null` when the property has no `Texture2D` or its image is a high dynamic range image |
| `floats`, `ints`, `colors` | Each saved property by name. A colour is `[r, g, b, a]`. `ints` is empty when the material stores no integer properties |

Properties are in stored order. A name stored more than once keeps its first value.

A number JSON cannot hold is written as the string `"NaN"`, `"Infinity"` or `"-Infinity"`.

`pbrMetallicRoughness.baseColorTexture` is the texture of `_MainTex`, or of `_BaseMap` when `_MainTex` has none. It is a preview for programs that do not read `extras.unity`: `metallicFactor` is 0, `roughnessFactor` is 1, and no other material field is derived from the properties.

A texture's sampler follows its `m_TextureSettings`. `m_WrapU` and `m_WrapV` give `wrapS` and `wrapT`: 0 (repeat) is `REPEAT`, 1 (clamp) `CLAMP_TO_EDGE` and 2 (mirror) `MIRRORED_REPEAT`. `m_FilterMode` gives the filters, with mipmaps when `m_MipCount` is above 1:

| `m_FilterMode` | `magFilter` | `minFilter` without mipmaps | `minFilter` with mipmaps |
|---|---|---|---|
| 0 (point) | `NEAREST` | `NEAREST` | `NEAREST_MIPMAP_NEAREST` |
| 1 (bilinear) | `LINEAR` | `LINEAR` | `LINEAR_MIPMAP_NEAREST` |
| 2 (trilinear) | `LINEAR` | `LINEAR` | `LINEAR_MIPMAP_LINEAR` |

Each image is outside the file. Its URI is `texture/<n>.png`, where `n` counts the images from 0, and names the member of the same asset with the role `texture/<n>`, an image in the form of the `image` member of the [texture form](texture.md).

## 3. Sources

The root, or each root, is a `GameObject` that the asset form selects. The model holds each root's `Transform` subtree, the components of its game objects, and everything the renderers reach: meshes, materials, shaders and textures, which may be in other bundles.

| Case | Result |
|---|---|
| A component of a game object, or an object a renderer reaches, could not be read | The error of that object |
| A mesh has no vertices, is compressed, uses variable bone weights, or has a submesh that is not a triangle list | Error `unsupported` |
| A vertex channel has a format or a component count this page does not list | Error `unsupported` |
| A set of texture coordinates follows a missing set | Error `unsupported` |
| A blend shape channel does not have exactly one frame | Error `unsupported` |
| The mesh of a skinned renderer has no bone indices, or a bone index with a weight does not fit in 16 bits | Error `unsupported` |
| A texture has a wrap mode or a filter mode the table above does not list | Error `unsupported` |
| A texture cannot be decoded | The error of the [texture form](texture.md) |
| The roots of a file are not disjoint: a game object is below two of them, or the scene shown first is not one of the scenes | Error `malformed` |
| A transform, a vertex position, a blend shape position offset or a bind pose is not finite | Error `malformed` |
| The mesh does not store a bone name hash for each bind pose and the numbers of bones and bind poses differ, a vertex weighs on a bone slot beyond the bind poses, or a renderer has more blend shape weights than its mesh has channels | Error `malformed` |
| A blend shape frame moves a vertex twice, a stored offset lies outside its data, or an index lies outside the vertices | Error `malformed` |

The supported vertex channels are positions, normals and texture coordinates as 32-bit floats, tangents as four 32-bit floats, colours as four 32-bit floats or four normalized bytes, bone weights as up to four 32-bit floats, and bone indices as up to four 32-bit unsigned integers.

## 4. Container

GLB: glTF 2.0 with one binary chunk. Each texture names its PNG image directly. A base colour texture with a tiling or an offset uses `KHR_texture_transform`.

Unity's left-handed space maps to glTF's right-handed space by negating X:

| Value | Mapping |
|---|---|
| Position, normal, blend shape offset of a position or normal | `(-x, y, z)` |
| Rotation | `(x, -y, -z, w)` |
| Tangent | `(-x, y, z, -w)` |
| Blend shape offset of a tangent | `(-x, y, z)` |
| Bind pose `M` | `S M S`, with `S = diag(-1, 1, 1, 1)` |
| Triangle | `(a, c, b)` |
| Texture coordinate | `(u, 1 - v)` |
| Base colour tiling `(sx, sy)` and offset `(ox, oy)` | Scale `(sx, sy)`, offset `(ox, 1 - sy - oy)` |

Images are stored top row first, so texture coordinates are flipped instead of images. Every vertex attribute, index list and inverse bind matrix list has a buffer view of its own, starting at a multiple of 4 bytes. Index accessors are 16-bit when every index is below 65535, and 32-bit otherwise. Joints are 16-bit, weights four 32-bit floats, and byte colours normalized. A morph target accessor has no buffer view and holds the offsets that are not zero as sparse values, with 32-bit indices in increasing order; when every offset is zero it has no sparse values. `POSITION` accessors, of vertices and of morph targets, have `min` and `max`. Numbers in the JSON chunk are the stored 32-bit values, written as the shortest decimal that reads back to the same value.

## 5. Correctness

The mapping above is the change of basis from Unity's coordinates to those of glTF, applied to each kind of value by how it transforms. Files are checked with the Khronos glTF Validator, which must report no errors; its notes about skinned nodes that have a transform or a parent are expected, because the hierarchy is kept as the prefab stores it. Continuous integration builds prefabs from synthetic objects and checks the nodes, accessors, skins, morph targets and materials against the values they were built from. On real bundles, the nodes, accessors and materials of each file are compared value by value with an independent export of the same prefabs, and every difference is explained.

## 6. Version

Format version: 2. Version 2 draws no mesh for a skinned renderer that cannot be posed, where version 1 failed the asset. A change to any of the above changes the version of every asset form that uses the format.
