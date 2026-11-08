//! ACB cue sheets.

use std::collections::BTreeMap;

use super::awb::{Awb, read_awb};
use super::utf::Table;
use crate::DecodeError;

/// How deeply the objects of a cue may refer to each other.
const MAX_DEPTH: usize = 8;
const NO_INDEX: i64 = 0xFFFF;
const NOTE_ON: u16 = 2000;
const NOTE_ON_WITH_NO: u16 = 2003;

fn malformed(message: impl Into<String>) -> DecodeError {
    DecodeError::Malformed {
        what: "ACB",
        message: message.into(),
    }
}

fn be16(bytes: &[u8]) -> usize {
    usize::from(u16::from_be_bytes([bytes[0], bytes[1]]))
}

/// One waveform of a cue sheet, with the values of the `WaveformTable` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waveform {
    /// 0 when the waveform is in the memory bank, 1 when in the stream bank, 2 for both.
    pub streaming: u8,
    /// The wave ID in the memory bank.
    pub memory_awb_id: Option<u32>,
    /// The wave ID in the stream bank.
    pub stream_awb_id: Option<u32>,
    /// The codec, as CRI numbers it.
    pub encode_type: u8,
    /// Nonzero if the waveform loops.
    pub loop_flag: u8,
    /// The channel count.
    pub num_channels: u8,
    /// The sample rate in hertz.
    pub sampling_rate: u32,
    /// The length in samples per channel.
    pub num_samples: u32,
    /// The first sample of the loop, when the sheet states it.
    pub loop_start: Option<u32>,
    /// The sample after the loop, when the sheet states it.
    pub loop_end: Option<u32>,
}

/// One cue of a cue sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    /// The names given to the cue.
    pub cue_names: Vec<String>,
    /// The `CueId` value.
    pub cue_id: u32,
    /// The length in milliseconds.
    pub length: u32,
    /// Indices into [`Acb::waveforms`] of everything the cue can play.
    pub waveforms: Vec<usize>,
}

/// A cue sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acb<'a> {
    /// The cues in the order of the `CueTable`.
    pub cues: Vec<Cue>,
    /// The waveforms in the order of the `WaveformTable`.
    pub waveforms: Vec<Waveform>,
    /// The bank stored inside the sheet.
    pub memory_awb: Option<Awb<'a>>,
}

struct Tables<'a> {
    synth: Option<Table<'a>>,
    sequence: Option<Table<'a>>,
    block_sequence: Option<Table<'a>>,
    block: Option<Table<'a>>,
    track: Option<Table<'a>>,
    event: Option<Table<'a>>,
}

fn require<'t, 'a>(table: &'t Option<Table<'a>>, name: &str) -> Result<&'t Table<'a>, DecodeError> {
    table
        .as_ref()
        .ok_or_else(|| malformed(format!("a cue refers to the missing {name}")))
}

/// The first `count` of the 16-bit indices in column `list` of row `row`, where `count` is
/// column `count` of the same row.
fn indices(
    table: &Table<'_>,
    row: usize,
    count: &str,
    list: &str,
) -> Result<Vec<usize>, DecodeError> {
    let n = table.int(row, count)?.unwrap_or(0) as usize;
    let values = table.data(row, list)?.unwrap_or(&[]);
    if values.len() < n * 2 {
        return Err(malformed(format!("{list} has fewer than {count} entries")));
    }
    Ok(values.as_chunks::<2>().0[..n]
        .iter()
        .map(|v| be16(v))
        .collect())
}

impl Tables<'_> {
    fn synth(&self, index: usize, depth: usize, out: &mut Vec<usize>) -> Result<(), DecodeError> {
        let table = require(&self.synth, "SynthTable")?;
        let items = table.data(index, "ReferenceItems")?.unwrap_or(&[]);
        for item in items.as_chunks::<4>().0 {
            self.reference(be16(item) as u8, be16(&item[2..]), depth + 1, out)?;
        }
        Ok(())
    }

    fn sequence(
        &self,
        index: usize,
        depth: usize,
        out: &mut Vec<usize>,
    ) -> Result<(), DecodeError> {
        let table = require(&self.sequence, "SequenceTable")?;
        self.tracks(table, index, depth, out)
    }

    /// A block sequence plays its own tracks, then the tracks of each of its blocks.
    fn block_sequence(
        &self,
        index: usize,
        depth: usize,
        out: &mut Vec<usize>,
    ) -> Result<(), DecodeError> {
        let table = require(&self.block_sequence, "BlockSequenceTable")?;
        self.tracks(table, index, depth, out)?;
        for block in indices(table, index, "NumBlocks", "BlockIndex")? {
            let blocks = require(&self.block, "BlockTable")?;
            self.tracks(blocks, block, depth + 1, out)?;
        }
        Ok(())
    }

    /// The tracks listed by row `index` of `table` in `NumTracks` and `TrackIndex`.
    fn tracks(
        &self,
        table: &Table<'_>,
        index: usize,
        depth: usize,
        out: &mut Vec<usize>,
    ) -> Result<(), DecodeError> {
        for track in indices(table, index, "NumTracks", "TrackIndex")? {
            self.track(track, depth + 1, out)?;
        }
        Ok(())
    }

    fn track(&self, index: usize, depth: usize, out: &mut Vec<usize>) -> Result<(), DecodeError> {
        let table = require(&self.track, "TrackTable")?;
        let event = table.int(index, "EventIndex")?.unwrap_or(NO_INDEX);
        if event == NO_INDEX {
            return Ok(());
        }
        let events = require(&self.event, "TrackEventTable")?;
        let command = events.data(event as usize, "Command")?.unwrap_or(&[]);
        let mut position = 0;
        while position + 3 <= command.len() {
            let code = be16(&command[position..]) as u16;
            let size = usize::from(command[position + 2]);
            position += 3;
            let value = command
                .get(position..position + size)
                .ok_or_else(|| malformed("a track command lies past its data"))?;
            if matches!(code, NOTE_ON | NOTE_ON_WITH_NO) && size >= 4 {
                self.reference(be16(value) as u8, be16(&value[2..]), depth + 1, out)?;
            }
            position += size;
        }
        Ok(())
    }

    /// Adds what a reference leads to. Kinds that hold no waveform are skipped.
    fn reference(
        &self,
        kind: u8,
        index: usize,
        depth: usize,
        out: &mut Vec<usize>,
    ) -> Result<(), DecodeError> {
        if depth > MAX_DEPTH {
            return Err(malformed("references are nested too deeply"));
        }
        match kind {
            1 => out.push(index),
            2 => self.synth(index, depth, out)?,
            3 => self.sequence(index, depth, out)?,
            8 => self.block_sequence(index, depth, out)?,
            _ => {}
        }
        Ok(())
    }
}

fn read_waveforms(
    table: &Table<'_>,
    extensions: Option<&Table<'_>>,
) -> Result<Vec<Waveform>, DecodeError> {
    let id = |row: usize, name: &str| -> Result<Option<u32>, DecodeError> {
        Ok(table
            .int(row, name)?
            .filter(|&value| value != NO_INDEX)
            .map(|value| value as u32))
    };
    let mut waveforms = Vec::with_capacity(table.rows());
    for row in 0..table.rows() {
        let streaming = table.int(row, "Streaming")?.unwrap_or(0) as u8;
        let older_id = id(row, "Id")?;
        let memory_awb_id = id(row, "MemoryAwbId")?.or(older_id.filter(|_| streaming == 0));
        let stream_awb_id = id(row, "StreamAwbId")?.or(older_id.filter(|_| streaming != 0));
        let extension = table
            .int(row, "ExtensionData")?
            .filter(|&value| value != NO_INDEX);
        let (mut loop_start, mut loop_end) = (None, None);
        if let (Some(extensions), Some(index)) = (extensions, extension) {
            loop_start = extensions
                .int(index as usize, "LoopStart")?
                .map(|v| v as u32);
            loop_end = extensions.int(index as usize, "LoopEnd")?.map(|v| v as u32);
        }
        waveforms.push(Waveform {
            streaming,
            memory_awb_id,
            stream_awb_id,
            encode_type: table.int(row, "EncodeType")?.unwrap_or(0) as u8,
            loop_flag: table.int(row, "LoopFlag")?.unwrap_or(0) as u8,
            num_channels: table.int(row, "NumChannels")?.unwrap_or(0) as u8,
            sampling_rate: table.int(row, "SamplingRate")?.unwrap_or(0) as u32,
            num_samples: table.int(row, "NumSamples")?.unwrap_or(0) as u32,
            loop_start,
            loop_end,
        });
    }
    Ok(waveforms)
}

/// Reads a cue sheet with its memory bank.
pub fn read_acb(data: &[u8]) -> Result<Acb<'_>, DecodeError> {
    let header = Table::open(data)?;
    if header.rows() != 1 {
        return Err(malformed("the header table does not have one row"));
    }
    let cue_table = header
        .table(0, "CueTable")?
        .ok_or_else(|| malformed("no CueTable"))?;
    let waveform_table = header
        .table(0, "WaveformTable")?
        .ok_or_else(|| malformed("no WaveformTable"))?;
    let extensions = header.table(0, "WaveformExtensionDataTable")?;
    let waveforms = read_waveforms(&waveform_table, extensions.as_ref())?;
    let event = match header.table(0, "TrackEventTable")? {
        Some(table) => Some(table),
        None => header.table(0, "CommandTable")?,
    };
    let tables = Tables {
        synth: header.table(0, "SynthTable")?,
        sequence: header.table(0, "SequenceTable")?,
        block_sequence: header.table(0, "BlockSequenceTable")?,
        block: header.table(0, "BlockTable")?,
        track: header.table(0, "TrackTable")?,
        event,
    };

    let mut names: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    if let Some(table) = header.table(0, "CueNameTable")? {
        for row in 0..table.rows() {
            let index = table.int(row, "CueIndex")?.unwrap_or(0) as usize;
            let name = table.text(row, "CueName")?.unwrap_or("");
            names.entry(index).or_default().push(name.to_string());
        }
    }

    let mut cues = Vec::with_capacity(cue_table.rows());
    for row in 0..cue_table.rows() {
        let mut played = Vec::new();
        let kind = cue_table.int(row, "ReferenceType")?.unwrap_or(0) as u8;
        let index = cue_table.int(row, "ReferenceIndex")?.unwrap_or(0) as usize;
        tables.reference(kind, index, 0, &mut played)?;
        if let Some(&bad) = played.iter().find(|&&w| w >= waveforms.len()) {
            return Err(malformed(format!("a cue refers to waveform {bad}")));
        }
        cues.push(Cue {
            cue_names: names.remove(&row).unwrap_or_default(),
            cue_id: cue_table.int(row, "CueId")?.unwrap_or(0) as u32,
            length: cue_table.int(row, "Length")?.unwrap_or(0) as u32,
            waveforms: played,
        });
    }

    let memory_awb = match header.data(0, "AwbFile")? {
        Some(bytes) if !bytes.is_empty() => Some(read_awb(bytes)?),
        _ => None,
    };
    Ok(Acb {
        cues,
        waveforms,
        memory_awb,
    })
}
