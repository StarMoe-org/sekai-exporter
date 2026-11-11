# Roadmap

[中文](roadmap.zh-CN.md)

This page lists every part of the exporter with its state, so that anyone can see what exists and pick up open work. Each row links to the specification that defines the part, when there is one.

## States

| State | Meaning |
|---|---|
| Done | Specified, implemented and covered by tests on synthetic inputs |
| Partial | Implemented for the cases listed; the rest is reported as an error, never guessed |
| Planned | Wanted, not implemented yet |
| Open | Wanted, but its form or its reader is not decided yet; the first step is a specification |

## Working on an item

1. Pick a row that is Planned or Open, and say in an issue that you are working on it, so that two people do not do the same work.
2. If the row has no specification, write one first, as described in [Specifications](spec/README.md). A specification names the runtime the form is for and how its correctness is checked.
3. Follow [Contributing](../CONTRIBUTING.md) for where the code goes, the rules and the checks.
4. The pull request that changes the state of a row also updates this page and its Chinese version.

## Reading

| Part | State | Notes |
|---|---|---|
| Unity asset bundles: the object graph with every cross-file reference resolved | Done | [Object documents](spec/object-documents.md) |
| The client package (APK and XAPK) with version class tables | Done | [Sources](sources.md); the class tables are supplied by the user |
| `MonoBehaviour` fields inside the client package | Done | Read through a script layout table supplied by the user; objects without a layout are reported as `no_layout` |
| Byte-for-byte comparison of object documents between native and WebAssembly builds | Done | |

## Decoding and encoding

| Part | State | Notes |
|---|---|---|
| Textures: Alpha8, R8, RGB24, RGBA32, RGB565, ETC1, ETC2 RGB, ETC2 RGBA8, EAC R, crunched DXT5, ASTC LDR and HDR | Done | Other formats are reported as unsupported |
| HCA and ADX audio, ACB and AWB archives, USM containers | Done | |
| PNG, lossy WebP, OpenEXR, MP3 | Done | [Features](features.md#backends) |

## Pipeline and publishing

| Part | State | Notes |
|---|---|---|
| `export` and `update`: streaming evaluation bundle by bundle into a content-addressed store | Done | [Pipeline](pipeline.md) |
| Selection runs (`--only`) | Done | |
| `index` layout and publication documents | Done | [Publication documents](spec/publication.md), [Asset documents](spec/asset-documents.md) |
| `tree` layout | Done | [Publishing](publishing.md) |
| `storage` view | Done | [Storage view](spec/storage-view.md) |
| S3-compatible stores | Done | [Publishing](publishing.md#s3-compatible-buckets) |
| WebAssembly core, compared with the native build in continuous integration | Done | |
| Lazy evaluator as a library | Done | [Architecture](architecture.md) |
| Path indexes and the `serve` resolver | Done | [Serving](serving.md), [Path index](spec/path-index.md) |
| Resolver for edge workers in front of a bucket | Planned | The `sekai-serve` core does no I/O; a worker supplies the reads |

## General forms

| Form | State | Notes |
|---|---|---|
| `texture` | Done | [Texture](spec/texture.md) |
| `sprite` | Partial | [Sprite](spec/sprite.md); sprites drawn from an atlas are exported when the atlas holds them whole, and trimmed or rotated atlas sprites are reported as unsupported |
| `audio`: CRI cue sheets | Done | [Audio](spec/audio.md) |
| `AudioClip` | Open | No converter; a bundle that contains one is reported as incomplete. No bundle of the current versions is known to hold one, so the first step is to find one |
| `video` | Done | [Video](spec/video.md) |
| `text` and `font` | Done | [Text](spec/text.md), [Font](spec/font.md) |
| `live2d` model and motion set | Done | [Live2D](spec/live2d.md), [Live2D motions](live2d-motion.md) |
| Model files (GLB) as members of other forms | Done | [Model files](spec/model.md) |
| Component documents | Done | [Component documents](spec/component-documents.md) |
| Clip documents | Done | [Clip documents](spec/clip-documents.md) |
| glTF animation from clips | Planned | Clip documents carry the curves today |
| `ugui`: UI layout documents | Open | The feature is defined and converts nothing; a form needs a named reader first |
| `timeline` | Open | The feature is defined and converts nothing |
| `particles` | Open | The feature is defined and converts nothing; MySekai effects decode their particle systems already |

## Story

| Form | State | Notes |
|---|---|---|
| `scenario` | Done | [Scenario](spec/scenario.md) |
| `scenario-background` | Done | [Scenario background](spec/scenario-background.md) |
| `action-set` | Done | [Action set](spec/action-set.md) |
| `episode` | Done | [Episode](spec/episode.md) |
| `virtual-live-script` | Done | [Virtual live script](spec/virtual-live-script.md) |

## Costumes

| Form | State | Notes |
|---|---|---|
| `costume-model` | Done | [Costumes](spec/costume.md#costume-model) |
| `costume-motion` | Done | [Costumes](spec/costume.md#costume-motion) |
| `costume-role`: every part a character of a unit can wear | Done | [Costumes](spec/costume.md#costume-role) |
| `costume`: the looks of each costume | Done | [Costumes](spec/costume.md#costume) |

## MySekai

| Form | State | Notes |
|---|---|---|
| `mysekai-fixtures` | Done | [Fixtures](spec/mysekai-fixtures.md) |
| `characters` | Done | [Characters](spec/characters.md) |
| `client-config` | Done | [Client config](spec/client-config.md) |
| `communication-actions` | Done | [Communication actions](spec/communication-actions.md) |
| `alone-actions` | Done | [Alone actions](spec/alone-actions.md) |
| `talks` | Done | [Talks](spec/talks.md) |
| `fixture-talks` | Done | [Fixture talks](spec/fixture-talks.md) |
| `fixture-reactions` | Done | [Fixture reactions](spec/fixture-reactions.md) |
| `fixture-meta` and `fixture-areas` | Done | [Fixture areas](spec/fixture-areas.md) |
| `emoticon` and `emoticons` | Done | [Emoticons](spec/emoticons.md) |
| `facial-table` and `facial-tables` | Done | [Facial tables](spec/facial-tables.md) |
| `field-camera` | Done | [Field camera](spec/field-camera.md) |
| `fixture-model`: furniture with materials, colliders and navigation data | Done | [Furniture models](spec/fixture-model.md) |
| Furniture attach points: the `attach-points` member of `fixture-model`, and `fixture-attach-points` joining them | Done | [Furniture models](spec/fixture-model.md#26-attach-points), [Fixture attach points](spec/fixture-attach-points.md) |
| Furniture gimmicks | Planned | The moving and interactive parts of furniture |
| Character models | Planned | Built on the model file writer shared with costumes and furniture |
| Motion library | Planned | The character motions a runtime plays, with an index |
| `site-model`: the packages of the places, every prefab a scene, with materials and colliders | Done | [Site models](spec/site-model.md) |
| Site placement and settings: where each field stands, its levels and layouts, from the game data tables | Planned | Joins the `mysekaiSite*` tables |
| Weather and effects | Planned | Builds on the particle decoding of `emoticons` |
| Timelines and actor animations | Planned | Needs character models and furniture |
| UI layouts | Open | Needs `MonoBehaviour` fields from the client package; the form is not decided |
| Sound | Done | Uses the general `audio` form; no form of its own |
