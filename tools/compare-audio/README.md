# Audio comparison

[中文](README.zh-CN.md)

These scripts compare the CRI readers and the HCA decoder of `sekai-codec` with
[vgmstream](https://github.com/vgmstream/vgmstream) on a user's own files. They run the example
programs of the crate:

```bash
cargo build -p sekai-codec --all-features --release --example decode_audio_stream --example acb_streams --example usm_streams
```

`compare.py` finds the HCA streams inside the files of a directory, and reads files that are
whole ADX streams, and compares each with `vgmstream-cli`, sample by sample:

```bash
python compare.py <decode_audio_stream> <vgmstream-cli> <directory>...
```

`compare_acb.py` reads ACB files and compares, for every stream of the memory bank, the samples and
the names of the cues that play it:

```bash
python compare_acb.py <acb_streams> <decode_audio_stream> <vgmstream-cli> <directory>...
```

Both exit with status 1 if anything differs. The results were checked against vgmstream r2168.

`compare_usm.py` compares the streams separated from USM files with
[WannaCRI](https://github.com/donmai-me/WannaCRI), byte by byte:

```bash
python compare_usm.py <usm_streams> <directory>...
```

`make_fixtures.py` builds `crates/sekai-codec/tests/fixtures/hca` from generated audio. It needs
[VGAudio](https://github.com/Thealexbarney/VGAudio) to encode:

```bash
python make_fixtures.py <VGAudioCli> <vgmstream-cli> ../../crates/sekai-codec/tests/fixtures/hca
```
