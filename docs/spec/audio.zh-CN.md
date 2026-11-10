# 音频

[English](audio.md)

`audio` 形式是 CRI cue sheet（ACB）中每个 cue 的声音，以 PCM 保存。转换器：`sekai-convert` 中的 `audio`（feature `audio`）。形式版本：3。

## 1. 运行时

任何播放或处理 cue sheet 所含声音的程序：音频播放器、播放音乐、语音与音效的网页，以及编辑或分析声音的工具。形式把每个声音写成该 cue 播放的波形，并说明每个声音的长度、循环与音量；它不施加 cue 的音量、音高、淡入淡出、循环播放，也不施加含多个波形的 cue 的选取规则。

## 2. 内容

每个 cue 所播放的每个波形一个成员，按 cue sheet 中 `CueTable` 的顺序排列：

| 角色 | 内容 |
|---|---|
| `cue/<cue 名称>` | 只播放一个波形的 cue 的波形 |
| `cue/<cue 名称>/<n>` | 播放多个不同波形的 cue 的第 `n` 个波形，从 1 计数 |

一个 cue 的波形即其引用所到达的波形（直接到达，或经由合成器、序列或块序列到达），按 cue sheet 列出的顺序排列。cue 多次列出的同一波形只计一次，位置取其第一次出现处。

这些成员都是 16 位 PCM，交错存放，采样率与声道数同流，且不含 HCA 头所声明的编码延迟与末尾填充。

最后一个成员是 cue 表：

| 角色 | 内容 |
|---|---|
| `cues` | cue 如何播放其波形，为 JSON，扩展名 `json` |

```json
{ "format": "audio-cues", "version": 1, "cues": [
  { "name": "bgm00000", "waveforms": [
    { "role": "cue/bgm00000", "frames": 500535, "sample_rate": 44100, "channels": 2, "loop": [175959, 500535], "volume": 0.63 }
  ] }
] }
```

| 键 | 值 |
|---|---|
| `format`、`version` | `"audio-cues"` 与本文档格式的版本，1 |
| `cues` | 播放波形的 cue，按成员的顺序排列 |
| `name` | cue 的名称 |
| `role` | 持有该波形的成员的角色 |
| `frames`、`sample_rate`、`channels` | 成员的每声道采样数、以赫兹计的采样率与声道数 |
| `loop` | 波形循环时出现：成员每声道采样中的 `[start, end)`。循环的 cue 先把波形播放到 `end` 一次，然后反复播放 `start` 到 `end` |
| `volume` | cue 播放该波形的音量，1 为所存的电平 |
| `arrangement` | 在需要说明的比它所播放的波形更多时出现在 cue 上，见下文 |

编配说明由轨道构成的 cue 的每个波形在何时播放。cue 的各层同时播放。一个序列按层保存每一层的音符；块序列一个接一个地播放块，每块带有各层的全部音符：

```json
{ "layers": [ { "control": { "name": "BGM_VERTICAL", "default": 0.0, "points": [[0.0, 0.0], [1.0, 1.0]] } }, {} ],
  "blocks": [
    { "length_us": 8000000, "loop": true, "transition_parts": 4, "relative": true,
      "layers": [ [ { "at_us": 0, "role": "cue/bgm90001/1", "volume": 0.63 } ], [] ] }
  ] }
```

| 键 | 值 |
|---|---|
| `layers` | 各层，按 cue 轨道的顺序排列 |
| `control` | 在音量随某个 control 变化的层上出现：control 的 `name`、它在被设置之前所具有的值 `default`，以及 `points`，即写作 `[control, volume]` 的曲线，点之间为线性 |
| `notes` | 序列中某一层的音符。为空时不存在 |
| `blocks` | 块序列的各块，按顺序排列。在序列上不存在 |
| `length_us` | 块的长度，以微秒计 |
| `loop` | 块是否重复，直到请求切换 |
| `repeats` | 不循环的块再播放多少次。为 0 时不存在 |
| `transition_parts` | 在请求的切换等待块被等分为这么多个部分后的下一个部分时出现。没有它时，切换等待块结束 |
| `relative` | 下一块是否从本块已达到的位置开始，而不是从其开头开始 |
| `at_us` | 波形开始的时刻，以微秒计，从其所在轨道的轨道或块的开头算起 |
| `role`、`volume` | 成员的角色以及音符播放的音量 |

## 3. 来源

**根对象：** 容器路径以 `.acb.bytes` 结尾的每个 `TextAsset`（类 49），比较时不区分大小写。形式不读取游戏数据表。

**读取的字段：** `m_Script`，其中保存 cue sheet。

**Cue sheet。** 表从其 `Header` 表读取：`CueTable`、`CueNameTable`、`WaveformTable`、`WaveformExtensionDataTable`，以及 cue 的引用所到达的表（`SynthTable`、`SequenceTable`、`BlockSequenceTable`、`BlockTable`、`TrackTable`、`TrackEventTable`）。每个波形是表内存库（`AwbFile`，一个 AFS2 归档）中的 HCA 流，以其 `MemoryAwbId` 找到。

**循环。** 波形的循环来自其 HCA 头的 `loop` 块：它从采样 `起始帧 × 1024 + 起始延迟` 开始，在采样 `(结束帧 + 1) × 1024 − 结束填充` 之前结束，两者都减去编码延迟。在成员之内不给出范围的块不给出循环。

**音量。** 音量为表的 `AcbVolume` 乘以从 cue 到波形途中的每个音量命令（`0x0057`，以百分之一为单位的 16 位值）：其序列或块序列、每条轨道以及每个合成器的音量命令。块的轨道与该块序列中同一位置的轨道一并计入。一行的命令按其 `CommandIndex` 在 `SeqCommandTable`、`TrackCommandTable` 或 `SynthCommandTable` 中查找（没有前者的表则在 `CommandTable` 中查找）。乘积精确计算，然后转换为 32 位浮点数。cue 以多种方式到达的波形取第一种方式的音量。

**编配。** 当 cue 的引用是块序列，或是多于一条轨道的序列、在其轨道开始之后才开始的音符，或带有局部 AISAC 时，cue 得到一份编配。音符是某条轨道事件的 `0x07D0` 或 `0x07D3` 命令（`NoteOn`）；`0x07D1` 命令会在其后的内容之前等待 32 位数的毫秒，`0x07D5` 命令则等待 16 位数的微秒。到达多个波形的音符同时为每个波形各给出一个音符。块的 `length_us` 是 `Length`（毫秒）与 `LengthUs`；`PlaybackType` 为 1 使其循环，否则 `NumLoops` 给出 `repeats`；`TransitionTiming` 为 1 且 `TransitionTimingValue` 在 1 到 65534 之间时给出 `transition_parts`；`TransitionMode` 为 1 使其 `relative`。层的 control 是其 `LocalAisacs`、`Controls` 与 `Destinations` 中第一个的音量图（`Type` 为 1）除以 10000，以及 `AisacControlNameTable` 中 `ControlId` 的名称，没有列出时为空；`DefaultControlFlag` 为 1 时 `default` 为 `DefaultControl`，否则为 0。轨道数多于其块序列的块为 `malformed`。

**密钥。** 密码类型为 56 的 HCA 流，用提供给转换器的 64 位密钥（`Audio::new(Some(key))`）与其所在内存库的子密钥解密。密钥是运行的配置；它不随资产保存。

**成员以外的结果：**

| 情况 | 结果 |
|---|---|
| 没有任何 cue 播放波形 | 无资产 |
| 某个 cue 没有名称或有多个名称 | 错误 `unsupported` |
| 某个 cue 播放流式波形（`Streaming` 不为 0） | 错误 `unsupported` |
| 某个 cue 播放不是 HCA 的波形（`EncodeType` 不为 2） | 错误 `unsupported` |
| 某个流已加密，而转换器没有密钥 | 错误 `missing_key` |
| 表或某个流损坏 | 错误 `malformed` 或 `truncated` |

## 4. 容器

波形成员是已解码内容，由求值器编码。规范文件是带 16 位采样的 FLAC，由 `flac` 编码器（版本 1）以该编码器的固定参数写出。需要其他容器的视图，例如 MP3 或 AAC，由同一批采样经其自身的编码器产生。cue 表是成品字节：UTF-8 JSON，无多余空白，键的顺序如上，缺省时省略 `loop`；音量写为能读回同一 32 位浮点数的最短十进制形式。

## 5. 正确性

HCA 解码遵循参考解码器：相同的反量化、相同的强度与立体声重建、相同的逆变换，以及相同的 16 位转换（`(int)(f * 32767)`，截断）。cue sheet 读取器按参考播放器的方式跟随 cue 引用。

解码器在专为测试制作的 HCA 文件上，与 vgmstream 逐采样比对，包括带与不带子密钥的加密文件（`crates/sekai-codec/tests`）；转换器在测试中构建的 cue sheet 上比对（`crates/sekai-convert/tests/audio.rs`）。在真实资源包上，读取器找到的流与 cue 名称用 `tools/compare-audio` 中的脚本与 vgmstream 比对，并且形式的每个成员都与 vgmstream 为同一波形解码出的流比对，同时比对 vgmstream 赋予该流的 cue 名称。

## 6. 版本

| 版本 | 变更 |
|---|---|
| 1 | 第一版 |
| 2 | cue 表 |
| 3 | 由轨道构成的 cue 的编配，以及块轨道在其层之下的音量 |
