//! CRI middleware formats: HCA audio, the ACB cue sheet with its AWB wave bank, and USM video.
//!
//! [`decode_hca`] turns one HCA stream into PCM, and [`Hca`] decodes one frame by frame. Reading
//! an ACB yields its cues and the HCA
//! streams they play, taken from the ACB itself or from its AWB; reading a USM yields its video
//! and audio streams as stored. Those readers are submodules of this one.

mod acb;
mod adx;
mod awb;
mod hca;
mod hca_tables;
mod usm;
mod utf;

pub use acb::{Acb, Arrangement, Block, Cue, Layer, Note, VolumeControl, Waveform, read_acb};
pub use awb::{Awb, AwbStream, read_awb};
pub use usm::{AudioStream, Usm, VideoFrame, VideoStream, read_usm};

use std::fmt;

use crate::{DecodeError, Pcm};

/// The key of an encrypted HCA stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HcaKey {
    /// The game's key.
    pub key: u64,
    /// The subkey of the AWB the stream comes from, or 0 for a stream outside an AWB.
    pub subkey: u16,
}

/// What the header of an HCA stream says about its audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HcaInfo {
    /// The channel count.
    pub channels: u16,
    /// The sample rate in hertz.
    pub sample_rate: u32,
    /// The samples per channel that the frames hold, including the encoder delay and padding.
    pub frames: u64,
    /// The samples per channel of the stream without its encoder delay and padding.
    pub length: u64,
    /// The loop of a looping stream, as `[start, end)` in the same samples as `length`.
    pub loop_frames: Option<[u64; 2]>,
    /// The bitrate of the frames, in bits per second.
    pub bitrate: u32,
}

/// Reads the header of one HCA stream without decoding its frames.
pub fn hca_info(data: &[u8]) -> Result<HcaInfo, DecodeError> {
    hca::info(data)
}

/// The version of the HCA decoder. It changes whenever a stream would decode to other samples.
pub const HCA_DECODER_VERSION: u32 = 1;

/// One HCA stream with the key that decrypts it, read far enough to know it decodes: its header
/// is valid, it holds every frame the header counts, and each frame has its sync word and a
/// matching checksum.
#[derive(Clone, PartialEq, Eq)]
pub struct Hca {
    data: Vec<u8>,
    key: Option<HcaKey>,
    info: HcaInfo,
}

impl Hca {
    /// Reads the stream `data`, which `key` decrypts.
    ///
    /// A stream of cipher type 56 needs `key`; reading one without it fails with
    /// [`DecodeError::Unsupported`]. The checksum of a frame covers its stored bytes, so a wrong
    /// key is not detected.
    pub fn new(data: Vec<u8>, key: Option<HcaKey>) -> Result<Self, DecodeError> {
        let frames = hca::Frames::new(&data, key)?;
        frames.check()?;
        let info = frames.info();
        Ok(Self { data, key, info })
    }

    /// What the header says about the audio.
    pub fn info(&self) -> HcaInfo {
        self.info
    }

    /// The stream as stored.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The key that decrypts the stream.
    pub fn key(&self) -> Option<HcaKey> {
        self.key
    }

    /// Decodes the stream frame by frame and gives `each` the samples of each frame,
    /// interleaved by channel, without the encoder delay and padding.
    pub fn decode_each(&self, mut each: impl FnMut(&[i16])) -> Result<(), DecodeError> {
        let mut frames = hca::Frames::new(&self.data, self.key)?;
        let mut samples = Vec::new();
        while frames.next(&mut samples)? {
            each(&samples);
            samples.clear();
        }
        Ok(())
    }
}

impl fmt::Debug for Hca {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let HcaInfo {
            channels,
            sample_rate,
            length,
            bitrate,
            ..
        } = self.info;
        write!(
            f,
            "Hca({length} frames, {channels} channels, {sample_rate} Hz, {bitrate} bit/s)"
        )
    }
}

/// Decodes one ADX stream to PCM.
///
/// Streams that are not version 4, encoding type 3 with four-bit samples, including encrypted
/// ones, are reported as [`DecodeError::Unsupported`].
pub fn decode_adx(data: &[u8]) -> Result<Pcm, DecodeError> {
    adx::decode(data, false)
}

/// Decodes the whole frames of one ADX stream. A stream carried in a USM file can end before the
/// sample count of its header.
pub fn decode_adx_available(data: &[u8]) -> Result<Pcm, DecodeError> {
    adx::decode(data, true)
}

/// Decodes one HCA stream to PCM, without its encoder delay and padding.
///
/// A stream of cipher type 56 needs `key`; decoding one without it fails with
/// [`DecodeError::Unsupported`]. The checksum of a frame covers its stored bytes, so a wrong key
/// is not detected.
pub fn decode_hca(data: &[u8], key: Option<HcaKey>) -> Result<Pcm, DecodeError> {
    hca::decode(data, key)
}
