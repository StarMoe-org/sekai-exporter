//! Clip documents, specified in `docs/spec/clip-documents.md`.

use serde::{Deserialize, Serialize};

/// The value of the `format` key.
pub const FORMAT: &str = "clip";
/// The version of the document format.
pub const VERSION: u32 = 1;

/// The curves, bindings and events of one animation clip, as the engine stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipDocument {
    /// Always [`FORMAT`].
    pub format: String,
    /// The version of the document format.
    pub version: u32,
    /// The clip's name.
    pub name: String,
    /// Frames per second.
    pub sample_rate: f32,
    /// The clip time at which playback starts, in seconds.
    pub start_time: f32,
    /// The clip time at which playback stops, in seconds.
    pub stop_time: f32,
    /// Whether the clip starts again at its end.
    #[serde(rename = "loop")]
    pub looping: bool,
    /// One curve per binding, in stored order.
    pub curves: Vec<Curve>,
    /// The events, in stored order.
    pub events: Vec<Event>,
}

impl ClipDocument {
    /// The bytes of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Reads a document.
    pub fn from_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

/// One binding and the curve that drives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    /// The CRC-32 of the path of the target.
    pub path: u32,
    /// The CRC-32 of the property name.
    pub attribute: u32,
    /// The class of the component the curve drives.
    pub class_id: i32,
    /// The script of a script component, as `"<assembly>/<namespace>.<class>"`.
    pub script: Option<String>,
    /// The binding's custom type.
    pub custom_type: u8,
    /// The curve.
    #[serde(flatten)]
    pub data: CurveData,
}

/// The values of a curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CurveData {
    /// Cubic segments `[t0, a, b, c, d]` by increasing `t0`.
    Streamed {
        /// The segments.
        segments: Vec<[f32; 5]>,
    },
    /// Samples at a fixed rate, interpolated linearly.
    Dense {
        /// The time of the first sample.
        begin_time: f32,
        /// Samples per second.
        sample_rate: f32,
        /// The samples.
        samples: Vec<f32>,
    },
    /// One value.
    Constant {
        /// The value.
        value: f32,
    },
}

impl CurveData {
    /// The value at clip time `time`, computed as the specification states, in 32-bit
    /// arithmetic.
    pub fn evaluate(&self, time: f32) -> f32 {
        match self {
            Self::Constant { value } => *value,
            Self::Streamed { segments } => {
                let index = segments.partition_point(|s| s[0] <= time).saturating_sub(1);
                let Some(&[t0, a, b, c, d]) = segments.get(index) else {
                    return 0.0;
                };
                let delta = time - t0;
                ((a * delta + b) * delta + c) * delta + d
            }
            Self::Dense {
                begin_time,
                sample_rate,
                samples,
            } => {
                let Some(last) = samples.len().checked_sub(1) else {
                    return 0.0;
                };
                let position = ((time - begin_time) * sample_rate).max(0.0);
                // `position` is not negative, so the conversion rounds down.
                let lower = (position.floor() as usize).min(last);
                let upper = (lower + 1).min(last);
                let fraction = (position - lower as f32).clamp(0.0, 1.0);
                samples[lower] + (samples[upper] - samples[lower]) * fraction
            }
        }
    }
}

/// An event the clip calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// The clip time, in seconds.
    pub time: f32,
    /// The name of the function called.
    pub function: String,
    /// The string parameter.
    pub data: String,
    /// The floating-point parameter.
    pub float: f32,
    /// The integer parameter.
    pub int: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> ClipDocument {
        ClipDocument {
            format: FORMAT.into(),
            version: VERSION,
            name: "w-a01".into(),
            sample_rate: 60.0,
            start_time: 0.0,
            stop_time: 2.5,
            looping: false,
            curves: vec![
                Curve {
                    path: 397_467_875,
                    attribute: 3_702_945_584,
                    class_id: 114,
                    script: Some("A/B.C".into()),
                    custom_type: 0,
                    data: CurveData::Streamed {
                        segments: vec![
                            [-f32::MAX, 0.0, 0.0, 0.0, 1.0],
                            [0.5, 1.0, -2.0, 0.25, 1.0],
                        ],
                    },
                },
                Curve {
                    path: 1,
                    attribute: 2,
                    class_id: 114,
                    script: None,
                    custom_type: 0,
                    data: CurveData::Constant { value: 0.1 },
                },
            ],
            events: vec![Event {
                time: 0.0,
                function: "F".into(),
                data: "x,1".into(),
                float: 0.0,
                int: 3,
            }],
        }
    }

    #[test]
    fn the_bytes_are_pinned_and_read_back() {
        let doc = document();
        let bytes = doc.to_bytes().unwrap();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"format":"clip","version":1,"name":"w-a01","sample_rate":60.0,"start_time":0.0,"stop_time":2.5,"loop":false,"curves":[{"path":397467875,"attribute":3702945584,"class_id":114,"script":"A/B.C","custom_type":0,"kind":"streamed","segments":[[-3.4028235e+38,0.0,0.0,0.0,1.0],[0.5,1.0,-2.0,0.25,1.0]]},{"path":1,"attribute":2,"class_id":114,"script":null,"custom_type":0,"kind":"constant","value":0.1}],"events":[{"time":0.0,"function":"F","data":"x,1","float":0.0,"int":3}]}"#
        );
        assert_eq!(ClipDocument::from_bytes(&bytes).unwrap(), doc);
    }

    #[test]
    fn streamed_curves_use_the_last_segment_that_has_started() {
        let doc = document();
        let curve = &doc.curves[0].data;
        assert_eq!(curve.evaluate(-1.0), 1.0);
        assert_eq!(curve.evaluate(0.5), 1.0);
        // δ = 0.5: ((1·0.5 − 2)·0.5 + 0.25)·0.5 + 1
        assert_eq!(curve.evaluate(1.0), 0.75);
    }

    #[test]
    fn dense_curves_interpolate_and_hold_their_ends() {
        let curve = CurveData::Dense {
            begin_time: 1.0,
            sample_rate: 2.0,
            samples: vec![0.0, 1.0, 3.0],
        };
        assert_eq!(curve.evaluate(0.0), 0.0);
        assert_eq!(curve.evaluate(1.25), 0.5);
        assert_eq!(curve.evaluate(1.75), 2.0);
        assert_eq!(curve.evaluate(9.0), 3.0);
    }
}
