# Features

[中文](features.zh-CN.md)

The exporter is assembled from Cargo features. Each converter family, view, encoding backend and input source is a feature, so a build contains exactly the parts it needs. The crates are described in [Architecture](architecture.md#crates).

## Rules

1. **One feature is one converter family, one view, one backend or one source.** A feature enables a whole module (`#[cfg(feature = "x")] mod x;`) or an optional dependency. Function bodies contain no `#[cfg]`.
2. **Features are additive.** No two features exclude each other. When several backends for the same format are compiled in, the configuration selects one, and the selected backend is part of every fingerprint it affects.
3. **Dependencies between features are declared in `Cargo.toml`.** Enabling a feature enables the features it needs.
4. **A converter that is not compiled in is reported, not skipped.** When a feature whose converters have their root rules is turned off, each asset they would have produced is recorded with the error `not_compiled` and the converter's name. Features whose asset forms have no root rules are listed in [Converter families](#converter-families); they produce nothing, so turning them off removes nothing.
5. **There is no runtime plug-in interface.** Extending the exporter means adding a module and a feature.
6. **Continuous integration builds every feature on its own** (`cargo hack check --each-feature`), as well as the presets with no default features, with default features and with all features. A feature that depends on another without declaring it fails this check.

## Converter families

In `sekai-convert`, general to Unity content:

| Feature | Converts |
|---|---|
| `image` | `Texture2D`, `Sprite` and sprite atlases into images |
| `audio` | CRI ACB/AWB cues into audio files and cue tables |
| `video` | CRI USM files into MP4 with H.264 and AAC, through the `ffmpeg` program with `libx264`, which is not distributed with the exporter |
| `model` | Prefab hierarchies, meshes, skins and materials into glTF |
| `animation` | `AnimationClip` into curve documents, and into glTF animation where the bindings allow it |
| `live2d` | Live2D Cubism models and their motion sets, in the [Live2D forms](spec/live2d.md) |
| `ugui` | UI prefabs into layout documents |
| `text` | `TextAsset` and fonts |
| `timeline` | Timeline assets into timeline documents |
| `particles` | Particle systems into particle documents |

The features `ugui`, `timeline` and `particles` are defined and do not convert anything yet. These three asset forms have no root rules, so a build without them reports nothing with `not_compiled`, as rule 4 describes. `model` reads a prefab into a [model file](spec/model.md) and a [component document](spec/component-documents.md), and `animation` decodes `AnimationClip` into [clip documents](spec/clip-documents.md). The Live2D and costume forms contain them; neither family produces assets of its own or glTF animation yet. `audio` does not convert `AudioClip` yet.

In `sekai-moesekai`, specific to this game, for the default consumer:

| Feature | Converts or produces |
|---|---|
| `scenario` | Scenario objects into scenario documents |
| `episode` | Story episodes: one table row with every asset it refers to resolved |
| `virtual-live` | The [scripts](spec/virtual-live-script.md) of virtual lives |
| `costume` | Costume parts, motions and combinations for 3D viewing, in the [costume forms](spec/costume.md) |

`costume` produces the `costume-model` and `costume-motion` forms, and from the game data tables the `costume-role` form, one per character and unit with every part it can wear, and the `costume` form, the looks of each costume.

The [storage view](publishing.md#the-storage-view) is a view rather than a converter, so it writes no asset. A build without `storage-view` refuses `update --storage`, and no asset is reported with `not_compiled`.

In `sekai-mysekai`: `mysekai`, which produces MySekai forms in the full form a game-reproducing runtime needs: [furniture models](spec/fixture-model.md) and [site models](spec/site-model.md), furniture and site documents, character actions and talks, and the field camera, each with a page among the [specifications](spec/README.md). A build without it reports the roots of its object forms with `not_compiled`.

## Backends

In `sekai-encode`:

| Feature | Format | Library |
|---|---|---|
| (always) | PNG | `png` |
| (always) | Lossless WebP, the canonical image form | `image-webp` |
| (always) | OpenEXR, the canonical HDR image form | `exr` |
| (always) | FLAC, the canonical audio form | `flacenc` |
| `webp` | Lossy WebP | libwebp, statically linked |
| `mp3` | MP3 | LAME, loaded at run time |
| `aac` | AAC audio tracks in MP4 | libavcodec 59 to 63 with the libavutil of the same FFmpeg release (FFmpeg 5.1 to 9.0), loaded at run time on 64-bit platforms |

The encoders of the canonical forms are written in pure Rust, so native and WebAssembly builds produce the same canonical bytes. Backends that need a C library are available only in native builds. The default build includes `mp3` and `aac`, so it needs LAME and libavcodec installed at run time. Each library is loaded when an asset of that format is encoded: an asset whose library is missing fails with the error `encoder_unavailable`, and the message names the library. The licences of these libraries are listed in [Third-party sources](third-party.md).

## Sources and publishing

| Feature | Crate | Enables |
|---|---|---|
| `cdn` | `sekai-game` | The `cp` and `nuverse` sources, with HTTP and TLS |
| `client` | `sekai-game`, `sekai-exporter` | The `apk` source, and the field layout tables (`sekai-layouts`) for assets inside the client package |
| `catalog-db` | `sekai-catalog` | The catalog kept in a redb database |
| `s3` | `sekai-publish` | S3-compatible stores |
| `serve` | `sekai-exporter` | The lazy evaluator behind an HTTP server; defined and not implemented yet |

## Presets

| Preset | Contents |
|---|---|
| `default` | Everything the default consumer needs: `cdn`, `catalog-db`, `image`, `audio`, `video`, `model`, `animation`, `live2d`, `text`, `scenario`, `episode`, `virtual-live`, `costume`, `storage-view`, `webp`, `mp3`, `aac` |
| `preset-mysekai` | `default` and `mysekai` |
| `preset-client` | `default` and `client` |
| `full` | Every feature |
