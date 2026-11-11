# Site models

[中文](site-model.zh-CN.md)

The `site-model` form is one package of the MySekai places as a runtime shows them: a field such as the home site or the grasslands, a level of the house, a door of the room, a harvest object, and the other packages under `mysekai/site/`. It holds every prefab of the package with its geometry, its materials and their values, and the colliders and navigation data the runtime reads for it. Converter: `site-model` in `sekai-mysekai`. Related pages: [model files](model.md) for the parts this form shares, [furniture models](fixture-model.md) for the node keys, the collision records and the material table this form takes over, [texture](texture.md) for each image.

## 1. Runtime

A runtime that reproduces the MySekai places loads the model file of a package and:

- shows the scene of the prefab the game loads, finding it by name: the field itself (`home`, `grasslands`, …), the obstacles of each level of the home site (`rank<level>`), or the floor and walls of a room (`mdl_static_floor`, `mdl_static_wall`). The scene shown first is the package's principal prefab (3.1);
- finds the ground of a field by the names of its meshes, and the walk surface, the camera blockers and the footstep surfaces by the collision records of the nodes and the collision meshes beside the file;
- reads the node `extras` for which source objects are inactive, which renderers are disabled and how they cast shadows, and the source identity of each node, which is how other documents name a node;
- reads each material's `extras` to build the material the source author assigned, and loads each texture outside the file by its relative URI.

Where a field stands in the world and which fields a version has come from the game data tables, not from this form. The runtime applies the position of a field itself: a model file is in the frame of its own prefabs.

## 2. Contents

Members:

| Role | Content |
|---|---|
| `model` | Every prefab of the package as a [model file](model.md) of several scenes, with the differences below |
| `texture/<n>` | The `n`th image of the model file, counted from 0, in the form of the `image` member of the [texture form](texture.md). Roles appear in order of `n` |
| `collision` | The collision document of the package (2.3), a UTF-8 JSON document with the extension `json` |

### 2.1 Scenes

The file has one scene for each prefab of the package (3.1), in the order of their container paths compared without regard to case, and named after the prefab's root game object. The default scene (`scene`) is the scene of the principal prefab. Each scene's only root node is its prefab's root, and the nodes of each prefab follow those of the prefabs before it. Meshes, materials, textures and images are shared between the scenes.

Two prefabs of a package can have the same root name; the scenes then share a name, in path order.

### 2.2 Nodes

The nodes, their order, their names, transforms and meshes, and the vertex attributes follow [furniture models](fixture-model.md#22-nodes). The `extras` of a node is the key-value table [model files](model.md) write for a node, with `engineMesh` (2.5) when the node's renderer draws a mesh of the engine, plus these keys of furniture models:

| Key | Written on |
|---|---|
| `sourceObject` | Every node |
| `sourceComponentIds` | Every node |
| `sourceChildOrder` | Every node |
| `shadowCastingMode` | Every node that a renderer draws on |
| `materialed` | Every node that a renderer is on |
| `sourceCollision` | Every node whose game object carries a collider, a `NavMeshModifier`, a `NavMeshAgent` or a `NavMeshObstacle` (2.3) |

The keys that only furniture has are not written: `fixtureViewRoot`, `fixtureCollision`, `roadCellType`, and `fixtureNavLayer` in a collision record.

### 2.3 Collision

A node's `sourceCollision` is the node record of [furniture models](fixture-model.md#23-collision), with these differences:

- it has no `fixtureNavLayer`;
- `activeInHierarchy` is computed from the root of the node's own prefab;
- every `MeshCollider` has the key `engineMesh`, which is `null` unless the collider's mesh is a mesh of the engine (2.5);
- a `MeshCollider` whose mesh is null, or is a mesh of the engine, is recorded as the source stores it: its `mesh` and `geometryId` are `null`. The engine treats such a collider as having no surface, and the runtime does the same.

The `collision` member is the collision document of furniture models: `{ "geometry": [ { "geometryId", "positions", "triangles" }, … ] }`, the collision meshes of every prefab of the package, in ascending order of `geometryId`, each written once. Positions are in the runtime frame and triangles have the winding of section 4.

Which collider is a walk surface, a camera blocker or a footstep surface is not a key: the game tells them apart by the names of their game objects (`…_nav_ground`, `…_nav_cam`, `…_nav_wall`, `…_footse`), and so does the runtime.

### 2.4 Materials

Materials are written as [furniture models](fixture-model.md#24-materials) write them, with the fields of 2.4.1, the `material.extras.unity` table of 2.4.2, and the render state of 2.4.3. The programs a runtime selects from `shader` (`Mysekai/Site/Ground`, `Mysekai/Site/FieldObject`, `Mysekai/Site/Tree`, `Mysekai/Water` and others) are the runtime's; this form writes the same table for every material.

### 2.5 Meshes of the engine

A few game objects draw a mesh that is part of the engine, such as its quad or plane, and that no bundle holds. A renderer whose mesh is such an object has no mesh in the file, and its node has `extras.engineMesh`: `{ "file": "<the engine's resource file>", "pathId": "<decimal path id>" }`, the reference as stored. A runtime that draws it makes the engine's mesh itself. The files of the engine are `Library/unity default resources` and `Resources/unity_builtin_extra`.

## 3. Sources

### 3.1 The input

A package is a bundle under `mysekai/site/`, other than the map of the places (`mysekai/site/sitemap/`). Its prefabs are the game objects its container lists under a path, compared without regard to case, of the form `…/resources/ondemand/mysekai/site/<path>.prefab`; a prefab listed under two paths goes by the first of them in order. The imported model files the prefabs were made from (`.fbx`) are not prefabs, and are not written: the game loads prefabs only.

The principal prefab is `<directory>/<name>.prefab`, where `<directory>` is the deepest directory that holds every prefab of the package and `<name>` is its last part, as `…/site/field/home/home.prefab`; when the package has no such prefab, it is the first prefab in path order, as `mdl_static_floor` of a level of the house. Every prefab is a root of the form, and only the principal prefab produces the asset; the others produce none. The asset is named after the principal prefab's container path.

The model file holds each prefab's `Transform` subtree, the components of its game objects, everything the renderers reach (meshes, materials, shaders, textures, which may be in other bundles), and the collision of the same subtrees (2.3).

### 3.2 Unity classes read

The classes of [furniture models](fixture-model.md#32-unity-classes-read), without `FixtureView` and `RoadView`. `NavMeshData`, the cameras, lights, volumes and the behaviours of the sites (`SiteObjectView`, `SiteEnvironmentConfig` and others) are not read for the form. A collider's mesh may be a mesh of the engine (2.5).

### 3.3 Outcomes

| Case | Result |
|---|---|
| A field the page names is missing, has another type, or does not resolve to an object of the class it names | Error `malformed` |
| A stored number the form writes is not finite | Error `malformed` (as model files) |
| A property the form writes cannot be expressed | Error `malformed` |
| A collision triangle index lies outside its positions, or a collider's mesh has no triangles | Error `malformed` |
| A collider's mesh resolves to an object that is not a `Mesh` | Error `malformed` |
| An object a prefab reaches is in no bundle and is not an object of the engine (2.5) | Error `missing_object` |
| A node's mesh data uses a format, a component count, a topology or a vertex channel that [model files](model.md) does not list | The model file's `unsupported` |
| A texture cannot be decoded | The error of the [texture form](texture.md) |

## 4. Container

The container of [furniture models](fixture-model.md#4-container): GLB with one binary chunk; images outside the file as `texture/<n>.png`, named directly by their textures; `KHR_texture_transform` only for a base colour texture with a tiling or an offset; texture coordinates not flipped; the change of basis of model files, applied to the scenes and the collision alike. The file has the scenes of 2.1.

**Version.** Format version: 1.

## 5. Correctness

A file is correct for the runtime when each of these holds:

1. It has one scene for each prefab of the package, named after its root, in path order; the default scene is the principal prefab's; every node is in exactly one scene.
2. Every node carries `sourceObject` with the ids of the source object it was written from, and no two nodes share the same `(file, gameObjectId, transformId)`.
3. Every `geometryId` a collider names is a geometry of the `collision` member, and every geometry is named by some collider.
4. Every material has the keys of furniture models; every texture index in `textures` names a texture, and every texture names an image that is a member of the asset.
5. The file has no `animations`; no object of the file carries a version field, a coordinate contract, or a unit or UV convention string.

**Checks.**

- Continuous integration builds packages from synthetic objects and checks the scenes, their order, names and default, the node keys, the collision document, the colliders without a mesh and the meshes of the engine.
- On a full version, every package converts; every file passes the Khronos glTF Validator with no errors; the scenes and meshes the runtime looks up by name are present; and the same packages give the same bytes with one thread and with many.
- The furniture models of the same version are byte-identical before and after the node keys, the collision records and the material table were shared with this form.

## 6. Version

Form version: 2. Version 2 follows version 2 of model files. A change to any of the above, or to a format it uses (model files, texture, the collision record, the material table), changes the version of every asset of the form.
