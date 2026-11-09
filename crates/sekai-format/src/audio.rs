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
