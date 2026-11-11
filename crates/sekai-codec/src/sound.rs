use crate::{DecodeError, Pcm};

/// Audio as a converter gives it: decoded samples, or a stream that is decoded while it is
/// encoded, so its samples are never all held at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sound {
    /// Decoded samples.
    Pcm(Pcm),
    /// An HCA stream.
    #[cfg(feature = "cri")]
    Hca(crate::cri::Hca),
}

impl Sound {
    /// The sample rate in hertz.
    pub fn sample_rate(&self) -> u32 {
        match self {
            Sound::Pcm(pcm) => pcm.sample_rate(),
            #[cfg(feature = "cri")]
            Sound::Hca(hca) => hca.info().sample_rate,
        }
    }

    /// The number of channels.
    pub fn channels(&self) -> u16 {
        match self {
            Sound::Pcm(pcm) => pcm.channels(),
            #[cfg(feature = "cri")]
            Sound::Hca(hca) => hca.info().channels,
        }
    }

    /// The number of frames, samples per channel, that the sound decodes to.
    pub fn frames(&self) -> u64 {
        match self {
            Sound::Pcm(pcm) => pcm.frames() as u64,
            #[cfg(feature = "cri")]
            Sound::Hca(hca) => hca.info().length,
        }
    }

    /// The bytes the sound holds in memory: its samples, or its stream.
    pub fn memory(&self) -> usize {
        match self {
            Sound::Pcm(pcm) => pcm.samples().len() * 2,
            #[cfg(feature = "cri")]
            Sound::Hca(hca) => hca.data().len(),
        }
    }

    /// The bitrate of a lossy stream, in bits per second; `None` for decoded samples.
    pub fn bitrate(&self) -> Option<u32> {
        match self {
            Sound::Pcm(_) => None,
            #[cfg(feature = "cri")]
            Sound::Hca(hca) => Some(hca.info().bitrate),
        }
    }

    /// Decodes the sound and gives `each` its samples, interleaved by channel, block after
    /// block.
    pub fn decode_each(&self, mut each: impl FnMut(&[i16])) -> Result<(), DecodeError> {
        match self {
            Sound::Pcm(pcm) => {
                each(pcm.samples());
                Ok(())
            }
            #[cfg(feature = "cri")]
            Sound::Hca(hca) => hca.decode_each(each),
        }
    }

    /// Gives `feed` bytes that identify what the sound decodes to: two sounds whose bytes are
    /// the same decode to the same samples. Decoded samples are given as they are; a stream is
    /// given with its key and the version of its decoder.
    pub fn identify(&self, mut feed: impl FnMut(&[u8])) {
        match self {
            Sound::Pcm(pcm) => {
                feed(format!("pcm {} {}\n", pcm.sample_rate(), pcm.channels()).as_bytes());
                for chunk in pcm.samples().chunks(4096) {
                    let bytes: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
                    feed(&bytes);
                }
            }
            #[cfg(feature = "cri")]
            Sound::Hca(hca) => {
                let key = hca.key().map(|key| (key.key, key.subkey));
                let decoder = crate::cri::HCA_DECODER_VERSION;
                feed(format!("hca {decoder} {key:?}\n").as_bytes());
                feed(hca.data());
            }
        }
    }
}
