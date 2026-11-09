# compare-texture

[中文](README.zh-CN.md)

Checks the texture decoders of `sekai-codec` against reference decoders. The scripts talk to the `decode_stream` example of that crate.

```bash
cargo build --release -p sekai-codec --features texture --example decode_stream
pip install UnityPy==1.25.4
```

| Script | What it does |
|---|---|
| `fuzz_astc.py <decode_stream> [blocks]` | Decodes random blocks of every footprint with the decoder and with ARM's astcenc. The pixels must be equal; a block the decoder rejects must be one astcenc decodes to an error colour. |
| `fuzz_astc_hdr.py <decode_stream> [blocks] [image...]` | Decodes random blocks of every footprint with the decoder and with astcenc in the HDR profile, comparing binary16 values. Optional image files are compared too |
| `compare_crunch.py <decode_stream> <crunch_decode> <bundle directory>...` | Compares every distinct crunched DXT5 texture with the blocks unpacked by Unity's crunch library, decoded with exact arithmetic from the S3TC formulas. `reference/crunch_decode.cpp` says how to build `crunch_decode` |
| `make_compressed_fixtures.py <crunch encoder> <crunch_decode> <output directory>` | Writes the synthetic crunch and ASTC HDR fixtures of the decoder tests |
| `compare.py <decode_stream> <bundle directory>...` | Counts the textures of the bundles per format and compares each distinct image with its reference. The exit status is 1 when an image differs or is rejected. |
| `make_fixtures.py <bundle directory> <output directory>` | Writes the fixtures in `crates/sekai-codec/tests/fixtures/textures` from the bundles that `BuildTextureFixtures.Build` in [unity-fixtures](../unity-fixtures/README.md) builds. |

The references are astcenc for ASTC and texture2ddecoder for ETC. An uncompressed format is compared with a direct expansion of its bytes. Alpha8 expands to black with the stored alpha, the 32-bit pixels Unity reads from that format.

Continuous integration runs `fuzz_astc.py` and `fuzz_astc_hdr.py`, and the test suite checks the fixtures.
