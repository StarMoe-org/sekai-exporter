# sekai-exporter

[![ci](https://github.com/StarMoe-org/sekai-exporter/actions/workflows/ci.yml/badge.svg)](https://github.com/StarMoe-org/sekai-exporter/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.99+](https://img.shields.io/badge/rust-1.99%2B-orange.svg)](https://www.rust-lang.org)

[中文](README.zh-CN.md)

An exporter for Project SEKAI COLORFUL STAGE! feat. Hatsune Miku. It reads the game's Unity asset bundles and client package and turns their contents into assets in open formats: images, audio, video, models, scenarios, curves and documents. Every output is recorded with the objects it came from, so a consumer can find out what any file is.

> [!TIP]
> **Start with [Objects and assets](docs/objects-and-assets.md).** It explains what exporting Unity content means, why this project converts rather than unpacks, and what it produces by default. Reading it first is the quickest way to understand the work in this repository.

## Highlights

- **Whole assets, not loose objects.** It follows the references of an asset across bundles and writes the whole asset in the form a named runtime reads, such as a GLB model with its skeleton, materials and textures, or a Live2D model with its textures, physics and motions.
- **Traceable output.** Every file is recorded with the bundle, objects and converter it came from.
- **Rebuilds only what changed.** Every output is a pure function of content-addressed inputs, so unchanged work is skipped, and identical content is shared across regions.
- **Pure Rust.** Decoders and the canonical encoders are written in Rust without `unsafe` code, and the core builds for WebAssembly with output byte-identical to the native build.
- **Publishes anywhere.** Results go to a content-addressed store with self-describing indexes, or to a plain path tree, locally or to S3-compatible storage.

## Quick start

Build the command-line program:

```bash
cargo build --release -p sekai-exporter
```

Write the object document of every bundle in a directory:

```bash
sekai-exporter objects <bundles> <output>
```

Convert every bundle in a directory to assets:

```bash
sekai-exporter export <bundles> <output>
```

Bring an output directory up to the current version of a region:

```bash
sekai-exporter update <region> <output> --config <file> --secrets <dir>
```

`sekai-exporter` with no arguments lists every command and option. [Sources](docs/sources.md) describes the configuration and credentials of a region, and [Features](docs/features.md) the Cargo features and presets.

## Status

In development. What works today:

| Area | State |
|---|---|
| Unpacking | `objects` writes the [object document](docs/spec/object-documents.md) of every bundle, and its field values are compared with UnityPy in continuous integration |
| Decoding | Alpha8, R8, RGB24, RGBA32, RGB565, ETC1, ETC2 RGB, ETC2 RGBA8, EAC R, crunched DXT5, ASTC LDR and ASTC HDR textures (including texture arrays and 3D textures), HCA and ADX audio, ACB and AWB archives, and USM containers |
| Encoding | PNG, lossy WebP, OpenEXR and MP3, and MP4 through the `ffmpeg` program |
| Converters | Textures, sprites, fonts, text assets, story scenes, action sets, scenario backgrounds, virtual live scripts, CRI movies, CRI cue sheets, Live2D models and motion sets, costume parts and motions, and MySekai furniture, site models, emoticons, character actions and talks. [Features](docs/features.md) lists what each feature converts and which ones convert nothing yet |
| Evaluation and publishing | `export` and `update` evaluate bundles into a local content-addressed store and publish the result as indexes, a path tree or the `storage` view, locally or to S3-compatible storage |
| WebAssembly | The core builds for `wasm32`, and continuous integration compares its object documents and its store with those of the native build byte for byte |

The [Roadmap](docs/roadmap.md) lists the state of every part and the open work.

## Documentation

The documents in [`docs/`](docs/) describe the design that the code implements.

| Document | Contents |
|---|---|
| [Objects and assets](docs/objects-and-assets.md) | **Start here.** What exporting Unity content means and why this project is an exporter |
| [Architecture](docs/architecture.md) | The derivation model, evaluation strategies and the crate structure |
| [Pipeline](docs/pipeline.md) | The stages of a run and the invariant each stage keeps |
| [Performance](docs/performance.md) | How the design avoids work, and where the remaining work runs |
| [Sources](docs/sources.md) | Inputs, regions, version discovery and credentials |
| [Publishing](docs/publishing.md) | Stores, layouts, asset documents and the publication protocol |
| [Serving](docs/serving.md) | The `serve` resolver: path indexes, routes and HTTP responses |
| [Features](docs/features.md) | Crates, Cargo features and presets |
| [Roadmap](docs/roadmap.md) | The state of every part, and how to pick up open work |
| [Live2D motions](docs/live2d-motion.md) | How the game stores Live2D motions, and why their canonical form is the clip's curves |
| [Specifications](docs/spec/README.md) | The specification of every asset form and published document format |
| [Glossary](docs/glossary.md) | Terms used throughout the documents and the code |
| [Third-party sources](docs/third-party.md) | The upstream specification, library or reference behind each component |
| [Contributing](CONTRIBUTING.md) | Where changes go, rules and checks |

## Game data

This repository contains no game data. Inputs are read from the user's own download or client install, and outputs are written outside the repository. Tests use synthetic inputs.

## License

MIT, see [LICENSE](LICENSE).
