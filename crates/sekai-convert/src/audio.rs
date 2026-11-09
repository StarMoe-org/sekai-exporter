//! Audio from CRI ACB cue sheets.
//!
//! [`Audio`] produces the `audio` form, specified in `docs/spec/audio.md`.

use sekai_codec::DecodeError;
use sekai_codec::cri::{Acb, HcaKey, decode_hca, read_acb};
use sekai_unity::Value;

use crate::{Asset, Content, Context, ConvertError, Converter, Member, ObjectId, ObjectIndex};

/// The class ID of `TextAsset`.
const TEXT_ASSET: i32 = 49;
/// The container path ending of a cue sheet.
const CUE_SHEET: &str = ".acb.bytes";
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
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        _script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        object.class_id == TEXT_ASSET
            && container_paths
                .iter()
                .any(|path| path.to_ascii_lowercase().ends_with(CUE_SHEET))
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
        for cue in &acb.cues {
            let [name] = cue.cue_names.as_slice() else {
                return Err(ConvertError::new(
                    "unsupported",
                    format!("cue {} has {} names", cue.cue_id, cue.cue_names.len()),
                ));
            };
            let mut waveforms = Vec::new();
            for &w in &cue.waveforms {
                if !waveforms.contains(&w) {
                    waveforms.push(w);
                }
            }
            let single = waveforms.len() == 1;
            for (n, w) in waveforms.into_iter().enumerate() {
                let role = if single {
                    format!("cue/{name}")
                } else {
                    format!("cue/{name}/{}", n + 1)
                };
                members.push(Member {
                    role,
                    content: Content::Audio(self.waveform(&acb, w, name)?),
                });
            }
        }
        if members.is_empty() {
            return Ok(None);
        }
        Ok(Some(Asset { members }))
    }
}

impl Audio {
    /// Decodes waveform `index` of `acb`, which cue `cue` plays.
    fn waveform(
        &self,
        acb: &Acb<'_>,
        index: usize,
        cue: &str,
    ) -> Result<sekai_codec::Pcm, ConvertError> {
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
        decode_hca(stream.data, key).map_err(|e| match e {
            DecodeError::Unsupported { what, .. }
                if key.is_none() && what.starts_with("HCA stream encrypted") =>
            {
                ConvertError::new(
                    "missing_key",
                    format!("cue {cue} is encrypted and no HCA key was given"),
                )
            }
            e => e.into(),
        })
    }
}

fn malformed(message: impl Into<String>) -> ConvertError {
    ConvertError::new("malformed", message)
}
