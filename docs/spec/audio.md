# Audio

[中文](audio.zh-CN.md)

The `audio` form is the sound of each cue of a CRI cue sheet (ACB), as MP3. Converter: `audio` in `sekai-convert` (feature `audio`). Form version: 5.

## 1. Runtime

Any program that plays or processes the sounds a cue sheet holds: audio players, web pages that play music, voices and sound effects, and tools that edit or analyse sound. The form gives each sound as the waveform the cue plays, and states the length, the loop and the volume of each; it does not apply the cue's volume, pitch, fades, loop playback or the selection rules of a cue with several waveforms.

## 2. Contents

One member per waveform each cue plays, in the order of the cue sheet's `CueTable`:

| Role | Content |
|---|---|
| `cue/<cue name>` | The waveform of a cue that plays one waveform |
| `cue/<cue name>/<n>` | The `n`-th waveform, counting from 1, of a cue that plays several different waveforms |

The waveforms of a cue are those its reference reaches (directly, through a synth, a sequence or a block sequence), in the order the cue sheet lists them. A waveform that a cue lists more than once counts once, at its first place.

Each of these members is the waveform at the sample rate and channel count of the stream, without the encoder delay and the padding at the end that the HCA header declares. Its file is an MP3 (see [Container](#4-container)); waveforms with the same stream share one file.

The last member is the cue table:

| Role | Content |
|---|---|
| `cues` | How the cues play their waveforms, as JSON, extension `json` |

```json
{ "format": "audio-cues", "version": 1, "cues": [
  { "name": "bgm00000", "waveforms": [
    { "role": "cue/bgm00000", "frames": 500535, "sample_rate": 44100, "channels": 2, "loop": [175959, 500535], "volume": 0.63 }
  ] }
] }
```

| Key | Value |
|---|---|
| `format`, `version` | `"audio-cues"` and the version of this document format, 1 |
| `cues` | The cues that play a waveform, in the order of the members |
| `name` | The name of the cue |
| `role` | The role of the member holding the waveform |
| `frames`, `sample_rate`, `channels` | The length in samples per channel, the sample rate in hertz and the channel count of the member |
| `loop` | Present when the waveform loops: `[start, end)` in samples per channel of the member. A looping cue plays the waveform once up to `end`, then `start` to `end` again and again |
| `volume` | The volume the cue plays the waveform at, 1 being the stored level |
| `arrangement` | Present on a cue with more to say than the waveforms it plays, as below |

An arrangement states when each waveform of a cue built of tracks plays. The layers of a cue play at the same time. A sequence has the notes of each layer in the layer; a block sequence plays one block after another, each with the notes of every layer:

```json
{ "layers": [ { "control": { "name": "BGM_VERTICAL", "default": 0.0, "points": [[0.0, 0.0], [1.0, 1.0]] } }, {} ],
  "blocks": [
    { "length_us": 8000000, "loop": true, "transition_parts": 4, "relative": true,
      "layers": [ [ { "at_us": 0, "role": "cue/bgm90001/1", "volume": 0.63 } ], [] ] }
  ] }
```

| Key | Value |
|---|---|
| `layers` | The layers, in the order of the tracks of the cue |
| `control` | Present on a layer whose volume follows a control: `name` of the control, the value `default` it has until set, and `points`, the curve as `[control, volume]`, linear between the points |
| `notes` | The notes of a layer of a sequence. Absent when empty |
| `blocks` | The blocks of a block sequence, in order. Absent on a sequence |
| `length_us` | The length of the block in microseconds |
| `loop` | Whether the block repeats until a transition is requested |
| `repeats` | How many more times a block that does not loop plays. Absent when 0 |
| `transition_parts` | Present when a requested transition waits for the next of this many equal parts of the block. Without it, a transition waits for the end of the block |
| `relative` | Whether the next block starts at the position this one has reached, rather than at its start |
| `at_us` | When the waveform starts, in microseconds from the start of its layer's track or block |
| `role`, `volume` | The role of the member and the volume the note plays at |

## 3. Sources

**Root objects:** every `TextAsset` (class 49) with a container path ending in `.acb.bytes`, compared without case. The form does not read game data tables.

**Fields read:** `m_Script`, which holds the cue sheet.

**Cue sheet.** The sheet is read from its `Header` table: `CueTable`, `CueNameTable`, `WaveformTable`, `WaveformExtensionDataTable` and the tables a cue's reference reaches (`SynthTable`, `SequenceTable`, `BlockSequenceTable`, `BlockTable`, `TrackTable`, `TrackEventTable`). Each waveform is an HCA stream in the sheet's memory bank (`AwbFile`, an AFS2 archive), found by its `MemoryAwbId`. A sheet whose `WaveformTable` is empty has no waveforms.

**Loop.** The loop of a waveform comes from the `loop` chunk of its HCA header: it starts at sample `start frame × 1024 + start delay` and ends before sample `(end frame + 1) × 1024 − end padding`, both less the encoder delay. A chunk that gives no range within the member gives no loop.

**Volume.** The volume is `AcbVolume` of the sheet times every volume command (`0x0057`, a 16-bit value in hundredths) on the way from the cue to the waveform: those of its sequence or block sequence, of each track and of each synth. A track of a block counts together with the track of the block sequence at the same place. The commands of a row are found by its `CommandIndex` in `SeqCommandTable`, `TrackCommandTable` or `SynthCommandTable`, or in `CommandTable` for a sheet without the former. The product is computed exactly and then converted to a 32-bit float. A waveform a cue reaches in several ways has the volume of the first.

**Arrangement.** A cue gets one when its reference is a block sequence, or a sequence that is more than one track, starts a note after the start of its track, or has a local AISAC. A note is a `0x07D0` or `0x07D3` command (`NoteOn`) of a track's event; a `0x07D1` command waits a 32-bit number of milliseconds and a `0x07D5` command a 16-bit number of microseconds before whatever follows. A note reaching several waveforms gives one note per waveform at the same time. A block's `length_us` is `Length` in milliseconds and `LengthUs`; `PlaybackType` 1 makes it loop, otherwise `NumLoops` gives `repeats`; `TransitionTiming` 1 with a `TransitionTimingValue` between 1 and 65534 gives `transition_parts`; `TransitionMode` 1 makes it `relative`. A layer's control is the volume graph (`Type` 1) of the first of its `LocalAisacs`, its `Controls` and its `Destinations` divided by 10000, and the name of `ControlId` in `AisacControlNameTable`, empty when none is listed; `default` is `DefaultControl` when `DefaultControlFlag` is 1, otherwise 0. A block with more tracks than its block sequence is `malformed`.

**Key.** HCA streams of cipher type 56 are decrypted with a 64-bit key given to the converter (`Audio::new(Some(key))`) and the subkey of the bank the stream comes from. The key is configuration of the run; it is not stored with the assets.

**Outcomes other than members:**

| Case | Result |
|---|---|
| No cue plays a waveform | No asset |
| A cue has no name or several names | Error `unsupported` |
| A cue plays a streamed waveform (`Streaming` other than 0) | Error `unsupported` |
| A cue plays a waveform that is not HCA (`EncodeType` other than 2) | Error `unsupported` |
| A stream is encrypted and the converter has no key | Error `missing_key` |
| The sheet or a stream is damaged | Error `malformed` or `truncated` |

## 4. Container

The converter gives each waveform as its HCA stream, which the evaluator encodes: the `mp3` encoder (version 3) decodes the stream frame by frame and encodes each block of samples as it comes, so the samples of a waveform are never all in memory. Members of one asset with the same stream and key are encoded once.

The file is MPEG audio layer III at a constant bitrate: MPEG-1 for 32, 44.1 and 48 kHz, MPEG-2 or MPEG-2.5 for lower rates. The bitrate is one and a half times the bitrate of the HCA stream (its frame size × 8 × sample rate / 1024), rounded up to a bitrate of that MPEG version and at most its highest, 320 kb/s for MPEG-1 and 160 kb/s for MPEG-2 and MPEG-2.5. Samples without a lossy source take the highest.

The first frame is an Info frame with a tag in the LAME format that gives the frame count, the encoder delay (528 samples, plus the 529 of the decoder) and the padding at the end. Decoders that read the tag, such as ffmpeg and Symphonia, give exactly the samples of the waveform, so `frames` and `loop` in the cue table count the samples of the decoded file. Decoders apply the tag only when its encoder string starts with a name they know, so the string is `LAME` followed by zeros.

The encoder is pure Rust (see [Third-party sources](../third-party.md)) and computes with functions whose results do not depend on the target, so a waveform encodes to the same bytes in native and WebAssembly builds.

**Why MP3.** HCA is a lossy transform codec: the samples the reference decoder gives are all the stream holds. A lossless file of them, WAV or FLAC, keeps one decoding exactly, but adds nothing a decoder of the stream could not give again, and its size follows the sample rate and the length rather than the stream: 16-bit stereo at 48 kHz is 1536 kb/s as WAV, and FLAC compresses the noise a lossy codec leaves poorly, so the file is several times the size of the stream it came from. The HCA stream itself plays only where a CRI decoder is present. MP3 plays in every browser and player, records its delay and padding for gapless playback, and has a pure-Rust encoder whose output is fixed. The AAC encoders the exporter can use are C libraries loaded at run time in native builds, so an AAC file would depend on the library installed and could not be written in WebAssembly, and Opus works at 48 kHz only, so 44.1 kHz waveforms would be resampled.

Encoding a lossy stream again adds the errors of a second codec. The bitrate is set above the source's so that this second loss stays small next to the first, and it stays tied to the source, so a stream encoded at a low bitrate does not become a large file. Programs that need the exact samples of a waveform decode its HCA stream from the bundle with `sekai-codec`.

The cue table is finished bytes: UTF-8 JSON without insignificant whitespace, with keys in the order above and `loop` omitted when absent; a volume is the shortest decimal that reads back to the same 32-bit float.

## 5. Correctness

HCA decoding follows the reference decoder: the same dequantisation, the same intensity and stereo reconstruction, the same inverse transform, and the same conversion to 16 bits (`(int)(f * 32767)`, truncating). The cue sheet reader follows the cue references the way the reference player does.

The decoders are checked sample for sample against vgmstream on HCA files made for the tests, including encrypted ones with and without a subkey (`crates/sekai-codec/tests`), and the converter on cue sheets built in the tests (`crates/sekai-convert/tests/audio.rs`). On real bundles, the streams and cue names the reader finds are compared with vgmstream with the scripts in `tools/compare-audio`, and every waveform the reader finds is decoded and compared with the stream vgmstream decodes for it, together with the cue names vgmstream gives that stream.

The MP3 encoder is checked in `crates/sekai-encode/tests/canonical.rs`: a file decodes with Symphonia to exactly as many samples as were encoded and close to them, its bitrate follows the rule above, and its bytes are fixed. `crates/sekai-codec/tests/cri.rs` checks that decoding a stream frame by frame gives the samples of decoding it whole.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
| 2 | The cue table |
| 3 | The arrangement of cues built of tracks, and the volume of a block's track under its layer |
| 4 | Waveforms are MP3 files at a bitrate that follows the stream |
| 5 | A sheet with an empty waveform table has no waveforms, so one whose cues play nothing gives no asset |
