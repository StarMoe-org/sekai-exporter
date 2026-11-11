//! Audio from CRI ACB cue sheets.
//!
//! [`Audio`] produces the `audio` form, specified in `docs/spec/audio.md`.

use std::collections::BTreeMap;

use sekai_codec::DecodeError;
use sekai_codec::cri::{Acb, Arrangement, HcaKey, Note, decode_hca, hca_info, read_acb};
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

/// The cues of a cue sheet, decoded to PCM.
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
        3
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
                let (pcm, loop_frames) = self.waveform(&acb, w, name)?;
                played.push(CueWaveform {
                    role: role.clone(),
                    frames: pcm.frames() as u64,
                    sample_rate: pcm.sample_rate(),
                    channels: pcm.channels(),
                    loop_frames,
                    volume,
                });
                members.push(Member {
                    role,
                    content: Content::Audio(pcm),
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

    /// The cue sheet twice, the samples of every member, and the encoded form of the largest
    /// member, taken as half the samples.
    fn cost(&self, root: ObjectId, object: &ObjectIndex, cx: &dyn Context) -> Cost {
        let stored = u64::from(object.size);
        let memory = match samples(root, cx) {
            Some(bytes) => stored
                .saturating_mul(2)
                .saturating_add(bytes)
                .saturating_add(bytes / 2),
            None => Cost::light(stored).memory,
        };
        Cost {
            class: CostClass::Heavy,
            memory,
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

/// The size of the 16-bit samples of every member the cue sheet `root` gives, from the headers of
/// its streams. Waveforms the converter cannot decode are left out.
fn samples(root: ObjectId, cx: &dyn Context) -> Option<u64> {
    let object = cx.object(root).ok()?;
    let acb = read_acb(object.fields.get("m_Script")?.as_bytes()?).ok()?;
    let bank = acb.memory_awb.as_ref()?;
    let mut total = 0u64;
    for cue in &acb.cues {
        let mut seen = Vec::new();
        for &w in &cue.waveforms {
            if seen.contains(&w) {
                continue;
            }
            seen.push(w);
            let Some(stream) = acb
                .waveforms
                .get(w)
                .and_then(|waveform| waveform.memory_awb_id)
                .and_then(|id| bank.stream(id))
            else {
                continue;
            };
            if let Ok(info) = hca_info(stream.data) {
                total = total.saturating_add(info.frames * u64::from(info.channels) * 2);
            }
        }
    }
    Some(total)
}

impl Audio {
    /// Decodes waveform `index` of `acb`, which cue `cue` plays, and gives its loop.
    fn waveform(
        &self,
        acb: &Acb<'_>,
        index: usize,
        cue: &str,
    ) -> Result<(sekai_codec::Pcm, Option<[u64; 2]>), ConvertError> {
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
        let pcm = decode_hca(stream.data, key).map_err(|e| match e {
            DecodeError::Unsupported { what, .. }
                if key.is_none() && what.starts_with("HCA stream encrypted") =>
            {
                ConvertError::new(
                    "missing_key",
                    format!("cue {cue} is encrypted and no HCA key was given"),
                )
            }
            e => ConvertError::from(e),
        })?;
        Ok((pcm, hca_info(stream.data)?.loop_frames))
    }
}

fn malformed(message: impl Into<String>) -> ConvertError {
    ConvertError::new("malformed", message)
}
