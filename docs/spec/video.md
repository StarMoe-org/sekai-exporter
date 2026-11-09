# Video

The `video` form is a CRI USM file as an MP4 video that browsers and media players can play. Converter: `video` in `sekai-convert` (feature `video`). Form version: 1.

## 1. Runtime

Web pages and media players that play the game's movies.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `video` | The video stream, the frame rate and picture size of the USM header, and the audio track when the file has one |

## 3. Sources

The root object is a `TextAsset` whose container path ends in `.usm.bytes`. A file can be split into segments, stored in one bundle at the container paths `<base>-001.usm.bytes`, `<base>-002.usm.bytes` and so on. The segment numbered 001 is the root; the bytes of all segments are joined in number order, and the result is the USM file. Every segment is listed in the `sources` of the asset.

An ADX audio track can end before the sample count of its header. The whole frames it holds are decoded.

| Case | Result |
|---|---|
| A segment number between 1 and the highest is missing | Error `missing_segment` |
| The file does not hold exactly one video stream, or the stream is not MPEG-1 | Error `unsupported` |
| The first frame does not begin with an MPEG-1 sequence header, as in an encrypted stream | Error `unsupported` |
| The file has several audio tracks, a track is neither ADX nor HCA, or an HCA track is encrypted | Error `unsupported` |
| The file is damaged | Error `malformed` or `truncated` |
| The `ffmpeg` program cannot be found | Error `encoder_unavailable` |
| The `ffmpeg` program fails | Error `encode_failed` |

## 4. Container

The member is an MP4 file with its index at the start, written by the `mp4-h264-aac` encoder (version 3) through the `ffmpeg` program, which must be installed with `libx264`. The environment variable `SEKAI_FFMPEG` can name the program.

| Stream | Parameters |
|---|---|
| Video | H.264 from `libx264`: preset `medium`, 4 Mb/s, a keyframe every 12 frames, up to 2 B-frames, `yuv420p`, 2 sliced threads, one output frame for each input frame. A picture with an odd width or height is padded to even |
| Audio | AAC from the FFmpeg encoder at 192 kb/s, at the sample rate and channel count of the track |

The container carries no metadata. The same input and the same FFmpeg build give the same bytes. The thread count is fixed because `libx264` output depends on it, and the threads encode slices of the same frame, so the output does not depend on how they are timed, also when other work runs at the same time.

## 5. Correctness

Separating the streams of a USM file is checked byte for byte against an independent USM demultiplexer. The frame count and the duration of the MP4 files are checked against the USM headers with `ffprobe`.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
