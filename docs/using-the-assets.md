# Using the published assets

[中文](using-the-assets.zh-CN.md)

This document is for readers of a published site: people who browse, download or link to the files the exporter publishes, and programs that read them. It explains what each view and each file is, how to refer to a file so the reference keeps working, and what the audio and video lose against the game's own streams, with the measurements behind those answers and how to repeat them. How the files are produced is described in [Objects and assets](objects-and-assets.md) and [Publishing](publishing.md).

## Versions and addresses

Every update of the game is published as a new version, and a published version never changes. The addresses below follow the routes of the example in [Serving](serving.md#routes); a site may choose other patterns.

| Address | What it names |
|---|---|
| `/{region}@{version}/{path}` | A file of one published version. Its bytes never change, so it can be linked to and cached for good |
| `/{region}/{path}` | The same file in the current version. It follows each new version |
| A path ending in `/` | A [directory listing](serving.md#directories): the subdirectories and files right under it, as JSON |
| A tree route, such as `/trees/{region}/{view}` | The [directory tree](spec/directory-tree.md) of a whole view in one gzip-compressed document |
| A history or changes route | The [version history](spec/version-history.md) of a view, and the files a version added, removed or changed |

A page that browses a whole view reads the tree once and then reads each file by its path in the same version, so the tree and the files never come from two versions.

## The views

| View | Contents |
|---|---|
| `storage` | The files a web page uses directly, by the game's resource paths: images as lossless PNG and as smaller lossy WebP, audio as MP3 and video as MP4 ([storage view](spec/storage-view.md)) |
| `canonical` | Each asset in the form its runtime reads: an asset document `<name>.asset.json` and its member files. Models are glTF, motions and curves are JSON documents. For programs that reproduce how the game shows an asset ([publication](spec/publication.md)) |
| `client` | Assets that ship inside the client package instead of being downloaded, such as honor frames and interface sprites, named by the path the game loads them from |

## Only forms with a named runtime

Every asset form has a named runtime it serves, such as the web page itself, the 3D costume viewer, the MySekai preview, the Live2D viewer or the story player. The form keeps what that runtime needs, and its specification says where each field comes from in the game, what it means and how it is checked against the game.

There are therefore no Unity objects dumped as they are. A field that no runtime reads cannot be checked, its layout can change with any version of the game unnoticed, and a reader would have to understand Unity's serialisation to use it.

Objects that no form claims are not dropped either: they are recorded as unknown in the ledger of the run, until a runtime needs them and a form is defined for them. Why a form is defined by its runtime is explained in [Objects and assets](objects-and-assets.md#who-defines-the-form-of-an-asset).

## Why not a Unity project

The Unity editor could serve as a runtime for everything: rebuild every object into a Unity project, and anything opens. The exporter does not do that:

- The editor is not a web page. Browsers, viewers and scripts cannot read a Unity project, so each of them would still need a conversion of its own.
- A project opens only in a given version of Unity, under its proprietary licence. The forms here are open formats (PNG, MP3, MP4, glTF, JSON) that any environment reads.
- Rebuilding a project means rebuilding scripts, shaders and package dependencies, most of which have nothing to do with what is shown and cannot be checked item by item. The exporter does less, and checks every form it writes against the game.
- A whole project for every version costs far more to store and to update than forms exported as they are needed.

## Files

| File | What it is |
|---|---|
| `*.asset.json` | The [asset document](spec/asset-documents.md): the form of the asset, the bundle and objects it came from, its member files and its fingerprint |
| `*.glb` | A glTF binary [model](spec/model.md). Besides meshes, skins and texture references, the game's material parameters and the sources of its nodes are in `extras` |
| `texture/<n>.png` | The n-th texture of a model or a Live2D model, numbered as the model refers to it |
| `*.moc3` | A Live2D model, with the bytes the game stores |
| `physics.json`, `*.physics3.json` | Live2D physics settings, with the bytes the game stores |
| `motion/*.json`, `facial/*.json` | A [Live2D motion](spec/live2d.md): its fade times and the Unity animation clip it plays, as a [clip document](spec/clip-documents.md) |
| `cues.json` | The cue table of an [audio](spec/audio.md) asset: the waveforms each cue plays, their lengths, loop ranges, volumes and arrangement |
| `*.mp3` | Audio at twice the bitrate of the HCA stream, with the exact sample count and loop points; see the [questions](#questions-about-audio-and-video) below |
| `*.mp4` | [Video](spec/video.md), H.264 at one and a half times the bitrate of the source, with AAC audio; see the [questions](#questions-about-audio-and-video) below |
| `*.png`, `*.webp` | Images. In the `storage` view each image has both: PNG is lossless, WebP is smaller and suits previews |
| `*.bytes`, `*.txt` | A text or binary resource of the game (`TextAsset`), as stored |
| Other `*.json` | A document whose `format` field names its format; each format has a page among the [specifications](spec/README.md) |
| Anything else | Another file of the game's resources, as stored |

## Questions about audio and video

### Why is the audio MP3, and not WAV or FLAC?

The game's audio is [HCA](https://www.criware.com/en/products/adx2.html), a codec of the CRI ADX middleware. Like MP3 and AAC it is lossy: what it leaves out was left out when the game was built. The exporter's HCA decoder gives the samples of [vgmstream](https://github.com/vgmstream/vgmstream) sample for sample, so writing WAV would be easy, but a lossless file of a lossy stream only spreads out the same decoded result: it is several times larger and holds no more sound.

The audio bundles of the Japanese version hold 19279 streams in 5978 bundles, every one HCA at 44.1 kHz. Songs are mostly 117 kb/s stereo and voices 70 kb/s mono; CD audio is 1411 kb/s. The MP3 is encoded at twice the bitrate of its HCA stream, rounded up to an MP3 bitrate: songs at 256 kb/s and voices at 160 kb/s ([audio](spec/audio.md#4-container)).

On 64 clips of songs, background music, interface sounds, story audio and voices, the MP3 was compared with the HCA stream decoded by the reference decoder:

- **Listening.** By [ITU-R BS.1387](https://www.itu.int/rec/R-REC-BS.1387) ([PEAQ](https://en.wikipedia.org/wiki/Perceptual_Evaluation_of_Audio_Quality)), the objective difference grade (ODG) of the basic model has a median of −0.02 and a worst of −0.45; that of the advanced model, on the clips of 3 seconds or more, a median of −0.03 and a worst of −0.29. The ODG predicts the impairment scale of [ITU-R BS.1116](https://www.itu.int/rec/R-REC-BS.1116) listening tests: 0 imperceptible, −1 perceptible but not annoying, −2 slightly annoying, −3 annoying, −4 very annoying. Grades above about −0.5 are commonly taken as hard to tell from the original. Every clip is above −0.5 under both models. A file compared with itself grades about +0.2 under the basic model.
- **Waveform.** The [signal-to-noise ratio](https://en.wikipedia.org/wiki/Signal-to-noise_ratio) of the difference, sample by sample, has a median of 28 dB for songs and 43 dB for voices. A lossy codec leaves out what [auditory masking](https://en.wikipedia.org/wiki/Auditory_masking) hides, so the difference shows in a waveform comparison without showing in listening; part of it can be the quantisation noise of the HCA stream itself.
- **Size**, against the HCA stream: the MP3 is about 2.2 times (voices 2.3), [FLAC](https://xiph.org/flac/) about 7 times (voices 5) and WAV about 12 times (voices 10). A lossless file takes two to three times the space of the MP3 for the decoded result of the same HCA stream.

The original streams are in the game's resources: take the `.acb` (and `.awb`) out of a bundle and decode it with vgmstream, or decode it in a program with `sekai-codec`. Both give the samples of CRI's reference decoder. To listen to the songs at their best, the official releases on music services come from the producers' masters, are usually full length, are at higher bitrates, and support their creators; the songs in the game are the game-size versions, compressed for the client.

### Can the audio be used to train models or for acoustic analysis?

It is not recommended. The concern is not the bitrate: the MP3 is a second lossy encoding on top of the lossy HCA stream ([transcoding](https://en.wikipedia.org/wiki/Transcoding)). The second loss cannot be heard, but for training and analysis:

- **Errors add up.** The decoded HCA samples are already the result of one lossy codec. The MP3 encoder leaves out more by its own psychoacoustic model and quantisation, and adds the traces of a second codec: other quantisation noise, other band limits and pre-echo around transients such as drum hits. This [generation loss](https://en.wikipedia.org/wiki/Generation_loss) only grows.
- **Inaudible is not unlearnable.** PEAQ measures whether a listener hears a difference. Vocoders, speech synthesis, singing voice conversion and source separation learn the statistics of the signal, and can learn the traces of a codec as part of the sound and reproduce them. How much depends on the model and the task, and no general measure bounds it in advance.
- **Timing.** An MP3 encoder adds a delay at the start and pads the last frame. The files record both in a LAME tag, and decoders that read it, such as ffmpeg, give the exact samples ([gapless playback](https://en.wikipedia.org/wiki/Gapless_playback)). Tools that ignore the tag delay the whole clip by about 1057 samples (24 ms) and add silence at the end, which breaks frame-accurate alignment with lines, scores or lip sync.
- **Mixed sources.** A data set that holds audio decoded from the originals next to transcoded audio gives a model the difference between the two codecs as a feature to learn, in the way of what it should learn.

For training or analysis, decode the HCA streams as described above to 16-bit PCM and store them as WAV or FLAC as needed. That is the most complete form of the sound in the game, without a further loss. Check the rights and terms of use of the material before using it.

### Why is the video MP4 with H.264, and what is it in the game?

The game's videos are USM files of the CRI [Sofdec2](https://www.criware.com/en/products/sofdec2.html) middleware: the pictures are [MPEG-1](https://en.wikipedia.org/wiki/MPEG-1), a standard finished in 1993, and the sound is CRI ADX or HCA. A long video is split into several numbered `.usm` files in its bundle, and joined in order it is one video. In 13 videos sampled from the Japanese version the pictures are all MPEG-1, from 512×288 to 3952×2400. The constrained parameters of MPEG-1 keep ordinary streams within 768×576; the larger pictures are outside them, which the standard allows and CRI's encoder writes.

Browsers cannot play the originals: a `<video>` element reads neither the USM container nor MPEG-1, nor ADX or HCA. A web page needs a conversion. [H.264](https://en.wikipedia.org/wiki/Advanced_Video_Coding) in MP4 is the most widely supported combination: browsers, phones, televisions and editors open it, and almost every device decodes it in hardware. The files put their index at the start, so they play while they download, and their sound is AAC. Newer codecs (AV1, VP9, HEVC) are smaller at the same quality, but older devices lack hardware decoders for them, they encode far more slowly, and HEVC does not play in every browser.

### How much picture quality does the MP4 lose?

Some, and little. MPEG-1 is lossy and H.264 is a second lossy encoding. H.264 came about ten years after MPEG-1 and keeps the same pictures in far fewer bits. The video is encoded at one and a half times the bitrate of its MPEG-1 stream, so the bitrate follows the source: a small video at a low bitrate does not become a large file, and a large picture at a high bitrate keeps its detail ([video](spec/video.md)).

On the 13 sampled videos, [VMAF](https://github.com/Netflix/vmaf) of the MP4 against the decoded MPEG-1 stream, frame by frame, has a median of 97.4, a lowest of 94.9 (a title screen of about 4K) and a highest of 98.9. VMAF is a perceptual quality measure from 0 to 100, where 100 is identical to the reference. Scores of 93 and above are commonly taken as [indistinguishable from the source or with only unannoying artefacts](https://www.realnetworks.com/sites/default/files/vmaf_reproducibility_ieee.pdf), and a difference of about 6 points as a [just noticeable difference](https://streaminglearningcenter.com/codecs/finding-the-just-noticeable-difference-with-netflix-vmaf.html).

Kept unchanged: the size of the picture (an odd width or height is padded to even), the frame rate and every frame (one output frame for each input frame), and [4:2:0 chroma subsampling](https://en.wikipedia.org/wiki/Chroma_subsampling), which MPEG-1 already uses. The sound of a video is AAC at 192 kb/s, transcoded as the audio is.

For the original pictures, or to analyse them frame by frame or train on them, take the `.usm` files out of the bundle, joined in order, and demultiplex the MPEG-1 stream with a tool that reads CRI USM, such as [cridecoder](https://github.com/Team-Haruki/cridecoder); VLC, mpv and ffmpeg play it. As with the audio, the transcoded file is for watching, not a source for data.

### Repeating the measurements

The measurements use the exporter's published files and the bundles of a user's own download.

**Audio.** For each clip:

1. Take the HCA stream of the clip from its ACB or AWB file and decode it with `vgmstream-cli -i` (no looping) to WAV: the reference. `tools/compare-audio` checks that `sekai-codec` gives the same samples.
2. Decode the published MP3 with a decoder that reads the LAME tag, such as ffmpeg, or libsndfile with mpg123, to WAV. It has as many samples as the reference.
3. Grade the pair with [GstPEAQ](https://github.com/HSU-ANT/gstpeaq): `peaq <reference> <test>` for the basic model and `peaq --advanced <reference> <test>`, the advanced model only for clips of 3 seconds or more.
4. The signal-to-noise ratio is 10·log₁₀(Σ reference² / Σ (reference − test)²) over all samples and channels.
5. The sizes are those of the HCA stream, the MP3, the reference as 16-bit WAV, and the reference as FLAC (`ffmpeg -i <reference> -c:a flac -compression_level 8`).

**Video.** For each video:

1. Join the `.usm` files of the video in order and demultiplex its MPEG-1 stream to an `.m1v` file. The frame rate is in the header of the USM file; the `usm_streams` example of `sekai-codec` prints it.
2. Score the published MP4 against the stream, with frames paired by their number:

```bash
ffmpeg -f mpegvideo -framerate <rate> -i video.m1v -i video.mp4 -filter_complex \
  "[0:v]pad=ceil(iw/2)*2:ceil(ih/2)*2,settb=1/1000,setpts=N*40[r];[1:v]settb=1/1000,setpts=N*40[t];[t][r]libvmaf" \
  -an -f null -
```

The `pad` filter gives the reference the even size of the MP4, and `setpts=N*40` gives the frames of both the same times, so the score compares frame n with frame n. ffmpeg must be built with libvmaf.
