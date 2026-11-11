//! Audio from CRI ACB cue sheets.
//!
//! [`Audio`] produces the `audio` form, specified in `docs/spec/audio.md`.

use std::collections::BTreeMap;

use sekai_codec::cri::{Acb, Arrangement, Hca, HcaKey, Note, read_acb};
use sekai_codec::{DecodeError, Sound};
use sekai_format::audio::{
    CueArrangement, CueBlock, CueEntry, CueLayer, CueNote, CueTable, CueVolumeControl, CueWaveform,
    FORMAT, VERSION,
};
use sekai_unity::Value;

use crate::roots;
use crate::{
    Asset, Content, Context, ConvertError, Converter, Cost, CostClass, Member, ObjectId,
    ObjectIndex,
};

/// The `EncodeType` of HCA waveforms.
const HCA: u8 = 2;

/// The cues of a cue sheet, with the stream each of their waveforms plays.
#[derive(Debug, Clone, Copy, Default)]
pub struct Audio {
    key: Option<u64>,
}

impl Audio {
    /// A converter that decrypts HCA streams with `key`. Without a key, an encrypted stream is
    /// the error `missing_key`.
    pub fn new(key: Option<u64>) -> Self {
        Self { key }
    }
}

impl Converter for Audio {
    fn name(&self) -> &'static str {
        "audio"
    }

    fn version(&self) -> u32 {
        5
    }

    fn encoders(&self) -> crate::Encoders {
        crate::Encoders::AUDIO
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        roots::audio(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let data = object
            .fields
            .get("m_Script")
            .and_then(Value::as_bytes)
            .ok_or_else(|| malformed("m_Script is missing or is not a string"))?;
        let acb = read_acb(data)?;
        let mut members = Vec::new();
        let mut cues = Vec::new();
        for cue in &acb.cues {
            let [name] = cue.cue_names.as_slice() else {
                return Err(ConvertError::new(
                    "unsupported",
                    format!("cue {} has {} names", cue.cue_id, cue.cue_names.len()),
                ));
            };
            let mut waveforms: Vec<(usize, f32)> = Vec::new();
            for (&w, &volume) in cue.waveforms.iter().zip(&cue.volumes) {
                if !waveforms.iter().any(|&(seen, _)| seen == w) {
                    waveforms.push((w, volume));
                }
            }
            let single = waveforms.len() == 1;
            let mut played = Vec::new();
            let mut roles = BTreeMap::new();
            for (n, (w, volume)) in waveforms.into_iter().enumerate() {
                let role = if single {
                    format!("cue/{name}")
                } else {
                    format!("cue/{name}/{}", n + 1)
                };
                roles.insert(w, role.clone());
                let hca = self.waveform(&acb, w, name)?;
                let info = hca.info();
                played.push(CueWaveform {
                    role: role.clone(),
                    frames: info.length,
                    sample_rate: info.sample_rate,
                    channels: info.channels,
                    loop_frames: info.loop_frames,
                    volume,
                });
                members.push(Member {
                    role,
                    content: Content::Audio(Sound::Hca(hca)),
                });
            }
            if !played.is_empty() {
                cues.push(CueEntry {
                    name: name.clone(),
                    waveforms: played,
                    arrangement: cue
                        .arrangement
                        .as_ref()
                        .map(|arrangement| cue_arrangement(arrangement, &roles)),
                });
            }
        }
        if members.is_empty() {
            return Ok(None);
        }
        let table = CueTable {
            format: FORMAT.to_owned(),
            version: VERSION,
            cues,
        };
        members.push(Member {
            role: "cues".into(),
            content: Content::File {
                extension: "json".into(),
                bytes: table
                    .to_bytes()
                    .map_err(|e| ConvertError::new("malformed", e.to_string()))?,
            },
        });
        Ok(Some(Asset { members }))
    }

    /// The cue sheet twice, a copy of the streams it holds, and their MP3 files, taken as
    /// twice the streams.
    fn cost(&self, _root: ObjectId, object: &ObjectIndex, _cx: &dyn Context) -> Cost {
        Cost {
            class: CostClass::Heavy,
            memory: u64::from(object.size).saturating_mul(5),
        }
    }
}

/// `arrangement` with each waveform given by the role of its member. `roles` has every waveform
/// the cue plays, and an arrangement names no others.
fn cue_arrangement(arrangement: &Arrangement, roles: &BTreeMap<usize, String>) -> CueArrangement {
    let notes = |notes: &[Note]| {
        notes
            .iter()
            .map(|note| CueNote {
                at_us: note.at_us,
                role: roles[&note.waveform].clone(),
                volume: note.volume,
            })
            .collect::<Vec<_>>()
    };
    let layers = arrangement.layers.iter().map(|layer| CueLayer {
        control: layer.control.as_ref().map(|control| CueVolumeControl {
            name: control.name.clone(),
            default: control.default,
            points: control.points.clone(),
        }),
        notes: notes(&layer.notes),
    });
    let blocks = arrangement.blocks.iter().map(|block| CueBlock {
        length_us: block.length_us,
        looping: block.looping,
        repeats: block.repeats,
        transition_parts: block.transition_parts,
        relative: block.relative,
        layers: block.layers.iter().map(|layer| notes(layer)).collect(),
    });
    CueArrangement {
        layers: layers.collect(),
        blocks: blocks.collect(),
    }
}

impl Audio {
    /// The stream of waveform `index` of `acb`, which cue `cue` plays.
    fn waveform(&self, acb: &Acb<'_>, index: usize, cue: &str) -> Result<Hca, ConvertError> {
        let waveform = acb.waveforms.get(index).ok_or_else(|| {
            malformed(format!(
                "cue {cue} plays waveform {index}, which the sheet lacks"
            ))
        })?;
        if waveform.streaming != 0 {
            return Err(ConvertError::new(
                "unsupported",
                format!("cue {cue} plays a streamed waveform"),
            ));
        }
        if waveform.encode_type != HCA {
            return Err(ConvertError::new(
                "unsupported",
                format!(
                    "cue {cue} plays a waveform of encode type {}",
                    waveform.encode_type
                ),
            ));
        }
        let bank = acb
            .memory_awb
            .as_ref()
            .ok_or_else(|| malformed("the sheet has in-memory waveforms but no memory bank"))?;
        let id = waveform
            .memory_awb_id
            .ok_or_else(|| malformed(format!("waveform {index} has no memory bank ID")))?;
        let stream = bank
            .stream(id)
            .ok_or_else(|| malformed(format!("the memory bank lacks wave {id}")))?;
        let key = self.key.map(|key| HcaKey {
            key,
            subkey: bank.subkey,
        });
        Hca::new(stream.data.to_vec(), key).map_err(|e| match e {
            DecodeError::MissingKey { .. } => ConvertError::new(
                e.code(),
                format!("cue {cue} is encrypted and no HCA key was given"),
            ),
            e => ConvertError::from(e),
        })
    }
}

fn malformed(message: impl Into<String>) -> ConvertError {
    ConvertError::new("malformed", message)
}
