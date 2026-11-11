# Third-party crates with local changes

English | [简体中文](README.zh-CN.md)

Canonical files must be the same bytes on every target, including `wasm32`. These crates are used through `[patch.crates-io]` with the changes below, and are otherwise the published sources.

| Crate | Version | Change |
|---|---|---|
| `rusty_mp3` | 0.6.1 | Sines, cosines, arc tangents, powers, exponentials and logarithms come from the `libm` crate, whose results do not depend on the target. Settings read from environment variables, the stage profiler, the telemetry counters and the `lab` module are removed, so the encoder's output depends only on its input and configuration. At a constant bitrate a granule is quantised by the rate loop at flat scalefactors, starting from the global gain of the previous granule of the channel and widening the step until it brackets the finest gain that fits before bisecting. The encoder runs past the input until a decoder has given its last sample, and the first frame is an Info frame whose tag, in the LAME layout, gives the audio frame count, the encoder delay and the padding, so decoders that read it trim the output to the input exactly. Quantised levels round without a libm call, and the loops of the analysis filterbank and the FFT run across independent sums so that they vectorize; every sum keeps the order of its terms, so the output bytes are those of the scalar loops. |
