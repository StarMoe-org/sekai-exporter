# Audio

The `audio` form is the sound of each cue of a CRI cue sheet (ACB), as PCM. Converter: `audio` in `sekai-convert` (feature `audio`). Form version: 1.

## 1. Runtime

Any program that plays or processes the sounds a cue sheet holds: audio players, web pages that play music, voices and sound effects, and tools that edit or analyse sound. The form gives each sound as the waveform the cue plays; it does not apply the cue's volume, pitch, fades, loop playback or the selection rules of a cue with several waveforms.

## 2. Contents

One member per waveform each cue plays, in the order of the cue sheet's `CueTable`:

| Role | Content |
|---|---|
| `cue/<cue name>` | The waveform of a cue that plays one waveform |
| `cue/<cue name>/<n>` | The `n`-th waveform, counting from 1, of a cue that plays several different waveforms |

The waveforms of a cue are those its reference reaches (directly, through a synth, a sequence or a block sequence), in the order the cue sheet lists them. A waveform that a cue lists more than once counts once, at its first place.

Each member is 16-bit PCM, interleaved, at the sample rate and channel count of the stream, without the encoder delay and the padding at the end that the HCA header declares.

## 3. Sources

**Root objects:** every `TextAsset` (class 49) with a container path ending in `.acb.bytes`, compared without case. The form does not read game data tables.

**Fields read:** `m_Script`, which holds the cue sheet.

**Cue sheet.** The sheet is read from its `Header` table: `CueTable`, `CueNameTable`, `WaveformTable`, `WaveformExtensionDataTable` and the tables a cue's reference reaches (`SynthTable`, `SequenceTable`, `BlockSequenceTable`, `BlockTable`, `TrackTable`, `TrackEventTable`). Each waveform is an HCA stream in the sheet's memory bank (`AwbFile`, an AFS2 archive), found by its `MemoryAwbId`.

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

The members are decoded content, which the evaluator encodes. The canonical file is FLAC with 16-bit samples, written by the `flac` encoder (version 1) with the fixed parameters of that encoder. Views that need another container, such as MP3 or AAC, produce it from the same samples with their own encoder.

## 5. Correctness

HCA decoding follows the reference decoder: the same dequantisation, the same intensity and stereo reconstruction, the same inverse transform, and the same conversion to 16 bits (`(int)(f * 32767)`, truncating). The cue sheet reader follows the cue references the way the reference player does.

The decoders are checked sample for sample against vgmstream on HCA files made for the tests, including encrypted ones with and without a subkey (`crates/sekai-codec/tests`), and the converter on cue sheets built in the tests (`crates/sekai-convert/tests/audio.rs`). On real bundles, the streams and cue names the reader finds are compared with vgmstream with the scripts in `tools/compare-audio`, and every member of the form is compared with the stream vgmstream decodes for the same waveform, together with the cue names vgmstream gives that stream.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
