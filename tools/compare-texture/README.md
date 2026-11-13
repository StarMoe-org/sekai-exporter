# compare-texture

[中文](README.zh-CN.md)

Checks the texture decoders of `sekai-codec` against reference decoders. The scripts talk to the `decode_stream` example of that crate.

```bash
cargo build --release -p sekai-codec --features texture --example decode_stream
pip install UnityPy==1.25.4
```

ETC, EAC and PVRTC are compared with the decoders of their owners: `etcdec.cxx` of [ETCPACK](https://github.com/Ericsson/ETCPACK) (Ericsson) and `PVRTDecompress.cpp` of the [PowerVR SDK](https://github.com/powervr-graphics/Native_SDK) (Imagination). `reference/block_decode.cpp` serves both; build it with a checkout of each:

```bash
g++ -O2 -std=c++17 -w -I<PowerVR SDK>/framework/PVRCore/texture reference/block_decode.cpp <ETCPACK>/source/etcdec.cxx <PowerVR SDK>/framework/PVRCore/texture/PVRTDecompress.cpp -o block_decode
```

| Script | What it does |
|---|---|
| `fuzz_astc.py <decode_stream> [blocks]` | Decodes random blocks of every footprint with the decoder and with ARM's astcenc. The pixels must be equal; a block the decoder rejects must be one astcenc decodes to an error colour. |
| `fuzz_astc_hdr.py <decode_stream> [blocks] [image...]` | Decodes random blocks of every footprint with the decoder and with astcenc in the HDR profile, comparing binary16 values. Optional image files are compared too |
| `compare_crunch.py <decode_stream> <crunch_decode> <bundle directory>...` | Compares every distinct crunched DXT5 texture with the blocks unpacked by Unity's crunch library, decoded with exact arithmetic from the S3TC formulas. `reference/crunch_decode.cpp` says how to build `crunch_decode` |
| `make_compressed_fixtures.py <crunch encoder> <crunch_decode> <output directory>` | Writes the synthetic crunch and ASTC HDR fixtures of the decoder tests |
| `fuzz_blocks.py <decode_stream> <block_decode> [images]` | Decodes random ETC1, ETC2, EAC R11 and PVRTC images of random sizes with the decoder and with the references. The pixels must be equal; an ETC1 image the decoder rejects must hold a differential block whose colour leaves the range. |
| `make_block_fixtures.py <block_decode> <output directory>` | Writes the seeded random ETC, EAC and PVRTC fixtures of the decoder tests and their listing `blocks.txt` |
| `compare.py <decode_stream> <block_decode> <bundle directory>...` | Counts the textures of the bundles per format and compares each distinct image with its reference. The exit status is 1 when an image differs or is rejected. |
| `make_fixtures.py <block_decode> <bundle directory> <output directory>` | Writes the fixtures in `crates/sekai-codec/tests/fixtures/textures` from the bundles that `BuildTextureFixtures.Build` in [unity-fixtures](../unity-fixtures/README.md) builds. |

The references are astcenc for ASTC, and ETCPACK and the PowerVR SDK through `block_decode` for ETC, EAC and PVRTC. EAC R11 decodes to eleven bits, where 2047 stands for 1.0, and red is the nearest eight-bit value. The PVRTC RGB formats have the base format RGB, so their alpha is 255. An uncompressed format is compared with a direct expansion of its bytes. Alpha8 expands to black with the stored alpha, the 32-bit pixels Unity reads from that format.

Continuous integration runs `fuzz_astc.py` and `fuzz_astc_hdr.py`, and the test suite checks the fixtures.
