# Architecture

[中文](architecture.zh-CN.md)

This document describes the model the exporter is built on, how results are evaluated, and how the code is divided into crates. Terms are defined in the [glossary](glossary.md).

## The model: a content-addressed derivation graph

Every step of an export is a pure function, and every input and output is identified by its content. Batch export, static publishing, on-demand export, serverless functions and unpacking in a browser are therefore not separate systems. They are the same graph of functions, evaluated in different places with different strategies. Storage is a cache of that graph, and publishing gives names to its results.

The model has four kinds of things:

| Kind | Definition | Example |
|---|---|---|
| **Input** | External bytes, identified by their hash | A downloaded bundle, a player data file from the client package, a row of a game data table |
| **Derivation** | `output = f(inputs…)`, where `f` is a pure function. Its **fingerprint** is `H(name and version of f, parameters, hashes of all inputs)` | `index(bundle)`, `convert(converter, closure, rows)`, `transform(encoder, parameters, blob)` |
| **Binding** | `(region, version, view, path) → one output of one derivation` | `jp/storage/character/member/x.png → transform(png, convert(image, …))` |
| **Materialization** | Writing a set of bindings in some shape to some place | A path tree, an index over the store, or nothing |

There are three functions:

- **index** reads one bundle and produces its object table and reference edges.
- **convert** applies a converter to the closure of a root object and the table rows the converter reads, and produces an asset.
- **transform** applies an encoder with fixed parameters to one blob and produces another, for example a lossless image re-encoded in the format a view requires.

A **view** is a function that produces bindings. A view that only rearranges paths is a path template. A view that needs different bytes produces transform derivations.

### What follows from the model

| Property | Why it holds |
|---|---|
| Incremental runs | A derivation whose fingerprint already has an output is not computed again |
| Sharing across regions | Fingerprints do not contain the region, so identical content in two regions has one fingerprint and one output |
| Self-describing output | Each asset is published with a document that records its function, version, inputs, outputs and the role of each file |
| Rebuildable catalog | Derivation records and bindings carry all of the semantics, so the catalog is an index that can be rebuilt from the store |
| Verifiable mirrors | Anyone who recomputes a derivation from the same inputs obtains the same hash |

## Identity

| Concept | Definition |
|---|---|
| Region and source | `jp`, `en`, `tw`, `kr`, `cn`. The source is `cdn` for downloaded bundles or `client` for assets inside the client package |
| Version | The asset identity and table identity of a region, see [Version identity](sources.md#version-identity); for the `client` source, the application version |
| Bundle | The bundle name from the manifest; for the `client` source, the player data file name |
| Object | `(bundle, path ID)` |
| Class | The native class name. For a `MonoBehaviour`, `assembly/namespace.class`, resolved through `m_Script` and its `MonoScript`, across files where necessary |
| Reference | Each PPtr resolved to `(bundle, path ID)`. External files are resolved through the external file table to the bundle that contains them. References into engine built-in resources are recorded as external references and are not errors |
| Logical path | The container path without the `assets/sekai/assetbundle/resources/<category>/` prefix and without the extension |

Assets are named by their logical path and asset form, inside the directory named by their bundle: a logical path outside that directory is placed under it as `<bundle>/<logical path>`, so a name depends on its own bundle alone. When several assets would receive the same name, assets whose root objects have different classes are ordered by class priority (`Texture2D`, then `Sprite`, then other classes by name), and each asset other than the first receives an `@<class>` suffix. Assets whose root objects have the same class are ordered by the `(bundle, path ID)` of the root, and each asset other than the first receives an `@<path ID>` suffix, with the path ID in decimal as in object documents. When both apply, the class suffix comes first. Assets of a group that have the same path ID in different bundles cannot be told apart by name and are reported as asset errors. The result depends only on the index of the version, never on the order in which work finishes. When any member of such a group changes, the names of the whole group are decided again.

## Evaluation

**Strategies.** *Eager* evaluation computes every binding in advance; the [pipeline](pipeline.md) does so bundle by bundle, with the streaming evaluator. *Lazy* evaluation computes a derivation when a request reaches it and caches the result. *Mixed* evaluation computes frequently requested bindings in advance and the rest on demand.

**Places.** A batch run on one machine, a self-hosted server, a serverless function running the WebAssembly core, a browser running the same core, or another program using the crates as a library.

**Addressing** decides how far laziness can go:

- **By bundle** (`<region>/bundle/<bundle name>/…`): fully lazy. A request can be served by fetching the bundle, unpacking it and returning the object, with no index prepared in advance.
- **By logical path** (`<region>/<view>/<logical path>`): the path must first be mapped to its bundle and objects, which requires the binding index. The index derivations are inexpensive and are computed eagerly; only the expensive convert and transform derivations are left lazy.

The lazy evaluator names and fingerprints the assets of a bundle from the unit of that bundle, as the streaming evaluator does, and writes the same documents and members. A request opens only the window of its bundle and reads object bytes only from the bundles of the closure of the asset. Concurrent requests for one fingerprint compute it once. A request can name an asset or a bundle and object.

On `wasm32` the core produces textures, sprites, audio as FLAC, text, fonts, scenarios, and Live2D models and motion sets, and they are byte-identical to native output. Video needs the `ffmpeg` program and reports `encoder_unavailable`. An asset estimated to need more than 2 GiB of memory reports `too_large`.

## Disciplines the model depends on

1. **Determinism.** Converters and encoders do not read the clock, do not depend on thread order and do not iterate unordered containers. The same input gives the same bytes with one thread or many, natively or on `wasm32`, and with or without the optimizations described in [Performance](performance.md). Continuous integration checks all three.
2. **A pure-Rust core.** The unpacking, decoding and conversion crates compile to `wasm32`. Libraries written in C are confined to the encoding crate and to native builds. When a derivation needs a backend that is not available on the current target, it fails with a named error; it never substitutes another backend, because a different backend produces different bytes.
3. **Declared cost.** Each converter declares a cost class. A converter whose closure is too large for a serverless function is declared batch-only, and a lazy request for it is refused by name.
4. **Input retention.** Lazy evaluation requires inputs to remain available, and the game's distribution servers do not keep old versions. A deployment that needs history keeps either the inputs or the outputs, as configured.
5. **Credentials stay with the deployment.** Accounts and keys are configured by whoever deploys the exporter and never enter the repository. See [Sources](sources.md).

## The converter contract

A converter declares:

- the root objects it accepts, by class and logical path;
- the game data tables it reads;
- the asset form it produces, and the specification in [`docs/spec/`](spec/) that defines that form;
- its version and cost class.

It receives a root object and a context through which it reads referenced objects, payload bytes and table rows. It returns the asset's members, each with its role: decoded content (image surfaces, PCM samples), which the evaluator encodes, or finished bytes. The evaluator writes the asset document. It does not read files, choose output paths or write to the catalog; the evaluator does all of that.

When a converter cannot produce a correct asset, it reports a named error for that asset and the run continues with other assets. An asset whose converter is not compiled into the build is reported with the error `not_compiled`.

## Crates

The workspace has fourteen crates in a flat `crates/` directory. A crate boundary exists where the dependency set differs (C libraries, networking, the catalog database), where a large body of code is optional, or where a stable interface is used by several crates. Everything else is a module behind a feature.

| Layer | Crate | Contents |
|---|---|---|
| 0 | `sekai-unity` | Unpacking: UnityFS, SerializedFile, type trees, layout compilation, block cache. No procedural macros and no C dependencies |
| 0 | `sekai-format` | Serde types of the published documents; consumers can use it directly as a git dependency |
| 1 | `sekai-layouts` | Field layout tables for objects stored without type trees, and their loader |
| 1 | `sekai-codec` | Texture and CRI decoding in pure Rust |
| 1 | `sekai-catalog` | The content-addressed store and the catalog |
| 1 | `sekai-game` | Game-specific inputs: bundle deobfuscation, manifests, downloads, game data tables, region configuration. The `cdn` feature brings TLS through `rustls` and `ring`, which contains C and assembly |
| 2 | `sekai-convert` | The converter interface, writers and the general-purpose converters |
| 2 | `sekai-encode` | Encoders for decoded content. The canonical encoders are pure Rust and always present; every C dependency of an encoder is confined here, one feature each |
| 2 | `sekai-publish` | Layouts, stores, the publication protocol and asset listings |
| 3 | `sekai-pipeline` | The streaming and lazy evaluators, the scheduler and the derivation cache |
| 3 | `sekai-moesekai` | Scenarios, episodes, virtual lives, costumes and the storage view |
| 3 | `sekai-mysekai` | MySekai assets in the full form a game-reproducing runtime needs |
| 4 | `sekai-exporter` | The command-line program |
| 4 | `sekai-wasm` | The core exposed to JavaScript |

Dependencies point only to lower layers. Converters return decoded content (image surfaces, PCM samples) or finished bytes; encoding decoded content is a separate transform derivation, so `sekai-convert` does not depend on `sekai-encode`. Converter families, views and backends are modules enabled by features, as described in [Features](features.md).
