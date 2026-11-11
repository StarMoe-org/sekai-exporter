# Storage view

The `storage` view writes the assets of a version as the path tree served by the asset storage. Module: `storage_view` in `sekai-moesekai` (feature `storage-view`). `sekai-exporter update --storage <dir>` publishes it. View version: 2.

## 1. Runtime

Sites and players that read files of the asset storage by path.

## 2. Contents

| Asset form | Files |
|---|---|
| `texture`, `sprite` | `<file>.png` and `<file>.webp` of the image |
| `audio` | `<cue>.mp3` for each cue; the waveforms of a cue with several are `<cue>_2.mp3` and so on |
| `video` | `<base>.mp4` |
| `scenario` | `<file>.json` |
| `text` | see below |
| `font` | `<file>.<extension>` |

Other members, such as the sprite document, are not part of the view. Under `music/long`, cues whose names end in `_VR` or `_SCREEN` are left out.

## 3. Sources

Every file comes from one member of an asset document. A file sits at `<bundle>/<name>`, where `<bundle>` is the bundle of the root object of its asset and `<file>` is the last component of the asset name. `<base>` is that component without `.usm` and without a numeric segment suffix such as `-001`.

A text asset stored with the extension `.bytes` loses it; under `music/music_score` the file is named `<file>.txt`. A text asset with any other extension keeps it.

Assets that share a name follow the naming rule in [architecture](../architecture.md). When several assets land on one path, the first by asset name keeps it and the others are left out of the view.

## 4. Container

| File | Parameters |
|---|---|
| PNG | The canonical member, unchanged |
| WebP | The pixels of the PNG member, lossy, libwebp quality factor 75 and method 5, `webp` encoder |
| MP3 | The canonical member, unchanged; see [audio](audio.md) |
| MP4 | The canonical member, unchanged; see [video](video.md) |
| JSON and text | The canonical member, unchanged |

A transform is a derivation: its fingerprint is made from the view version, the transform, its encoder version and the hash of the member. A second run reuses every result.

The tree is written in the order given in [publishing](../publishing.md). A fixed path cannot be switched atomically.

## 5. Correctness

A view is compared with a reference tree made from the same input: documents by value, images by decoded pixels, and lossy media by their format contract (codec, container, sample rate, channels, duration within one frame and bit rate tier). `tools/compare-storage` performs the comparison.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
| 2 | Images are the canonical PNG and a WebP made from it; sounds are the canonical MP3 |
