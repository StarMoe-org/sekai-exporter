# 音频比对

[English](README.md)

这些脚本在用户自己的文件上，把 `sekai-codec` 的 CRI 读取器和 HCA 解码器与
[vgmstream](https://github.com/vgmstream/vgmstream) 比对。脚本调用本 crate 的示例程序：

```bash
cargo build -p sekai-codec --all-features --release --example decode_hca_stream --example acb_streams
```

`compare.py` 找出目录中各文件里的 HCA 流，逐采样与 `vgmstream-cli` 比对：

```bash
python compare.py <decode_hca_stream> <vgmstream-cli> <目录>...
```

`compare_acb.py` 读取 ACB 文件，对内存波形库中的每条流比对采样，以及播放它的 cue 的名字：

```bash
python compare_acb.py <acb_streams> <decode_hca_stream> <vgmstream-cli> <目录>...
```

有任何差异时，两个脚本都以状态 1 退出。结果已与 vgmstream r2168 核对。

`make_fixtures.py` 用生成的音频构建 `crates/sekai-codec/tests/fixtures/hca`，编码需要
[VGAudio](https://github.com/Thealexbarney/VGAudio)：

```bash
python make_fixtures.py <VGAudioCli> <vgmstream-cli> ../../crates/sekai-codec/tests/fixtures/hca
```
