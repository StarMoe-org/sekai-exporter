# Objects and assets: what exporting Unity content means

[中文](objects-and-assets.zh-CN.md)

This project is an **exporter**. It reads the game's Unity files, but reading them is not the goal. The goal is to turn what is inside them into **assets**: files that a program outside the game can open, play, render or run. This document explains the difference, why it decides how the tool is built, what this repository produces by default, and how to add an asset form of your own.

## What an object is

A Unity file (an AssetBundle, or a player data file shipped inside the app) is a container of **serialized files**. A serialized file is a list of **objects**. Each object is one instance of one Unity class, stored as bytes:

| Part | Meaning |
|---|---|
| path ID | a 64-bit number that identifies the object inside its file |
| class | a built-in engine class (`Texture2D`, `Mesh`, `Transform`, `Material`, `AnimationClip`, …) or, for `MonoBehaviour`, a script class written by the game developer |
| type tree | the field layout of the class: names, types, order and alignment. Bundles usually carry it; player data shipped inside an app usually does not |
| data | the field values, in the layout above |

Objects refer to each other with **PPtrs**: a pair `(file index, path ID)`, where the file index points either to the same file or to an entry in the file's list of external files.

Objects are the engine's **storage form**. They are designed for the engine to load fast on one platform, not for anybody else to use:

- One thing you see in the game is spread over many objects, linked by PPtrs and often split across several bundles.
- Payloads stay in the platform's encoding: GPU-compressed textures (ASTC, ETC, BC), compressed mesh channels, animation curves packed into streamed or dense blocks with bindings stored as hashes of transform paths, and audio in middleware formats stored as plain byte blobs.
- Game scripts (`MonoBehaviour`) hold whatever fields the game's code defines. The file says which class an object is and what its fields are called. It does not say what the fields mean.

**Unpacking** turns bytes into objects: open the container, read the serialized files, read each object's fields by its type tree, resolve PPtrs. The result is an object graph. It is exact, and it is still storage.

## What an asset is

An **asset** is a self-contained result that a program outside the engine can use: an image, a model, an audio file, a scenario document, a curve document.

An asset almost never maps to one object. A model is a **closure** of objects: a root prefab and everything reachable from it that the model needs. A sprite is a **region** of a texture, cut out by a rectangle stored in another object. One audio object can hold dozens of **cues**, each an asset of its own. Some assets also need the game's **tables**, which are not in any bundle.

**Exporting** turns object closures into assets. The hard question is what exactly an asset should contain, and the answer does not come from the files.

## An example: one piece of MySekai furniture

Take a single piece of furniture from MySekai, the game's building mode. Inside the bundles it is not one thing. It is spread over:

- a prefab per colour variant: a tree of game objects and transforms;
- meshes, some skinned, with several materials each;
- materials that use the game's own shaders, which carry the game's own parameters and keywords;
- textures in a GPU-compressed format, some in sRGB and some linear;
- animation clips, plus an animator state machine with events that open a door or switch on a lamp;
- particle systems for its effects;
- game script components that mark the points where a character can sit or act, the grid cells the piece occupies, its colliders and the obstacle it leaves in the navigation mesh;
- rows in the game's tables: its name, its category, the sound it makes when placed.

Now export it to glTF, as three different programs need it.

| | A web model viewer | A 3D editor such as Blender | A runtime that reproduces the game |
|---|---|---|---|
| Geometry | one variant, static mesh | all variants, with armature | all variants, with skins exactly as stored |
| Materials | approximated as PBR or unlit, base colour texture only | approximated as editor shader nodes | shader name, every parameter, every keyword, the colour space of every texture, so the runtime's own port of the shader gets the same inputs |
| Animation | maybe one baked clip | baked keyframes on bones | the original curves and the state machine with its events, because the runtime executes the state machine itself |
| Effects | none | none | particle systems in their source form, for the runtime's own particle simulation |
| Interaction | none | none | sit and action points, the placement grid, colliders and the navigation obstacle, in the coordinate space the runtime uses |
| Coordinates | glTF conventions | the editor's import conventions | glTF conventions plus the exact transform back to authored space, because collision is computed there |

All three files can be valid glTF, and they are still three different assets. And for a program to *use* the third one, a glTF file is not enough: the runtime also needs its own shader implementation, a particle simulator, a state machine executor and placement rules. The asset is one half of a contract. The other half is the runtime that consumes it.

The container differs as well. One user wants FBX, another glTF, another USD. Even within one container, the content differs: how the skeleton is laid out, whether animation is baked or kept as curves, whether materials are approximated or carried raw, whether textures are embedded or referenced, whether each variant is a file or one file holds them all.

## Who defines the form of an asset

**The runtime that consumes the asset defines its form.** It decides what has to be inside, in which layout, and what may be left out.

- If that runtime reproduces the game, the game defines the asset, because the runtime needs the game's meaning: what the shader parameters do, how the state machine runs, which value a script treats as its default. The exporter then has to reproduce the game's semantics faithfully.
- If that runtime is a viewer or an editor, the program defines the asset. What it can render, import and play is what the asset should contain, and the game's semantics matter only as far as they change what you see.

Open standards (PNG, WebP, FLAC, glTF, JSON) define the container and its rules, such as glTF's right-handed coordinates and top-down texture rows. They say nothing about which content goes inside.

So an asset form is a decision: *this* closure of objects, interpreted *this* way, written in *this* container, for *this* runtime. Every asset form in this repository is written down as a specification that states what is exported, for which use, and why the result is correct for it.

## Why generic unpackers exist and generic exporters do not

Unpacking has one contract: the engine's file format. Every game built with the same engine version stores objects the same way, and type trees make the files describe their own layout. One implementation serves every game, so good general-purpose unpackers exist.

Exporting has a contract with the consumer's runtime, and there are as many of those as there are programs. A general-purpose tool knows neither your runtime nor the game's meaning. What it can do is:

- export what the engine itself defines, such as a texture to an image or a mesh to a model file. Even there it picks one container, one coordinate convention, one idea of what "a model" is and one approximation of materials, and that choice fits some runtimes and not others;
- dump everything else as fields.

A third kind of tool rebuilds an editor project from the objects. That is a real asset form too: its runtime is the engine's editor.

So unpacking can be general and exporting cannot. Whoever wants assets for a particular runtime has to write the part that fits that runtime, and that part is most of the work.

## What this repository produces by default

By default, the asset forms in this repository serve the **Moesekai** site and the viewers it runs: card and story images, story scenarios, music and voice, videos, 3D costumes, Live2D models and virtual lives. Each default form is the one those viewers consume, and its specification says so.

Other forms are optional and sit behind their own features. One of them produces MySekai assets in the full form a runtime that reproduces the game needs, as in the right-hand column above.

## Why this project is an exporter

The people using this tool want assets. Unpacking is the shared foundation, small and stable: it is done once and checked value by value. Most of the code and most of the correctness work is in exporting: assembling closures, decoding payloads, applying meaning, and writing standard formats in the shape a runtime expects.

The project is built around that:

```
bytes ──unpack──▶ object graph ──convert──▶ assets ──view──▶ output layouts
                  (exact, indexed,         (one converter       (paths and formats
                   cross-file refs)         per asset form)      for one consumer)
                         │                                              │
                         └──────────── catalog: what exists, where it came from ──┘
```

- **Unpack** reads every object exactly and indexes it, with its class, its name and its references, across all files of a version.
- **Convert** is a set of converters. Each one declares which root objects it takes, which game tables it reads and which asset form it produces, and points to the specification of that form.
- **Views** arrange converted assets into the directory layout and file formats one consumer expects. Several views can be produced from the same assets.
- **Object documents** are always available too: every object's fields as JSON, with references resolved. A batch run writes them with every bundle, and a server can generate them on request. Anything without a converter yet is still available there, without loss.
- **The catalog** (SQLite) records versions, bundles, objects, assets and files, so a consumer can browse the output and only changed assets are rebuilt.

## Adding the asset form you want

If your runtime needs a form this repository does not have, contributions are welcome. The architecture is built so that a new form is a small change:

1. **Start from object documents.** Export the bundles you care about and read the object documents to see which objects make up your asset.
2. **Write the specification first**: which runtime it is for, what the asset contains, which objects and tables it comes from, which container it uses, and why the result is correct for that runtime.
3. **Add a converter** as its own module behind its own feature. It receives a root object and a context for fetching referenced objects and game tables, and returns the asset. It does not read files, pick output paths or write to the catalog; the framework does that.
4. **Reuse existing converters.** A form for a new runtime is often an existing converter with different choices for materials, animation or coordinates.
5. **Test with synthetic data.** Tests and fixtures must not contain game data, so build the inputs yourself (see the contributing guide).
6. **If you only need a different layout** for assets that already exist, add a view instead of a converter.

A converter that is not compiled in is reported by name for every asset it would have produced. Turning a feature off never silently drops output.

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the workflow and checks.
