# Third-party sources

[中文](third-party.zh-CN.md)

Every component of the exporter rests on an upstream source: a format specification, a library it links, or a reference implementation its output is checked against. This document lists them by component. Each entry states how the source is used:

- **Linked**: the library is a dependency of a crate and is distributed under its own licence.
- **Specification**: the component is written for this project from the description of the format.
- **Reference**: the component is written for this project; its output is checked value by value against the named implementation, and no code is copied from it.
- **Ported**: code is translated from the named project with the permission its licence gives, and the project's notice is kept in the ported files.

An entry is added in the same change that adds the component, and removed in the change that removes it.

## Unity files

| Component | Source | Use | Licence |
|---|---|---|---|
| UnityFS containers, SerializedFile, type trees | [UnityPy](https://github.com/K0lb3/UnityPy) | Reference: object fields are compared value by value | MIT |
| Class layouts and reading order | [AssetStudio](https://github.com/Perfare/AssetStudio), maintained fork [AssetStudioMod](https://github.com/aelurum/AssetStudio) | Reference for the structure of the reader | MIT |
| Engine serialization rules | [UnityCsReference](https://github.com/Unity-Technologies/UnityCsReference) | Read for the semantics of serialized fields; no code is copied | Unity Reference-Only License |
| Common strings of type trees | Unity editor 2022.3.62f2 (`UnityFileSystemApi.dll`) | Copied: a list of engine field and type names that type trees refer to by offset. UnityPy distributes the same table | Not applicable to a list of identifiers |
| LZ4 block decompression | [`lz4_flex`](https://crates.io/crates/lz4_flex) | Linked | MIT |
| LZMA block decompression | [`lzma-rs`](https://crates.io/crates/lzma-rs) | Linked | MIT |
| Engine class names by class ID | [UnityPy](https://github.com/K0lb3/UnityPy) `ClassIDType` | Ported: the table is generated from it, and the notice is kept in the generated file | MIT |
| Engine class layouts in the TPK file format | [Tpk](https://github.com/AssetRipper/Tpk), [UnityPy](https://github.com/K0lb3/UnityPy) | Reference for the file format and for choosing a layout by version; layouts are compared node by node. A table file is supplied by the user and is not distributed | MIT |
| Bundle header deobfuscation used by this game | [sssekai](https://github.com/MetaSekaiLab/sssekai) | Reference: the transform is described there publicly | MIT |

## Textures

| Component | Source | Use | Licence |
|---|---|---|---|
| ASTC decoding, LDR and HDR | Khronos ASTC specification; [astc-encoder](https://github.com/ARM-software/astc-encoder) | Specification; reference: decoded values are compared bit by bit | Apache-2.0 (astc-encoder) |
| Crunched DXT5 decoding | [crunch](https://github.com/Unity-Technologies/crunch) (branch `unity`) | Stream layout translated from `crn_decomp.h`; reference: unpacked blocks are compared | ZLIB |
| BC3 decoding | Khronos Data Format Specification | Specification; DXT blocks are compared with exact arithmetic from its formulas | — |
| ETC1, ETC2 and EAC decoding | Khronos Data Format Specification; [texture2ddecoder](https://github.com/K0lb3/texture2ddecoder) | Specification; reference: decoded pixels are compared byte by byte | MIT (texture2ddecoder) |
| PVRTC 1 decoding, 2 and 4 bits per pixel | [PowerVR SDK](https://github.com/powervr-graphics/Native_SDK) `PVRTDecompress.cpp`; Khronos `IMG_texture_compression_pvrtc` | Ported; reference: decoded pixels are compared byte by byte. The extension gives the RGB formats an alpha of one | MIT (PowerVR SDK) |
| PNG writing and reading (the canonical image form) | [`png`](https://crates.io/crates/png) | Linked | MIT OR Apache-2.0 |
| Lossy WebP writing | [libwebp](https://chromium.googlesource.com/webm/libwebp) through [`webp`](https://crates.io/crates/webp) and [`libwebp-sys`](https://crates.io/crates/libwebp-sys) | Linked statically in native builds | BSD-3-Clause (libwebp), MIT (`libwebp-sys`) |

## Audio and video

| Component | Source | Use | Licence |
|---|---|---|---|
| HCA and ADX decoding | [vgmstream](https://github.com/vgmstream/vgmstream) (clHCA and its ADX decoder) | Ported: the decoders follow their structure and the constants are generated from it, with the notice kept in the generated file; decoded samples are compared sample by sample | ISC |
| CRI ACB tables, AWB archives | [vgmstream](https://github.com/vgmstream/vgmstream) | Reference: results are compared with what it reads and decodes | ISC |
| USM demultiplexing | [WannaCRI](https://github.com/donmai-me/WannaCRI) | Reference: separated streams are compared byte by byte | MIT |
| HCA test fixtures | [VGAudio](https://github.com/Thealexbarney/VGAudio) | Used to encode generated audio; not linked or distributed | MIT |
| OpenEXR writing (the canonical HDR image form) | [`exr`](https://crates.io/crates/exr) | Linked | BSD-3-Clause |
| MP3 writing (the canonical audio form) | [`rusty_mp3`](https://crates.io/crates/rusty_mp3), with the changes listed in [`third_party`](../third_party/README.md), and [`libm`](https://crates.io/crates/libm) | Linked | Apache-2.0; MIT |
| MP3 reading in tests | [`symphonia`](https://crates.io/crates/symphonia) | Used in tests, where written MP3 is decoded and compared with its samples; not linked into the exporter | MPL-2.0 |
| MP4 writing with H.264 and AAC | [FFmpeg](https://ffmpeg.org/) with [x264](https://www.videolan.org/developers/x264.html) | Run as a separate program; not linked or distributed with the exporter | GPL-2.0-or-later |

## Models and documents

| Component | Source | Use | Licence |
|---|---|---|---|
| glTF writing | Khronos glTF 2.0 specification; [glTF Validator](https://github.com/KhronosGroup/glTF-Validator) | Specification; every written file passes the validator without errors | Apache-2.0 (validator) |
| JSON | [`serde`](https://crates.io/crates/serde), [`serde_json`](https://crates.io/crates/serde_json) | Linked | MIT OR Apache-2.0 |

## Live2D

| Component | Source | Use | Licence |
|---|---|---|---|
| Evaluation of `motion3.json` curves in [`compare-live2d-motion`](../tools/compare-live2d-motion/README.md) | [Cubism Web Framework](https://github.com/Live2D/CubismWebFramework) `5-r.5` | Reference: curves are evaluated by its `evaluateCurve`, from a checkout the user provides; not distributed | Live2D Open Software License |
| Mapping of `motion3.json` segments to Unity keyframes | [Cubism SDK for Unity](https://github.com/Live2D/CubismUnityComponents) `5-r.5` (`CubismMotion3Json`) | Read for the mapping described in [Live2D motions](live2d-motion.md); no code is copied | Live2D Open Software License |
| Bundling the framework for the comparison | [`esbuild`](https://www.npmjs.com/package/esbuild) | Installed with the comparison script; not distributed | MIT |

## Inputs, storage and the command line

| Component | Source | Use | Licence |
|---|---|---|---|
| Manifest and interface decryption | [`aes`](https://crates.io/crates/aes), [`cbc`](https://crates.io/crates/cbc) | Linked | MIT OR Apache-2.0 |
| Base64 | [`base64`](https://crates.io/crates/base64) | Linked with the `cdn` feature, which reads the user id inside an account credential | MIT or Apache-2.0 |
| JavaScript bindings | [`wasm-bindgen`](https://crates.io/crates/wasm-bindgen), [`js-sys`](https://crates.io/crates/js-sys) | Linked on `wasm32` | MIT or Apache-2.0 |
| MessagePack | [`rmp-serde`](https://crates.io/crates/rmp-serde) | Linked | MIT |
| Hashing | [`sha2`](https://crates.io/crates/sha2) | Linked | MIT OR Apache-2.0 |
| CRC-32 of node paths | [`crc32fast`](https://crates.io/crates/crc32fast) | Linked when the `model` feature is enabled | MIT OR Apache-2.0 |
| HTTP and TLS | [`reqwest`](https://crates.io/crates/reqwest), [`hyper`](https://crates.io/crates/hyper), [`h2`](https://crates.io/crates/h2), [`tokio`](https://crates.io/crates/tokio), [`ureq`](https://crates.io/crates/ureq), [`rustls`](https://crates.io/crates/rustls), [`ring`](https://crates.io/crates/ring), [`webpki-roots`](https://crates.io/crates/webpki-roots), [`socks`](https://crates.io/crates/socks) | Linked when the `cdn` or `s3` feature is enabled | MIT OR Apache-2.0; Apache-2.0 OR ISC OR MIT; Apache-2.0 AND ISC; CDLA-Permissive-2.0; MIT OR Apache-2.0 |
| Catalog | [`redb`](https://crates.io/crates/redb) | Linked when the `catalog-db` feature is enabled | MIT OR Apache-2.0 |
| Command-line parsing | [`lexopt`](https://crates.io/crates/lexopt) | Linked | MIT |
| CPU and memory sampling | [`sysinfo`](https://crates.io/crates/sysinfo), [`cpu-time`](https://crates.io/crates/cpu-time) | Linked in the command-line program | MIT; MIT OR Apache-2.0 |
