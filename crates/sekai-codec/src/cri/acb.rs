//! ACB cue sheets.

mod arrangement;

use std::collections::BTreeMap;

use super::awb::{Awb, read_awb};
use super::utf::Table;
use crate::DecodeError;

pub use arrangement::{Arrangement, Block, Layer, Note, VolumeControl};

/// How deeply the objects of a cue may refer to each other.
const MAX_DEPTH: usize = 8;
const NO_INDEX: i64 = 0xFFFF;
const NOTE_ON: u16 = 2000;
const NOTE_ON_WITH_NO: u16 = 2003;
/// A wait before the next command: a 32-bit value in milliseconds.
const WAIT_MILLISECONDS: u16 = 2001;
/// A wait before the next command: a 16-bit value in microseconds.
const WAIT_MICROSECONDS: u16 = 2005;
/// The reference to a sequence.
const SEQUENCE: u8 = 3;
/// The reference to a block sequence.
const BLOCK_SEQUENCE: u8 = 8;
/// A volume command: a 16-bit value in hundredths.
const VOLUME: u16 = 0x0057;

fn malformed(message: impl Into<String>) -> DecodeError {
    DecodeError::Malformed {
        what: "ACB",
        message: message.into(),
    }
}

fn be16(bytes: &[u8]) -> usize {
    usize::from(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn be32(bytes: &[u8]) -> u64 {
    u64::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
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
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    /// The names given to the cue.
    pub cue_names: Vec<String>,
    /// The `CueId` value.
    pub cue_id: u32,
    /// The length in milliseconds.
    pub length: u32,
    /// Indices into [`Acb::waveforms`] of everything the cue can play.
    pub waveforms: Vec<usize>,
    /// The volume each entry of `waveforms` plays at: the volume of the cue sheet times every
    /// volume command on the way from the cue to the waveform.
    pub volumes: Vec<f32>,
    /// How the cue plays its tracks, when it has more to say than the waveforms it plays.
    pub arrangement: Option<Arrangement>,
}

/// A cue sheet.
#[derive(Debug, Clone, PartialEq)]
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
    synth_commands: Option<Table<'a>>,
    sequence_commands: Option<Table<'a>>,
    track_commands: Option<Table<'a>>,
    aisac: Option<Table<'a>>,
    graph: Option<Table<'a>>,
    control_names: Option<Table<'a>>,
    /// The `AcbVolume` of the sheet.
    sheet_volume: f64,
}

/// A volume as a product of percentages, exact until it is read.
#[derive(Debug, Clone, Copy)]
struct Percent {
    product: u64,
    factors: i32,
}

impl Percent {
    const ONE: Self = Self {
        product: 1,
        factors: 0,
    };

    fn value(self) -> f64 {
        self.product as f64 / 100_f64.powi(self.factors)
    }
}

/// Calls `visit` with the code and the value of each command of a command list.
fn each_command(
    list: &[u8],
    mut visit: impl FnMut(u16, &[u8]) -> Result<(), DecodeError>,
) -> Result<(), DecodeError> {
    let mut position = 0;
    while position + 3 <= list.len() {
        let code = be16(&list[position..]) as u16;
        let size = usize::from(list[position + 2]);
        position += 3;
        let value = list
            .get(position..position + size)
            .ok_or_else(|| malformed("a command lies past its data"))?;
        visit(code, value)?;
        position += size;
    }
    Ok(())
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

type Played = Vec<(usize, Percent)>;

/// A waveform a track starts, with the volume on its way there.
struct TrackNote {
    at_us: u64,
    waveform: usize,
    volume: Percent,
}

impl Tables<'_> {
    /// `volume` as the volume a waveform plays at.
    fn level(&self, volume: Percent) -> f32 {
        (self.sheet_volume * volume.value()) as f32
    }

    /// `volume` times the volume commands of row `row` of `table`, whose `CommandIndex` is a row
    /// of `commands`.
    fn times(
        volume: Percent,
        commands: Option<&Table<'_>>,
        table: &Table<'_>,
        row: usize,
    ) -> Result<Percent, DecodeError> {
        let index = table.int(row, "CommandIndex")?.unwrap_or(NO_INDEX);
        let (Some(commands), true) = (commands, index != NO_INDEX) else {
            return Ok(volume);
        };
        let list = commands.data(index as usize, "Command")?.unwrap_or(&[]);
        let mut volume = volume;
        each_command(list, |code, value| {
            if code == VOLUME && value.len() == 2 {
                volume.product = volume.product.saturating_mul(be16(value) as u64);
                volume.factors += 1;
            }
            Ok(())
        })?;
        Ok(volume)
    }

    fn synth(
        &self,
        index: usize,
        depth: usize,
        volume: Percent,
        out: &mut Played,
    ) -> Result<(), DecodeError> {
        let table = require(&self.synth, "SynthTable")?;
        let volume = Self::times(volume, self.synth_commands.as_ref(), table, index)?;
        let items = table.data(index, "ReferenceItems")?.unwrap_or(&[]);
        for item in items.as_chunks::<4>().0 {
            self.reference(be16(item) as u8, be16(&item[2..]), depth + 1, volume, out)?;
        }
        Ok(())
    }

    fn sequence(
        &self,
        index: usize,
        depth: usize,
        volume: Percent,
        out: &mut Played,
    ) -> Result<(), DecodeError> {
        let table = require(&self.sequence, "SequenceTable")?;
        let volume = Self::times(volume, self.sequence_commands.as_ref(), table, index)?;
        self.tracks(table, index, depth, volume, out)
    }

    /// A block sequence plays its own tracks, then the tracks of each of its blocks.
    fn block_sequence(
        &self,
        index: usize,
        depth: usize,
        volume: Percent,
        out: &mut Played,
    ) -> Result<(), DecodeError> {
        let table = require(&self.block_sequence, "BlockSequenceTable")?;
        let volume = Self::times(volume, self.sequence_commands.as_ref(), table, index)?;
        self.tracks(table, index, depth, volume, out)?;
        let layer_volumes = self.layer_volumes(table, index, volume)?;
        for block in indices(table, index, "NumBlocks", "BlockIndex")? {
            for (track, layer_volume) in self.block_tracks(block, &layer_volumes)? {
                self.track(track, depth + 2, layer_volume, out)?;
            }
        }
        Ok(())
    }

    /// `volume` times the volume commands of each track of row `index` of a block sequence.
    fn layer_volumes(
        &self,
        table: &Table<'_>,
        index: usize,
        volume: Percent,
    ) -> Result<Vec<Percent>, DecodeError> {
        let tracks = require(&self.track, "TrackTable")?;
        indices(table, index, "NumTracks", "TrackIndex")?
            .into_iter()
            .map(|track| Self::times(volume, self.track_commands.as_ref(), tracks, track))
            .collect()
    }

    /// The tracks of block `block`, each with the volume of the layer it plays in.
    fn block_tracks(
        &self,
        block: usize,
        layer_volumes: &[Percent],
    ) -> Result<Vec<(usize, Percent)>, DecodeError> {
        let table = require(&self.block, "BlockTable")?;
        let tracks = indices(table, block, "NumTracks", "TrackIndex")?;
        if tracks.len() > layer_volumes.len() {
            return Err(malformed("a block has more tracks than its sequence"));
        }
        Ok(tracks
            .into_iter()
            .zip(layer_volumes.iter().copied())
            .collect())
    }

    /// The tracks listed by row `index` of `table` in `NumTracks` and `TrackIndex`.
    fn tracks(
        &self,
        table: &Table<'_>,
        index: usize,
        depth: usize,
        volume: Percent,
        out: &mut Played,
    ) -> Result<(), DecodeError> {
        for track in indices(table, index, "NumTracks", "TrackIndex")? {
            self.track(track, depth + 1, volume, out)?;
        }
        Ok(())
    }

    fn track(
        &self,
        index: usize,
        depth: usize,
        volume: Percent,
        out: &mut Played,
    ) -> Result<(), DecodeError> {
        let notes = self.track_notes(index, depth, volume)?;
        out.extend(notes.into_iter().map(|note| (note.waveform, note.volume)));
        Ok(())
    }

    /// What track `index` plays, in the order of its commands. A wait delays what follows it.
    fn track_notes(
        &self,
        index: usize,
        depth: usize,
        volume: Percent,
    ) -> Result<Vec<TrackNote>, DecodeError> {
        let table = require(&self.track, "TrackTable")?;
        let volume = Self::times(volume, self.track_commands.as_ref(), table, index)?;
        let event = table.int(index, "EventIndex")?.unwrap_or(NO_INDEX);
        if event == NO_INDEX {
            return Ok(Vec::new());
        }
        let events = require(&self.event, "TrackEventTable")?;
        let command = events.data(event as usize, "Command")?.unwrap_or(&[]);
        let (mut at_us, mut notes) = (0, Vec::new());
        each_command(command, |code, value| {
            match code {
                NOTE_ON | NOTE_ON_WITH_NO if value.len() >= 4 => {
                    let (kind, target) = (be16(value) as u8, be16(&value[2..]));
                    let mut played = Vec::new();
                    self.reference(kind, target, depth + 1, volume, &mut played)?;
                    notes.extend(played.into_iter().map(|(waveform, volume)| TrackNote {
                        at_us,
                        waveform,
                        volume,
                    }));
                }
                WAIT_MILLISECONDS if value.len() >= 4 => at_us += be32(value) * 1000,
                WAIT_MICROSECONDS if value.len() >= 2 => at_us += be16(value) as u64,
                _ => {}
            }
            Ok(())
        })?;
        Ok(notes)
    }

    /// Adds what a reference leads to. Kinds that hold no waveform are skipped.
    fn reference(
        &self,
        kind: u8,
        index: usize,
        depth: usize,
        volume: Percent,
        out: &mut Played,
    ) -> Result<(), DecodeError> {
        if depth > MAX_DEPTH {
            return Err(malformed("references are nested too deeply"));
        }
        match kind {
            1 => out.push((index, volume)),
            2 => self.synth(index, depth, volume, out)?,
            SEQUENCE => self.sequence(index, depth, volume, out)?,
            BLOCK_SEQUENCE => self.block_sequence(index, depth, volume, out)?,
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
    let extensions = header.table(0, "WaveformExtensionDataTable")?;
    // The waveform table of a sheet whose cues play nothing is empty.
    let waveforms = match header.table(0, "WaveformTable")? {
        Some(table) => read_waveforms(&table, extensions.as_ref())?,
        None => Vec::new(),
    };
    let event = match header.table(0, "TrackEventTable")? {
        Some(table) => Some(table),
        None => header.table(0, "CommandTable")?,
    };
    let commands = |name: &str| match header.table(0, name)? {
        Some(table) => Ok(Some(table)),
        None => header.table(0, "CommandTable"),
    };
    let tables = Tables {
        sheet_volume: f64::from(header.float(0, "AcbVolume")?.unwrap_or(1.0)),
        aisac: header.table(0, "AisacTable")?,
        graph: header.table(0, "GraphTable")?,
        control_names: header.table(0, "AisacControlNameTable")?,
        synth_commands: commands("SynthCommandTable")?,
        sequence_commands: commands("SeqCommandTable")?,
        track_commands: commands("TrackCommandTable")?,
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
        tables.reference(kind, index, 0, Percent::ONE, &mut played)?;
        if let Some(&(bad, _)) = played.iter().find(|&&(w, _)| w >= waveforms.len()) {
            return Err(malformed(format!("a cue refers to waveform {bad}")));
        }
        cues.push(Cue {
            cue_names: names.remove(&row).unwrap_or_default(),
            cue_id: cue_table.int(row, "CueId")?.unwrap_or(0) as u32,
            length: cue_table.int(row, "Length")?.unwrap_or(0) as u32,
            waveforms: played.iter().map(|&(waveform, _)| waveform).collect(),
            volumes: played
                .iter()
                .map(|&(_, volume)| tables.level(volume))
                .collect(),
            arrangement: tables.arrangement(kind, index)?,
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
