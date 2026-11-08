# moesekai-exporter

Working name; the final name is not settled yet.

An exporter that turns Project SEKAI asset bundles into open formats
(PNG/WebP, WAV/OGG/MP3, MP4, glTF, JSON) in one pipeline, with one
incremental ledger shared by every output family.

## Layout

| Layer | Role |
|---|---|
| core | bytes to value trees: UnityFS, SerializedFile, TypeTree, PPtr, resS; type layouts are injected, never bundled |
| remap | objects to files, placed by container path |
| codecs | Unity payloads to neutral forms: textures, audio, video, curves, meshes |
| writers | target formats, including the Unity to glTF coordinate conversion |
| capabilities | per-family rules: which object, which fields, where the output goes |
| ledger | bundle CRC, output hash and tool version; drives skipping and deduplication |

## Game data

This repository contains no game data. Inputs are read from the user's own
download or client install; outputs stay outside the repository.

## License

MIT, see [LICENSE](LICENSE).
