//! How a cue built of tracks plays them over time.

use super::{BLOCK_SEQUENCE, Percent, SEQUENCE, Table, Tables, be16, indices, require};
use crate::DecodeError;

/// The `PlaybackType` of a block that repeats until a transition is requested.
const LOOPING_PLAYBACK: i64 = 1;
/// The `TransitionTiming` of a block whose transitions wait for equal parts of it.
const AT_EQUAL_PARTS: i64 = 1;
/// The `TransitionTimingValue` that gives no number of parts.
const NO_PARTS: i64 = 0xFFFF;
/// The `TransitionMode` that carries the position in a block over to the next.
const RELATIVE_TRANSITION: i64 = 1;
/// The `Type` of the graph that drives the volume.
const VOLUME_GRAPH: i64 = 1;
/// The stored value of a graph point of volume 1.
const FULL_VOLUME: f32 = 10000.0;

/// How a cue plays its tracks together and, for a block sequence, one block after another.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrangement {
    /// The tracks of the cue, which play at the same time.
    pub layers: Vec<Layer>,
    /// The blocks of a block sequence, in order; empty for a sequence.
    pub blocks: Vec<Block>,
}

impl Arrangement {
    /// Whether the cue is one track whose waveforms all start at once, which the waveforms of the
    /// cue already say.
    fn is_plain(&self) -> bool {
        matches!(
            self.layers.as_slice(),
            [layer] if layer.control.is_none() && layer.notes.iter().all(|note| note.at_us == 0)
        )
    }
}

/// One track of a cue.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// What sets the volume of the layer while the cue plays.
    pub control: Option<VolumeControl>,
    /// The notes of a layer of a sequence. The notes of a block sequence are in its blocks.
    pub notes: Vec<Note>,
}

/// A curve from a named control value to a volume.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeControl {
    /// The name of the control.
    pub name: String,
    /// The value of the control until something sets it.
    pub default: f32,
    /// The curve as `[control, volume]`, linear between the points.
    pub points: Vec<[f32; 2]>,
}

/// One block of a block sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// The length in microseconds.
    pub length_us: u64,
    /// Whether the block repeats until a transition is requested.
    pub looping: bool,
    /// How many more times a block that does not loop plays.
    pub repeats: u32,
    /// The number of equal parts of the block a requested transition waits for the next of. A
    /// transition waits for the end of the block without one.
    pub transition_parts: Option<u32>,
    /// Whether the next block starts at the position this one has reached.
    pub relative: bool,
    /// The notes of each layer of the block.
    pub layers: Vec<Vec<Note>>,
}

/// One waveform a track starts.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// When the waveform starts, in microseconds from the start of its track.
    pub at_us: u64,
    /// The index of the waveform in the sheet.
    pub waveform: usize,
    /// The volume it plays at.
    pub volume: f32,
}

impl Tables<'_> {
    /// How the cue with the reference `kind` and `index` plays its tracks, for a cue with more to
    /// say than the waveforms it plays.
    pub(super) fn arrangement(
        &self,
        kind: u8,
        index: usize,
    ) -> Result<Option<Arrangement>, DecodeError> {
        match kind {
            SEQUENCE => self.sequence_arrangement(index),
            BLOCK_SEQUENCE => self.block_arrangement(index).map(Some),
            _ => Ok(None),
        }
    }

    fn sequence_arrangement(&self, index: usize) -> Result<Option<Arrangement>, DecodeError> {
        let table = require(&self.sequence, "SequenceTable")?;
        let volume = Self::times(Percent::ONE, self.sequence_commands.as_ref(), table, index)?;
        let arrangement = Arrangement {
            layers: self.layers(table, index, |track| self.notes(track, volume))?,
            blocks: Vec::new(),
        };
        Ok((!arrangement.is_plain()).then_some(arrangement))
    }

    fn block_arrangement(&self, index: usize) -> Result<Arrangement, DecodeError> {
        let table = require(&self.block_sequence, "BlockSequenceTable")?;
        let volume = Self::times(Percent::ONE, self.sequence_commands.as_ref(), table, index)?;
        let layer_volumes = self.layer_volumes(table, index, volume)?;
        let blocks = indices(table, index, "NumBlocks", "BlockIndex")?
            .into_iter()
            .map(|block| self.block(block, &layer_volumes))
            .collect::<Result<_, DecodeError>>()?;
        Ok(Arrangement {
            layers: self.layers(table, index, |_| Ok(Vec::new()))?,
            blocks,
        })
    }

    /// The layers of the tracks of row `index` of `table`, with the notes `notes` gives each.
    fn layers(
        &self,
        table: &Table<'_>,
        index: usize,
        notes: impl Fn(usize) -> Result<Vec<Note>, DecodeError>,
    ) -> Result<Vec<Layer>, DecodeError> {
        indices(table, index, "NumTracks", "TrackIndex")?
            .into_iter()
            .map(|track| {
                Ok(Layer {
                    control: self.volume_control(track)?,
                    notes: notes(track)?,
                })
            })
            .collect()
    }

    fn block(&self, index: usize, layer_volumes: &[Percent]) -> Result<Block, DecodeError> {
        let table = require(&self.block, "BlockTable")?;
        let number = |name: &str| table.int(index, name).map(|value| value.unwrap_or(0));
        let looping = number("PlaybackType")? == LOOPING_PLAYBACK;
        let parts = number("TransitionTimingValue")?;
        Ok(Block {
            length_us: number("Length")? as u64 * 1000 + number("LengthUs")? as u64,
            looping,
            repeats: if looping {
                0
            } else {
                number("NumLoops")? as u32
            },
            transition_parts: (number("TransitionTiming")? == AT_EQUAL_PARTS
                && (1..NO_PARTS).contains(&parts))
            .then_some(parts as u32),
            relative: number("TransitionMode")? == RELATIVE_TRANSITION,
            layers: self
                .block_tracks(index, layer_volumes)?
                .into_iter()
                .map(|(track, volume)| self.notes(track, volume))
                .collect::<Result<_, DecodeError>>()?,
        })
    }

    /// The notes of track `track`, played under `volume`.
    fn notes(&self, track: usize, volume: Percent) -> Result<Vec<Note>, DecodeError> {
        Ok(self
            .track_notes(track, 0, volume)?
            .into_iter()
            .map(|note| Note {
                at_us: note.at_us,
                waveform: note.waveform,
                volume: self.level(note.volume),
            })
            .collect())
    }

    /// The control of the first local AISAC of track `track`, when that AISAC has a volume graph.
    fn volume_control(&self, track: usize) -> Result<Option<VolumeControl>, DecodeError> {
        let tracks = require(&self.track, "TrackTable")?;
        let local = tracks.data(track, "LocalAisacs")?.unwrap_or(&[]);
        let Some(first) = local.as_chunks::<2>().0.first() else {
            return Ok(None);
        };
        let aisacs = require(&self.aisac, "AisacTable")?;
        let aisac = be16(first);
        let Some(points) =
            self.volume_points(aisacs.data(aisac, "GraphIndexes")?.unwrap_or(&[]))?
        else {
            return Ok(None);
        };
        let default = match aisacs.int(aisac, "DefaultControlFlag")? {
            Some(1) => aisacs.float(aisac, "DefaultControl")?.unwrap_or(0.0),
            _ => 0.0,
        };
        Ok(Some(VolumeControl {
            name: self.control_name(aisacs.int(aisac, "ControlId")?)?,
            default,
            points,
        }))
    }

    /// The points of the volume graph among the graphs in `indexes`.
    fn volume_points(&self, indexes: &[u8]) -> Result<Option<Vec<[f32; 2]>>, DecodeError> {
        let graphs = require(&self.graph, "GraphTable")?;
        for index in indexes.as_chunks::<2>().0 {
            let graph = be16(index);
            if graphs.int(graph, "Type")? == Some(VOLUME_GRAPH) {
                let controls = graphs.data(graph, "Controls")?.unwrap_or(&[]);
                let volumes = graphs.data(graph, "Destinations")?.unwrap_or(&[]);
                let points = controls
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(volumes.as_chunks::<2>().0);
                return Ok(Some(
                    points
                        .map(|(control, volume)| {
                            [
                                f32::from_be_bytes(*control),
                                be16(volume) as f32 / FULL_VOLUME,
                            ]
                        })
                        .collect(),
                ));
            }
        }
        Ok(None)
    }

    /// The name of the control numbered `id`, empty when the sheet names none.
    fn control_name(&self, id: Option<i64>) -> Result<String, DecodeError> {
        let names = require(&self.control_names, "AisacControlNameTable")?;
        for row in 0..names.rows() {
            if names.int(row, "AisacControlId")? == id {
                return Ok(names
                    .text(row, "AisacControlName")?
                    .unwrap_or("")
                    .to_owned());
            }
        }
        Ok(String::new())
    }
}
