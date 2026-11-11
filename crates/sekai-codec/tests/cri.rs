//! HCA decoding against samples from vgmstream, and ACB and AWB reading of files built here.

#![cfg(feature = "cri")]

use std::fs;
use std::path::PathBuf;

use sekai_codec::DecodeError;
use sekai_codec::cri::{
    Hca, HcaKey, decode_adx, decode_hca, hca_info, read_acb, read_awb, read_usm,
};
use sha2::{Digest, Sha256};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hca");
    fs::read(path.join(name)).expect("the fixture exists")
}

#[test]
fn decoded_samples_match_the_reference() {
    let listing = String::from_utf8(fixture("expected.txt")).expect("text");
    let mut count = 0;
    for line in listing.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let [name, rate, channels, bytes, digest, key, subkey] = fields[..] else {
            panic!("bad line {line}");
        };
        let key = u64::from_str_radix(key, 16).expect("hex");
        let subkey = u16::from_str_radix(subkey, 16).expect("hex");
        let key = (key != 0).then_some(HcaKey { key, subkey });
        let data = fixture(name);
        let decoded = if name.ends_with(".adx") {
            decode_adx(&data)
        } else {
            decode_hca(&data, key)
        };
        let pcm = decoded.expect(name);
        assert_eq!(pcm.sample_rate().to_string(), rate, "{name}");
        assert_eq!(pcm.channels().to_string(), channels, "{name}");
        let samples: Vec<u8> = pcm.samples().iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(samples.len().to_string(), bytes, "{name}");
        let actual: String = Sha256::digest(&samples)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(actual, digest, "{name}");
        count += 1;
    }
    assert_eq!(count, 9);
}

#[test]
fn a_stream_decodes_frame_by_frame_to_the_whole_decode() {
    for name in ["sine_mono.hca", "sine_stereo.hca"] {
        let data = fixture(name);
        let whole = decode_hca(&data, None).expect(name);
        let hca = Hca::new(data, None).expect(name);
        let mut samples = Vec::new();
        hca.decode_each(|block| samples.extend_from_slice(block))
            .expect(name);
        assert_eq!(samples, whole.samples(), "{name}");
    }
}

#[test]
fn the_bitrate_is_the_frames_over_their_duration() {
    let data = fixture("sine_stereo.hca");
    let info = hca_info(&data).expect("info");
    let header = u64::from(u16::from_be_bytes([data[6], data[7]]));
    let body = u64::try_from(data.len()).expect("size") - header;
    let per_second = body * 8 * u64::from(info.sample_rate) / info.frames;
    assert_eq!(u64::from(info.bitrate), per_second);
}

#[test]
fn a_stream_is_checked_when_it_is_opened() {
    let mut data = fixture("sine_mono.hca");
    let last = data.len() - 1;
    data[last] ^= 1;
    let result = Hca::new(data, None);
    assert!(matches!(result, Err(DecodeError::Malformed { .. })));
}

#[test]
fn encrypted_stream_needs_a_key() {
    let result = decode_hca(&fixture("sine_stereo_key.hca"), None);
    assert!(matches!(result, Err(DecodeError::MissingKey { .. })));
}

#[test]
fn frame_with_a_wrong_checksum_is_malformed() {
    let mut data = fixture("sine_mono.hca");
    let last = data.len() - 1;
    data[last] ^= 1;
    let result = decode_hca(&data, None);
    assert!(matches!(result, Err(DecodeError::Malformed { .. })));
}

#[test]
fn cut_stream_is_truncated() {
    let data = fixture("sine_mono.hca");
    let result = decode_hca(&data[..data.len() - 1], None);
    assert!(matches!(result, Err(DecodeError::Truncated { .. })));
}

fn afs2(subkey: u16, streams: &[(u16, &[u8])]) -> Vec<u8> {
    const ALIGNMENT: usize = 0x20;
    let mut out = b"AFS2\x01\x04\x02\x00".to_vec();
    out.extend((streams.len() as u32).to_le_bytes());
    out.extend((ALIGNMENT as u16).to_le_bytes());
    out.extend(subkey.to_le_bytes());
    for (id, _) in streams {
        out.extend(id.to_le_bytes());
    }
    let mut body = Vec::new();
    let first = out.len() + (streams.len() + 1) * 4;
    let mut offset = first;
    for (_, data) in streams {
        out.extend((offset as u32).to_le_bytes());
        let start = offset.next_multiple_of(ALIGNMENT);
        body.resize(start - first, 0);
        body.extend(*data);
        offset = first + body.len();
    }
    out.extend((offset as u32).to_le_bytes());
    out.extend(body);
    out
}

#[test]
fn bank_gives_each_stream_and_the_subkey() {
    let plain = decode_hca(&fixture("sine_stereo.hca"), None).expect("plain");
    let stream = fixture("sine_stereo_subkey.hca");
    let bank = afs2(0x4A1C, &[(7, b"not read"), (9, &stream)]);
    let awb = read_awb(&bank).expect("bank");
    assert_eq!(awb.subkey, 0x4A1C);
    assert_eq!(awb.streams.len(), 2);
    assert_eq!(awb.streams[0].wave_id, 7);
    let found = awb.stream(9).expect("stream 9");
    assert_eq!(found.data, &stream[..]);
    let key = HcaKey {
        key: 0x0123456789ABCDE,
        subkey: awb.subkey,
    };
    let decoded = decode_hca(found.data, Some(key)).expect("decoded");
    assert_eq!(decoded.samples(), plain.samples());
}

#[test]
fn bank_with_a_stream_outside_the_file_is_malformed() {
    let mut bank = afs2(0, &[(0, b"stream")]);
    bank.truncate(bank.len() - 1);
    let last_offset = bank.len() + 1;
    let position = 0x10 + 2 + 4;
    bank[position..position + 4].copy_from_slice(&(last_offset as u32 + 64).to_le_bytes());
    assert!(matches!(
        read_awb(&bank),
        Err(DecodeError::Malformed { .. })
    ));
}

enum Cell {
    U8(u8),
    U16(u16),
    U32(u32),
    F32(f32),
    Text(&'static str),
    Data(Vec<u8>),
}

/// Builds an `@UTF` table whose columns all live in the rows.
fn utf(name: &str, columns: &[&str], rows: &[Vec<Cell>]) -> Vec<u8> {
    let mut strings = b"<NULL>\0".to_vec();
    let text = |value: &str, strings: &mut Vec<u8>| {
        let offset = strings.len() as u32;
        strings.extend(value.as_bytes());
        strings.push(0);
        offset
    };
    let name_offset = text(name, &mut strings);
    let mut schema = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        let kind = match &rows[0][index] {
            Cell::U8(_) => 0u8,
            Cell::U16(_) => 2,
            Cell::U32(_) => 4,
            Cell::F32(_) => 8,
            Cell::Text(_) => 10,
            Cell::Data(_) => 11,
        };
        schema.push(0x50 | kind);
        let offset = text(column, &mut strings);
        schema.extend(offset.to_be_bytes());
    }
    let mut body = Vec::new();
    let mut blobs: Vec<u8> = Vec::new();
    for row in rows {
        for cell in row {
            match cell {
                Cell::U8(value) => body.push(*value),
                Cell::U16(value) => body.extend(value.to_be_bytes()),
                Cell::U32(value) => body.extend(value.to_be_bytes()),
                Cell::F32(value) => body.extend(value.to_bits().to_be_bytes()),
                Cell::Text(value) => {
                    let offset = text(value, &mut strings);
                    body.extend(offset.to_be_bytes());
                }
                Cell::Data(value) => {
                    body.extend((blobs.len() as u32).to_be_bytes());
                    body.extend((value.len() as u32).to_be_bytes());
                    blobs.extend(value);
                }
            }
        }
    }
    let rows_offset = 0x18 + schema.len();
    let strings_offset = rows_offset + body.len();
    let data_offset = strings_offset + strings.len();
    let size = data_offset + blobs.len();

    let mut out = b"@UTF".to_vec();
    out.extend((size as u32).to_be_bytes());
    out.extend(1u16.to_be_bytes());
    out.extend((rows_offset as u16).to_be_bytes());
    out.extend((strings_offset as u32).to_be_bytes());
    out.extend((data_offset as u32).to_be_bytes());
    out.extend(name_offset.to_be_bytes());
    out.extend((columns.len() as u16).to_be_bytes());
    out.extend(((body.len() / rows.len()) as u16).to_be_bytes());
    out.extend((rows.len() as u32).to_be_bytes());
    out.extend(schema);
    out.extend(body);
    out.extend(strings);
    out.extend(blobs);
    out
}

fn sheet() -> Vec<u8> {
    use Cell::{Data, Text, U8, U16, U32};
    let cue = |kind, index, id| vec![U8(kind), U16(index), U32(id), U32(1500)];
    let cues = utf(
        "Cue",
        &["ReferenceType", "ReferenceIndex", "CueId", "Length"],
        &[cue(1, 1, 10), cue(2, 0, 20), cue(3, 0, 30), cue(8, 0, 40)],
    );
    let name = |text, index| vec![Text(text), U16(index)];
    let names = utf(
        "CueName",
        &["CueName", "CueIndex"],
        &[
            name("direct", 0),
            name("synth", 1),
            name("sequence", 2),
            name("block sequence", 3),
        ],
    );
    let wave = |id, flag, samples| {
        vec![
            U8(0),
            U16(id),
            U16(0xFFFF),
            U8(2),
            U8(flag),
            U8(2),
            U16(44100),
            U32(samples),
            U16(if flag == 0 { 0xFFFF } else { 0 }),
        ]
    };
    let waves = utf(
        "Waveform",
        &[
            "Streaming",
            "MemoryAwbId",
            "StreamAwbId",
            "EncodeType",
            "LoopFlag",
            "NumChannels",
            "SamplingRate",
            "NumSamples",
            "ExtensionData",
        ],
        &[wave(3, 0, 100), wave(4, 1, 200), wave(5, 0, 300)],
    );
    let extension = utf(
        "WaveformExtensionData",
        &["LoopStart", "LoopEnd"],
        &[vec![U32(16), U32(180)]],
    );
    let synth = utf(
        "Synth",
        &["ReferenceItems"],
        &[vec![Data(vec![0, 1, 0, 2, 0, 1, 0, 0])]],
    );
    let sequence = utf(
        "Sequence",
        &["NumTracks", "TrackIndex"],
        &[vec![U16(2), Data(vec![0, 1, 0, 0])]],
    );
    // A block sequence with track 2 of its own and block 0, which has track 0.
    let block_sequence = utf(
        "BlockSequence",
        &["NumTracks", "TrackIndex", "NumBlocks", "BlockIndex"],
        &[vec![U16(1), Data(vec![0, 2]), U16(1), Data(vec![0, 0])]],
    );
    let blocks = utf(
        "Block",
        &["NumTracks", "TrackIndex"],
        &[vec![U16(1), Data(vec![0, 0])]],
    );
    let tracks = utf(
        "Track",
        &["EventIndex"],
        &[vec![U16(0)], vec![U16(0xFFFF)], vec![U16(1)]],
    );
    // A command that names nothing, then a note on of synth 0.
    let event = vec![0x03, 0xE8, 1, 0, 0x07, 0xD0, 4, 0, 2, 0, 0];
    // A note on of waveform 1.
    let direct = vec![0x07, 0xD0, 4, 0, 1, 0, 1];
    let events = utf(
        "TrackEvent",
        &["Command"],
        &[vec![Data(event)], vec![Data(direct)]],
    );
    utf(
        "Header",
        &[
            "CueTable",
            "CueNameTable",
            "WaveformTable",
            "WaveformExtensionDataTable",
            "SynthTable",
            "SequenceTable",
            "BlockSequenceTable",
            "BlockTable",
            "TrackTable",
            "TrackEventTable",
            "AwbFile",
        ],
        &[vec![
            Data(cues),
            Data(names),
            Data(waves),
            Data(extension),
            Data(synth),
            Data(sequence),
            Data(block_sequence),
            Data(blocks),
            Data(tracks),
            Data(events),
            Data(afs2(0, &[(3, b"three"), (4, b"four")])),
        ]],
    )
}

#[test]
fn cue_sheet_gives_cues_waveforms_and_its_bank() {
    let data = sheet();
    let acb = read_acb(&data).expect("sheet");
    assert_eq!(acb.cues.len(), 4);
    assert_eq!(acb.cues[0].cue_names, ["direct"]);
    assert_eq!(acb.cues[0].cue_id, 10);
    assert_eq!(acb.cues[0].length, 1500);
    assert_eq!(acb.cues[0].waveforms, [1]);
    assert_eq!(acb.cues[1].waveforms, [2, 0]);
    assert_eq!(acb.cues[2].cue_names, ["sequence"]);
    assert_eq!(acb.cues[2].waveforms, [2, 0]);
    assert_eq!(acb.cues[3].cue_names, ["block sequence"]);
    assert_eq!(acb.cues[3].waveforms, [1, 2, 0]);
    assert!(
        acb.cues
            .iter()
            .all(|cue| cue.volumes.len() == cue.waveforms.len())
    );
    assert!(
        acb.cues
            .iter()
            .flat_map(|cue| &cue.volumes)
            .all(|&v| v == 1.0)
    );

    let wave = &acb.waveforms[1];
    assert_eq!(wave.memory_awb_id, Some(4));
    assert_eq!(wave.stream_awb_id, None);
    assert_eq!(wave.loop_flag, 1);
    assert_eq!(wave.num_channels, 2);
    assert_eq!(wave.sampling_rate, 44100);
    assert_eq!(wave.num_samples, 200);
    assert_eq!((wave.loop_start, wave.loop_end), (Some(16), Some(180)));
    assert_eq!(acb.waveforms[0].loop_start, None);

    let bank = acb.memory_awb.expect("bank");
    assert_eq!(bank.stream(4).expect("stream 4").data, b"four");
}

/// A sheet of one cue whose sequence has a track with a command that names nothing, and an
/// empty waveform table and bank, with the waveform that `played` names when it is given.
fn silent_sheet(played: Option<u16>) -> Vec<u8> {
    use Cell::{Data, Text, U8, U16, U32};
    let cues = utf(
        "Cue",
        &["ReferenceType", "ReferenceIndex", "CueId", "Length"],
        &[vec![U8(3), U16(0), U32(0), U32(0)]],
    );
    let names = utf(
        "CueName",
        &["CueName", "CueIndex"],
        &[vec![Text("silent"), U16(0)]],
    );
    let sequence = utf(
        "Sequence",
        &["NumTracks", "TrackIndex"],
        &[vec![U16(1), Data(vec![0, 0])]],
    );
    let tracks = utf("Track", &["EventIndex"], &[vec![U16(0)]]);
    let command = match played {
        Some(waveform) => {
            let [high, low] = waveform.to_be_bytes();
            vec![0x07, 0xD0, 4, 0, 1, high, low]
        }
        None => vec![0x03, 0xE8, 1, 0],
    };
    let events = utf("TrackEvent", &["Command"], &[vec![Data(command)]]);
    utf(
        "Header",
        &[
            "CueTable",
            "CueNameTable",
            "WaveformTable",
            "SequenceTable",
            "TrackTable",
            "TrackEventTable",
            "AwbFile",
        ],
        &[vec![
            Data(cues),
            Data(names),
            Data(Vec::new()),
            Data(sequence),
            Data(tracks),
            Data(events),
            Data(Vec::new()),
        ]],
    )
}

#[test]
fn a_sheet_whose_cue_plays_nothing_has_no_waveforms() {
    let data = silent_sheet(None);
    let acb = read_acb(&data).expect("sheet");
    assert_eq!(acb.cues.len(), 1);
    assert_eq!(acb.cues[0].cue_names, ["silent"]);
    assert!(acb.cues[0].waveforms.is_empty());
    assert!(acb.waveforms.is_empty());
    assert!(acb.memory_awb.is_none());
}

#[test]
fn a_cue_that_plays_a_waveform_of_an_empty_table_is_malformed() {
    let data = silent_sheet(Some(0));
    assert!(matches!(
        read_acb(&data),
        Err(DecodeError::Malformed { .. })
    ));
}

#[test]
fn cut_table_is_reported() {
    let data = sheet();
    assert!(matches!(
        read_acb(&data[..data.len() - 1]),
        Err(DecodeError::Truncated { .. })
    ));
    assert!(matches!(
        read_acb(b"@UTX0000000000000000000000000000"),
        Err(DecodeError::Malformed { .. })
    ));
}

fn usm_chunk(kind: &[u8; 4], channel: u8, data_type: u8, time: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = kind.to_vec();
    out.extend((payload.len() as u32 + 0x18).to_be_bytes());
    out.extend([0, 0x18, 0, 0, channel, 0, 0, data_type]);
    out.extend(time.to_be_bytes());
    out.extend(3000u32.to_be_bytes());
    out.extend([0; 8]);
    out.extend(payload);
    out
}

fn movie() -> Vec<u8> {
    use Cell::U32;
    let video = utf(
        "VIDEO_HDRINFO",
        &[
            "mpeg_codec",
            "width",
            "height",
            "framerate_n",
            "framerate_d",
        ],
        &[vec![U32(1), U32(640), U32(360), U32(30000), U32(1000)]],
    );
    let audio = utf(
        "AUDIO_HDRINFO",
        &["audio_codec", "sampling_rate", "num_channels"],
        &[vec![U32(2), U32(48000), U32(2)]],
    );
    let seek = utf(
        "VIDEO_SEEKINFO",
        &["ofs_frmid"],
        &[vec![U32(0)], vec![U32(2)]],
    );
    let directory = utf("CRIUSF_DIR_STREAM", &["fmtver"], &[vec![U32(1)]]);
    [
        usm_chunk(b"CRID", 0, 1, 0, &directory),
        usm_chunk(b"@SFV", 0, 1, 0, &video),
        usm_chunk(b"@SFA", 0, 1, 0, &audio),
        usm_chunk(b"@SFV", 0, 3, 0, &seek),
        usm_chunk(b"@SFV", 0, 2, 0, b"#HEADER END"),
        usm_chunk(b"@SFV", 0, 0, 0, b"first"),
        usm_chunk(b"@SFA", 0, 0, 0, b"AUDIO"),
        usm_chunk(b"@SFV", 0, 0, 100, b"second"),
        usm_chunk(b"@SFA", 0, 0, 100, b"MORE"),
        usm_chunk(b"@SFV", 0, 0, 200, b"third"),
    ]
    .concat()
}

#[test]
fn bytes_after_the_last_stream_end_are_ignored() {
    use Cell::U32;
    let video = utf(
        "VIDEO_HDRINFO",
        &[
            "mpeg_codec",
            "width",
            "height",
            "framerate_n",
            "framerate_d",
        ],
        &[vec![U32(1), U32(640), U32(360), U32(30000), U32(1000)]],
    );
    let directory = utf("CRIUSF_DIR_STREAM", &["fmtver"], &[vec![U32(1)]]);
    let mut data = [
        usm_chunk(b"CRID", 0, 1, 0, &directory),
        usm_chunk(b"@SFV", 0, 1, 0, &video),
        usm_chunk(b"@SFV", 0, 2, 0, b"#HEADER END"),
        usm_chunk(b"@SFV", 0, 0, 0, b"only"),
        usm_chunk(b"@SFV", 0, 2, 0, b"#CONTENTS END"),
    ]
    .concat();
    data.extend([0xA9; 64]);
    let usm = read_usm(&data).expect("movie");
    assert_eq!(usm.video[0].frames.len(), 1);
}

#[test]
fn movie_separates_frames_and_audio() {
    let data = movie();
    let usm = read_usm(&data).expect("movie");
    assert_eq!(usm.video.len(), 1);
    let video = &usm.video[0];
    assert_eq!(video.mpeg_codec, 1);
    assert_eq!((video.width, video.height), (640, 360));
    assert_eq!((video.framerate_n, video.framerate_d), (30000, 1000));
    let frames: Vec<&[u8]> = video.frames.iter().map(|f| f.data).collect();
    assert_eq!(frames, [&b"first"[..], b"second", b"third"]);
    assert_eq!(video.frames[1].time, 100);
    assert_eq!(video.frames[1].time_scale, 3000);
    let seek: Vec<bool> = video.frames.iter().map(|f| f.seek_point).collect();
    assert_eq!(seek, [true, false, true]);

    assert_eq!(usm.audio.len(), 1);
    let audio = &usm.audio[0];
    assert_eq!(audio.audio_codec, 2);
    assert_eq!((audio.sampling_rate, audio.num_channels), (48000, 2));
    assert_eq!(audio.chunks.concat(), b"AUDIOMORE");
}

#[test]
fn cut_movie_is_truncated() {
    let data = movie();
    assert!(matches!(
        read_usm(&data[..data.len() - 1]),
        Err(DecodeError::Truncated { .. })
    ));
    assert!(matches!(
        read_usm(b"RIFFxxxx"),
        Err(DecodeError::Malformed { .. })
    ));
}

#[test]
fn frame_before_its_header_is_malformed() {
    let data = [
        usm_chunk(b"CRID", 0, 1, 0, b""),
        usm_chunk(b"@SFV", 0, 0, 0, b"frame"),
    ]
    .concat();
    assert!(matches!(
        read_usm(&data),
        Err(DecodeError::Malformed { .. })
    ));
}

#[test]
fn cut_adx_is_truncated() {
    let data = fixture("sine_mono.adx");
    assert!(matches!(
        decode_adx(&data[..data.len() - 40]),
        Err(DecodeError::Truncated { .. })
    ));
}

#[test]
fn adx_of_another_encoding_type_is_unsupported() {
    let mut data = fixture("sine_mono.adx");
    data[4] = 4;
    assert!(matches!(
        decode_adx(&data),
        Err(DecodeError::Unsupported { .. })
    ));
}

/// A command list with a command that sets no volume and then the volumes `percents`.
fn volumes(percents: &[u16]) -> Vec<u8> {
    let mut list = vec![0x00, 0x41, 2, 0, 5];
    for percent in percents {
        list.extend([0x00, 0x57, 2]);
        list.extend(percent.to_be_bytes());
    }
    list
}

#[test]
fn a_cue_plays_at_the_product_of_the_volumes_on_its_way() {
    use Cell::{Data, F32, Text, U8, U16, U32};
    let commands =
        |name, percents: &[u16]| Data(utf(name, &["Command"], &[vec![Data(volumes(percents))]]));
    let data = utf(
        "Header",
        &[
            "AcbVolume",
            "CueTable",
            "CueNameTable",
            "WaveformTable",
            "SynthTable",
            "SequenceTable",
            "TrackTable",
            "TrackEventTable",
            "SeqCommandTable",
            "TrackCommandTable",
            "SynthCommandTable",
        ],
        &[vec![
            F32(0.5),
            Data(utf(
                "Cue",
                &["ReferenceType", "ReferenceIndex", "CueId", "Length"],
                &[
                    vec![U8(3), U16(0), U32(1), U32(0)],
                    vec![U8(1), U16(0), U32(2), U32(0)],
                ],
            )),
            Data(utf(
                "CueName",
                &["CueName", "CueIndex"],
                &[vec![Text("sequence"), U16(0)], vec![Text("direct"), U16(1)]],
            )),
            Data(utf("Waveform", &["EncodeType"], &[vec![U8(2)]])),
            Data(utf(
                "Synth",
                &["ReferenceItems", "CommandIndex"],
                &[vec![Data(vec![0, 1, 0, 0]), U16(0)]],
            )),
            Data(utf(
                "Sequence",
                &["NumTracks", "TrackIndex", "CommandIndex"],
                &[vec![U16(1), Data(vec![0, 0]), U16(0)]],
            )),
            Data(utf(
                "Track",
                &["EventIndex", "CommandIndex"],
                &[vec![U16(0), U16(0)]],
            )),
            Data(utf(
                "TrackEvent",
                &["Command"],
                &[vec![Data(vec![0x07, 0xD0, 4, 0, 2, 0, 0])]],
            )),
            commands("SeqCommand", &[70]),
            commands("TrackCommand", &[90]),
            commands("SynthCommand", &[80, 50]),
        ]],
    );
    let acb = read_acb(&data).expect("sheet");
    assert_eq!(acb.cues[0].waveforms, [0]);
    assert_eq!(acb.cues[0].volumes, [0.126]);
    assert_eq!(acb.cues[1].volumes, [0.5]);
}

#[test]
fn a_cue_built_of_tracks_gives_its_arrangement() {
    use Cell::{Data, F32, Text, U8, U16, U32};
    use sekai_codec::cri::{Arrangement, Block, Layer, Note, VolumeControl};
    let note = |waveform: u8| vec![0x07, 0xD0, 4, 0, 1, 0, waveform];
    // Waits of one second and of 500 microseconds between two notes.
    let wait = vec![0x07, 0xD1, 4, 0, 0, 3, 0xE8, 0x07, 0xD5, 2, 1, 0xF4];
    let late = [note(0), wait, note(1)].concat();
    let track =
        |event: u16, command: u16, local: Vec<u8>| vec![U16(event), U16(command), Data(local)];
    let cue = |kind: u8, index: u16, id: u32| vec![U8(kind), U16(index), U32(id), U32(0)];
    let name = |name, index: u16| vec![Text(name), U16(index)];
    let block = |tracks: Vec<u8>, length: u32, looping: u8, parts: u16| {
        vec![
            U16(tracks.len() as u16 / 2),
            Data(tracks),
            U32(length),
            U16(500 * u16::from(looping)),
            U8(looping),
            U16(2),
            U8(looping),
            U16(parts),
            U8(looping),
        ]
    };
    let data = utf(
        "Header",
        &[
            "AcbVolume",
            "CueTable",
            "CueNameTable",
            "WaveformTable",
            "SequenceTable",
            "BlockSequenceTable",
            "BlockTable",
            "TrackTable",
            "TrackEventTable",
            "TrackCommandTable",
            "AisacTable",
            "GraphTable",
            "AisacControlNameTable",
        ],
        &[vec![
            F32(0.5),
            Data(utf(
                "Cue",
                &["ReferenceType", "ReferenceIndex", "CueId", "Length"],
                &[cue(8, 0, 1), cue(3, 0, 2), cue(3, 1, 3)],
            )),
            Data(utf(
                "CueName",
                &["CueName", "CueIndex"],
                &[name("blocks", 0), name("layers", 1), name("plain", 2)],
            )),
            Data(utf(
                "Waveform",
                &["EncodeType"],
                &[vec![U8(2)], vec![U8(2)], vec![U8(2)]],
            )),
            Data(utf(
                "Sequence",
                &["NumTracks", "TrackIndex"],
                &[
                    vec![U16(2), Data(vec![0, 4, 0, 5])],
                    vec![U16(1), Data(vec![0, 5, 0, 0])],
                ],
            )),
            Data(utf(
                "BlockSequence",
                &["NumTracks", "TrackIndex", "NumBlocks", "BlockIndex"],
                &[vec![
                    U16(2),
                    Data(vec![0, 0, 0, 1]),
                    U16(2),
                    Data(vec![0, 0, 0, 1]),
                ]],
            )),
            Data(utf(
                "Block",
                &[
                    "NumTracks",
                    "TrackIndex",
                    "Length",
                    "LengthUs",
                    "PlaybackType",
                    "NumLoops",
                    "TransitionTiming",
                    "TransitionTimingValue",
                    "TransitionMode",
                ],
                &[
                    block(vec![0, 2, 0, 3], 2000, 1, 4),
                    block(vec![0, 3], 1000, 0, 0xFFFF),
                ],
            )),
            Data(utf(
                "Track",
                &["EventIndex", "CommandIndex", "LocalAisacs"],
                &[
                    track(0xFFFF, 0, vec![0, 0]),
                    track(0xFFFF, 0xFFFF, Vec::new()),
                    track(0, 0xFFFF, Vec::new()),
                    track(1, 0xFFFF, Vec::new()),
                    track(2, 0xFFFF, Vec::new()),
                    track(3, 0xFFFF, Vec::new()),
                ],
            )),
            Data(utf(
                "TrackEvent",
                &["Command"],
                &[
                    vec![Data(late)],
                    vec![Data(note(2))],
                    vec![Data(note(0))],
                    vec![Data(note(1))],
                ],
            )),
            Data(utf(
                "TrackCommand",
                &["Command"],
                &[vec![Data(volumes(&[50]))]],
            )),
            Data(utf(
                "Aisac",
                &[
                    "ControlId",
                    "DefaultControlFlag",
                    "DefaultControl",
                    "GraphIndexes",
                ],
                &[vec![U16(7), U8(1), F32(0.25), Data(vec![0, 1, 0, 0])]],
            )),
            Data(utf(
                "Graph",
                &["Type", "Controls", "Destinations"],
                &[
                    vec![
                        U8(1),
                        Data([0f32.to_be_bytes(), 1f32.to_be_bytes()].concat()),
                        Data(vec![0, 0, 0x13, 0x88]),
                    ],
                    vec![U8(2), Data(Vec::new()), Data(Vec::new())],
                ],
            )),
            Data(utf(
                "AisacControlName",
                &["AisacControlId", "AisacControlName"],
                &[vec![U16(7), Text("BGM_VERTICAL")]],
            )),
        ]],
    );
    let acb = read_acb(&data).expect("sheet");
    let at = |at_us, waveform, volume| Note {
        at_us,
        waveform,
        volume,
    };
    assert_eq!(acb.cues[0].waveforms, [0, 1, 2, 2]);
    assert_eq!(acb.cues[0].volumes, [0.25, 0.25, 0.5, 0.25]);
    assert_eq!(
        acb.cues[0].arrangement,
        Some(Arrangement {
            layers: vec![
                Layer {
                    control: Some(VolumeControl {
                        name: "BGM_VERTICAL".into(),
                        default: 0.25,
                        points: vec![[0.0, 0.0], [1.0, 0.5]],
                    }),
                    notes: Vec::new(),
                },
                Layer {
                    control: None,
                    notes: Vec::new(),
                },
            ],
            blocks: vec![
                Block {
                    length_us: 2_000_500,
                    looping: true,
                    repeats: 0,
                    transition_parts: Some(4),
                    relative: true,
                    layers: vec![
                        vec![at(0, 0, 0.25), at(1_000_500, 1, 0.25)],
                        vec![at(0, 2, 0.5)],
                    ],
                },
                Block {
                    length_us: 1_000_000,
                    looping: false,
                    repeats: 2,
                    transition_parts: None,
                    relative: false,
                    layers: vec![vec![at(0, 2, 0.25)]],
                },
            ],
        })
    );
    assert_eq!(
        acb.cues[1].arrangement,
        Some(Arrangement {
            layers: vec![
                Layer {
                    control: None,
                    notes: vec![at(0, 0, 0.5)],
                },
                Layer {
                    control: None,
                    notes: vec![at(0, 1, 0.5)],
                },
            ],
            blocks: Vec::new(),
        })
    );
    assert_eq!(acb.cues[2].arrangement, None);
}

/// The header of `data` with a `loop` chunk after its `comp` chunk, with a valid checksum.
fn with_loop(data: &[u8], chunk: [u32; 2], delays: [u16; 2]) -> Vec<u8> {
    const AFTER_COMP: usize = 40;
    let size = usize::from(u16::from_be_bytes([data[6], data[7]]));
    let mut out = data[..AFTER_COMP].to_vec();
    out.extend(b"loop");
    out.extend(chunk[0].to_be_bytes());
    out.extend(chunk[1].to_be_bytes());
    out.extend(delays[0].to_be_bytes());
    out.extend(delays[1].to_be_bytes());
    out.extend(&data[AFTER_COMP..size - 2]);
    let new_size = (size + 16) as u16;
    out[6..8].copy_from_slice(&new_size.to_be_bytes());
    let mut sum = 0u16;
    for &byte in &out {
        sum ^= u16::from(byte) << 8;
        for _ in 0..8 {
            sum = if sum & 0x8000 != 0 {
                (sum << 1) ^ 0x8005
            } else {
                sum << 1
            };
        }
    }
    out.extend(sum.to_be_bytes());
    out
}

#[test]
fn the_header_gives_the_loop_without_the_encoder_delay() {
    let data = fixture("sine_mono.hca");
    assert_eq!(hca_info(&data).unwrap().loop_frames, None);
    let delay = u16::from_be_bytes([data[20], data[21]]);
    let looping = with_loop(&data, [0, 1], [delay + 5, 7]);
    let end = 2 * 1024 - 7 - u64::from(delay);
    assert_eq!(hca_info(&looping).unwrap().loop_frames, Some([5, end]));
    let past_the_end = with_loop(&data, [0, 0xFFFF], [delay, 0]);
    assert_eq!(hca_info(&past_the_end).unwrap().loop_frames, None);
}

#[test]
fn the_header_gives_the_stored_length() {
    for name in ["sine_mono.hca", "sine_stereo.hca", "random_v3.hca"] {
        let data = fixture(name);
        let info = hca_info(&data).unwrap();
        let pcm = decode_hca(&data, None).unwrap();
        assert_eq!(info.channels, pcm.channels(), "{name}");
        assert_eq!(info.sample_rate, pcm.sample_rate(), "{name}");
        // The frames hold whole blocks of 1024 samples, including the encoder delay and padding.
        let decoded = (pcm.samples().len() / usize::from(pcm.channels())) as u64;
        assert!(
            info.frames >= decoded && info.frames.is_multiple_of(1024),
            "{name}"
        );
        assert_eq!(info.length, decoded, "{name}");
    }
    assert!(hca_info(b"not a stream").is_err());
}

#[test]
fn a_usm_chunk_with_an_overflowing_size_is_rejected() {
    let mut data = vec![0u8; 0x20];
    data[..4].copy_from_slice(b"CRID");
    data[4..8].copy_from_slice(&0xFFFF_FFF8u32.to_be_bytes());
    assert!(sekai_codec::cri::read_usm(&data).is_err());
}
