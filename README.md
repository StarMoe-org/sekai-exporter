# sekai-exporter

[中文](README.zh-CN.md)

An exporter for Project SEKAI COLORFUL STAGE! feat. Hatsune Miku. It reads the game's Unity asset bundles and client package and turns their contents into assets in open formats: images, audio, video, models, scenarios, curves and documents. Every output is recorded with the objects it came from, so a consumer can find out what any file is.

## Status

In development. Unpacking works: `sekai-exporter objects <bundles> <output>` writes the [object document](docs/spec/object-documents.md) of every bundle in a directory, and its field values are compared with UnityPy in continuous integration. `sekai-codec` decodes Alpha8, R8, RGB24, RGBA32, RGB565, ETC1, ETC2 RGB, ETC2 RGBA8, EAC R, crunched DXT5, ASTC LDR and ASTC HDR textures (including texture arrays and 3D textures), HCA and ADX audio, ACB and AWB archives and USM containers in pure Rust. `sekai-encode` writes PNG, lossless and lossy WebP, OpenEXR, FLAC, MP3 and AAC. Converters for textures, sprites, fonts, text assets, story scenes, action sets, scenario backgrounds, virtual live scripts, CRI movies, CRI cue sheets, Live2D models and motion sets, costume parts and motions, and MySekai furniture, site models, emoticons, character actions and talks are in place, and [Features](docs/features.md) lists what each feature converts and which ones convert nothing yet. `sekai-exporter export` and `update` evaluate bundles one at a time into a local content-addressed store and publish the result as indexes, a path tree or the `storage` view. The core also builds for WebAssembly, and continuous integration compares its object documents and its store with those of the native build byte for byte. The documents in [`docs/`](docs/) describe the design that the code implements, and the [Roadmap](docs/roadmap.md) lists the state of every part and the open work.

## What it does

- **Unpacks** asset bundles and the client's player data into an exact object graph, with every cross-file reference resolved.
- **Converts** object closures into assets with one converter per asset form. The default forms are the ones the [Moesekai](https://github.com/StarMoe-org/Moesekai) site and its viewers consume. Further forms are optional features.
- **Publishes** the results as a content-addressed store with self-describing indexes, or as a plain path tree, locally or to S3-compatible storage.
- **Rebuilds only what changed.** Every output is the result of a pure function of content-addressed inputs, so unchanged work is skipped, and identical content is shared across regions.

Why the project is built around converting rather than unpacking is explained in [Objects and assets](docs/objects-and-assets.md).

## Documents

| Document | Contents |
|---|---|
| [Objects and assets](docs/objects-and-assets.md) | What exporting Unity content means and why this project is an exporter |
| [Architecture](docs/architecture.md) | The derivation model, evaluation strategies and the crate structure |
| [Pipeline](docs/pipeline.md) | The stages of a run and the invariant each stage keeps |
| [Performance](docs/performance.md) | How the design avoids work, and where the remaining work runs |
| [Sources](docs/sources.md) | Inputs, regions, version discovery and credentials |
| [Publishing](docs/publishing.md) | Stores, layouts, asset documents and the publication protocol |
| [Features](docs/features.md) | Crates, Cargo features and presets |
| [Roadmap](docs/roadmap.md) | The state of every part, and how to pick up open work |
| [Live2D motions](docs/live2d-motion.md) | How the game stores Live2D motions, and why their canonical form is the clip's curves |
| [Object documents](docs/spec/object-documents.md) | The specification of object documents |
| [Texture](docs/spec/texture.md) | The specification of the texture form |
| [Live2D](docs/spec/live2d.md) | The specification of the Live2D model and motion-set forms |
| [Clip documents](docs/spec/clip-documents.md) | The specification of animation clip documents |
| [Model files](docs/spec/model.md) | The specification of model files: a prefab as GLB with external images |
| [Component documents](docs/spec/component-documents.md) | The specification of component documents: the components of a prefab and their references |
| [Costumes](docs/spec/costume.md) | The specification of the costume part and motion forms |
| [Glossary](docs/glossary.md) | Terms used throughout the documents and the code |
| [Third-party sources](docs/third-party.md) | The upstream specification, library or reference behind each component |
| [Contributing](CONTRIBUTING.md) | Where changes go, rules and checks |

## Game data

This repository contains no game data. Inputs are read from the user's own download or client install, and outputs are written outside the repository. Tests use synthetic inputs.

## License

MIT, see [LICENSE](LICENSE).
