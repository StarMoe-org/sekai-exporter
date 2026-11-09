//! The cue table of the `audio` form, specified in `docs/spec/audio.md`.

use serde::{Deserialize, Serialize};

/// The value of the `format` key.
pub const FORMAT: &str = "audio-cues";
/// The version of the document format.
pub const VERSION: u32 = 1;

/// How the cues of a cue sheet play their waveforms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueTable {
    /// Always [`FORMAT`].
    pub format: String,
    /// The version of the document format.
    pub version: u32,
    /// The cues, in the order of the cue sheet.
    pub cues: Vec<CueEntry>,
}

impl CueTable {
    /// The bytes of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Reads a document.
    pub fn from_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

/// One cue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueEntry {
    /// The name of the cue.
    pub name: String,
    /// The waveforms the cue plays, in the order of its members.
    pub waveforms: Vec<CueWaveform>,
    /// How a cue built of tracks plays them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrangement: Option<CueArrangement>,
}

/// One waveform of a cue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueWaveform {
    /// The role of the member that holds the samples.
    pub role: String,
    /// The length in samples per channel.
    pub frames: u64,
    /// The sample rate in hertz.
    pub sample_rate: u32,
    /// The channel count.
    pub channels: u16,
    /// The loop of a waveform that loops, as `[start, end)` in samples per channel.
    #[serde(rename = "loop", default, skip_serializing_if = "Option::is_none")]
    pub loop_frames: Option<[u64; 2]>,
    /// The volume the cue plays the waveform at, 1 for its stored level.
    pub volume: f32,
}

/// How a cue plays its tracks together and, for a block sequence, one block after another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueArrangement {
    /// The tracks of the cue, which play at the same time.
    pub layers: Vec<CueLayer>,
    /// The blocks of a block sequence, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<CueBlock>,
}

/// One track of a cue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueLayer {
    /// What sets the volume of the layer while the cue plays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<CueVolumeControl>,
    /// The notes of a layer of a sequence. The notes of a block sequence are in its blocks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<CueNote>,
}

/// A curve from a named control value to a volume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueVolumeControl {
    /// The name of the control.
    pub name: String,
    /// The value of the control until something sets it.
    pub default: f32,
    /// The curve as `[control, volume]`, linear between the points.
    pub points: Vec<[f32; 2]>,
}

/// One block of a block sequence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueBlock {
    /// The length in microseconds.
    pub length_us: u64,
    /// Whether the block repeats until a transition is requested.
    #[serde(rename = "loop")]
    pub looping: bool,
    /// How many more times a block that does not loop plays.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub repeats: u32,
    /// The number of equal parts of the block a requested transition waits for the next of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_parts: Option<u32>,
    /// Whether the next block starts at the position this one has reached.
    pub relative: bool,
    /// The notes of each layer of the block.
    pub layers: Vec<Vec<CueNote>>,
}

/// One waveform a track starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueNote {
    /// When the waveform starts, in microseconds from the start of its track.
    pub at_us: u64,
    /// The role of the member that holds the samples.
    pub role: String,
    /// The volume it plays at.
    pub volume: f32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> CueTable {
        CueTable {
            format: FORMAT.to_owned(),
            version: VERSION,
            cues: vec![CueEntry {
                name: "a".to_owned(),
                waveforms: vec![CueWaveform {
                    role: "cue/a".to_owned(),
                    frames: 100,
                    sample_rate: 44100,
                    channels: 2,
                    loop_frames: Some([10, 90]),
                    volume: 0.63,
                }],
                arrangement: None,
            }],
        }
    }

    #[test]
    fn the_bytes_are_fixed() {
        assert_eq!(
            String::from_utf8(table().to_bytes().unwrap()).unwrap(),
            r#"{"format":"audio-cues","version":1,"cues":[{"name":"a","waveforms":[{"role":"cue/a","frames":100,"sample_rate":44100,"channels":2,"loop":[10,90],"volume":0.63}]}]}"#
        );
    }

    #[test]
    fn the_bytes_of_an_arrangement_are_fixed() {
        let mut table = table();
        let note = CueNote {
            at_us: 500,
            role: "cue/a".to_owned(),
            volume: 0.5,
        };
        table.cues[0].arrangement = Some(CueArrangement {
            layers: vec![CueLayer {
                control: Some(CueVolumeControl {
                    name: "c".to_owned(),
                    default: 0.25,
                    points: vec![[0.0, 1.0]],
                }),
                notes: Vec::new(),
            }],
            blocks: vec![CueBlock {
                length_us: 1000,
                looping: true,
                repeats: 0,
                transition_parts: Some(4),
                relative: false,
                layers: vec![vec![note]],
            }],
        });
        let bytes = table.to_bytes().unwrap();
        assert!(
            String::from_utf8_lossy(&bytes).contains(
                r#""arrangement":{"layers":[{"control":{"name":"c","default":0.25,"points":[[0.0,1.0]]}}],"blocks":[{"length_us":1000,"loop":true,"transition_parts":4,"relative":false,"layers":[[{"at_us":500,"role":"cue/a","volume":0.5}]]}]}"#
            )
        );
        assert_eq!(CueTable::from_bytes(&bytes).unwrap(), table);
    }

    #[test]
    fn a_table_reads_back_and_a_waveform_need_not_loop() {
        let mut table = table();
        assert_eq!(
            CueTable::from_bytes(&table.to_bytes().unwrap()).unwrap(),
            table
        );
        table.cues[0].waveforms[0].loop_frames = None;
        let bytes = table.to_bytes().unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("loop"));
        assert_eq!(CueTable::from_bytes(&bytes).unwrap(), table);
    }
}
