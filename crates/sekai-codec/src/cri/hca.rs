//! HCA decoding.
//!
//! Follows the structure of clHCA in vgmstream (by nyaga, kode54, bnnm and others), and covers
//! versions 2.0 and 3.0 with one or two channels per track. The notice of vgmstream is in
//! `hca_tables.rs`.

use super::HcaKey;
use super::hca_tables::{
    COS, CRC_MASK, INTENSITY_RATIO, INVERT, MAX_BIT, RANGE, READ_BIT, READ_VAL, SCALE_CONVERSION,
    SCALING, SIN, WINDOW,
};
use crate::{DecodeError, Pcm};

const SUBFRAMES: usize = 8;
const BANDS: usize = 128;
const FRAME_SAMPLES: usize = SUBFRAMES * BANDS;
const TRANSFORM_STAGES: usize = 7;

const CHUNK_MASK: u32 = 0x7F7F7F7F;
const VERSION_2: u32 = 0x0200;
const VERSION_3: u32 = 0x0300;

fn malformed(message: impl Into<String>) -> DecodeError {
    DecodeError::Malformed {
        what: "HCA stream",
        message: message.into(),
    }
}

fn unsupported(what: &'static str, value: u32) -> DecodeError {
    DecodeError::Unsupported {
        what,
        value: value.into(),
    }
}

fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0, |sum, &byte| {
        (sum << 8) ^ CRC_MASK[((sum >> 8) as u8 ^ byte) as usize]
    })
}

/// Reads bits from the top of each byte. A read past the end gives zero, as the format's
/// decoder does.
struct Bits<'a> {
    data: &'a [u8],
    position: usize,
}

impl Bits<'_> {
    fn peek(&self, count: usize) -> u32 {
        if count == 0 || self.position + count > self.data.len() * 8 {
            return 0;
        }
        let window = (0..8).fold(0u64, |value, i| {
            let byte = self.data.get(self.position / 8 + i).copied().unwrap_or(0);
            value << 8 | u64::from(byte)
        });
        ((window << (self.position % 8)) >> (64 - count)) as u32
    }

    fn read(&mut self, count: usize) -> u32 {
        let value = self.peek(count);
        self.position += count;
        value
    }

    fn chunk(&mut self, name: &[u8; 4]) -> bool {
        let found = self.peek(32) & CHUNK_MASK == u32::from_be_bytes(*name);
        if found {
            self.position += 32;
        }
        found
    }
}

struct Header {
    version: u32,
    size: usize,
    channels: usize,
    sample_rate: u32,
    frame_count: usize,
    delay: usize,
    padding: usize,
    looping: Option<Looping>,
    frame_size: usize,
    min_resolution: u8,
    max_resolution: u8,
    track_count: usize,
    total_bands: usize,
    base_bands: usize,
    stereo_bands: usize,
    bands_per_group: usize,
    group_count: usize,
    cipher: u32,
}

/// The `loop` chunk of a header.
struct Looping {
    start_frame: usize,
    end_frame: usize,
    start_delay: usize,
    end_padding: usize,
}

fn parse_header(data: &[u8]) -> Result<Header, DecodeError> {
    if data.len() < 8 {
        return Err(DecodeError::Truncated {
            what: "HCA header",
            needed: 8,
            available: data.len(),
        });
    }
    let mut bits = Bits { data, position: 0 };
    if !bits.chunk(b"HCA\0") {
        return Err(malformed("no HCA signature"));
    }
    let version = bits.read(16);
    let size = bits.read(16) as usize;
    if version != VERSION_2 && version != VERSION_3 {
        return Err(unsupported("HCA version", version));
    }
    if data.len() < size {
        return Err(DecodeError::Truncated {
            what: "HCA header",
            needed: size,
            available: data.len(),
        });
    }
    if crc16(&data[..size]) != 0 {
        return Err(malformed("header checksum does not match"));
    }
    let mut bits = Bits {
        data: &data[..size],
        position: bits.position,
    };

    if !bits.chunk(b"fmt\0") {
        return Err(malformed("no fmt chunk"));
    }
    let channels = bits.read(8) as usize;
    let sample_rate = bits.read(24);
    let frame_count = bits.read(32) as usize;
    let delay = bits.read(16) as usize;
    let padding = bits.read(16) as usize;
    if !(1..=2).contains(&channels) {
        return Err(unsupported("HCA channel count", channels as u32));
    }
    if sample_rate == 0 || frame_count == 0 {
        return Err(malformed("empty format"));
    }

    if !bits.chunk(b"comp") {
        return Err(unsupported("HCA header without a comp chunk", version));
    }
    let frame_size = bits.read(16) as usize;
    let min_resolution = bits.read(8) as u8;
    let max_resolution = bits.read(8) as u8;
    let track_count = (bits.read(8) as usize).max(1);
    let _channel_config = bits.read(8);
    let total_bands = bits.read(8) as usize;
    let base_bands = bits.read(8) as usize;
    let stereo_bands = bits.read(8) as usize;
    let bands_per_group = bits.read(8) as usize;
    let ms_stereo = bits.read(8);
    let _reserved = bits.read(8);
    if ms_stereo != 0 {
        return Err(unsupported("HCA mid-side stereo", ms_stereo));
    }

    for name in [b"vbr\0", b"ath\0"] {
        if bits.peek(32) & CHUNK_MASK == u32::from_be_bytes(*name) {
            return Err(unsupported("HCA header chunk", u32::from_be_bytes(*name)));
        }
    }
    let looping = bits.chunk(b"loop").then(|| Looping {
        start_frame: bits.read(32) as usize,
        end_frame: bits.read(32) as usize,
        start_delay: bits.read(16) as usize,
        end_padding: bits.read(16) as usize,
    });
    let cipher = if bits.chunk(b"ciph") {
        bits.read(16)
    } else {
        0
    };

    if frame_size < 8 {
        return Err(malformed("frame size below the minimum"));
    }
    if min_resolution > max_resolution || max_resolution > 15 {
        return Err(malformed("resolution range is invalid"));
    }
    if track_count > channels {
        return Err(malformed("more tracks than channels"));
    }
    if total_bands == 0
        || total_bands > BANDS
        || base_bands + stereo_bands == 0
        || base_bands + stereo_bands > total_bands
        || stereo_bands > base_bands
        || bands_per_group > BANDS
    {
        return Err(malformed("band counts are invalid"));
    }
    let group_count = (total_bands - base_bands - stereo_bands)
        .checked_div(bands_per_group)
        .map_or(0, |whole| {
            whole
                + usize::from(
                    !(total_bands - base_bands - stereo_bands).is_multiple_of(bands_per_group),
                )
        });
    Ok(Header {
        version,
        size,
        channels,
        sample_rate,
        frame_count,
        delay,
        padding,
        looping,
        frame_size,
        min_resolution,
        max_resolution,
        track_count,
        total_bands,
        base_bands,
        stereo_bands,
        bands_per_group,
        group_count,
        cipher,
    })
}

/// The table that maps stored frame bytes to plain ones.
fn cipher_table(header: &Header, key: Option<HcaKey>) -> Result<[u8; 256], DecodeError> {
    match (header.cipher, key) {
        (0, _) => Ok(core::array::from_fn(|i| i as u8)),
        (1, _) => Ok(cipher_table_1()),
        (56, Some(key)) => Ok(cipher_table_56(key)),
        (56, None) => Err(DecodeError::MissingKey { what: "HCA stream" }),
        (other, _) => Err(unsupported("HCA cipher type", other)),
    }
}

fn cipher_table_1() -> [u8; 256] {
    let mut table = [0; 256];
    let mut value = 0u32;
    for slot in &mut table[1..255] {
        value = (value * 13 + 11) & 0xFF;
        if value == 0 || value == 0xFF {
            value = (value * 13 + 11) & 0xFF;
        }
        *slot = value as u8;
    }
    table[255] = 0xFF;
    table
}

fn cipher_nibbles(key: u8) -> [u8; 16] {
    let multiplier = ((key & 1) << 3) | 5;
    let increment = (key & 0xE) | 1;
    let mut value = key >> 4;
    core::array::from_fn(|_| {
        value = value.wrapping_mul(multiplier).wrapping_add(increment) & 0xF;
        value
    })
}

fn cipher_table_56(key: HcaKey) -> [u8; 256] {
    let mut code = key.key;
    if key.subkey != 0 {
        let subkey = u64::from(key.subkey);
        code = code.wrapping_mul((subkey << 16) | (u64::from(!key.subkey) + 2));
    }
    if code == 0 {
        return core::array::from_fn(|i| i as u8);
    }
    let k = (code - 1).to_le_bytes();
    let seed = [
        k[1],
        k[1] ^ k[6],
        k[2] ^ k[3],
        k[2],
        k[2] ^ k[1],
        k[3] ^ k[4],
        k[3],
        k[3] ^ k[2],
        k[4] ^ k[5],
        k[4],
        k[4] ^ k[3],
        k[5] ^ k[6],
        k[5],
        k[5] ^ k[4],
        k[6] ^ k[1],
        k[6],
    ];
    let rows = cipher_nibbles(k[0]);
    let mut base = [0u8; 256];
    for (r, seed) in seed.iter().enumerate() {
        let columns = cipher_nibbles(*seed);
        for (c, column) in columns.iter().enumerate() {
            base[r * 16 + c] = (rows[r] << 4) | column;
        }
    }
    let mut table = [0; 256];
    let mut position = 1;
    let mut x = 0usize;
    for _ in 0..256 {
        x = (x + 17) & 0xFF;
        if base[x] != 0 && base[x] != 0xFF {
            table[position] = base[x];
            position += 1;
        }
    }
    table[255] = 0xFF;
    table
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Discrete,
    Primary,
    Secondary,
}

#[derive(Clone)]
struct Channel {
    kind: Kind,
    coded_count: usize,
    intensity: [u8; SUBFRAMES],
    scalefactors: [u8; BANDS],
    resolution: [u8; BANDS],
    noises: [u8; BANDS],
    noise_count: usize,
    valid_count: usize,
    gain: [f32; BANDS],
    spectra: [[f32; BANDS]; SUBFRAMES],
    previous: [f32; BANDS],
    wave: [[f32; BANDS]; SUBFRAMES],
}

impl Channel {
    fn new(kind: Kind, coded_count: usize) -> Self {
        Self {
            kind,
            coded_count,
            intensity: [0; SUBFRAMES],
            scalefactors: [0; BANDS],
            resolution: [0; BANDS],
            noises: [0; BANDS],
            noise_count: 0,
            valid_count: 0,
            gain: [0.0; BANDS],
            spectra: [[0.0; BANDS]; SUBFRAMES],
            previous: [0.0; BANDS],
            wave: [[0.0; BANDS]; SUBFRAMES],
        }
    }

    fn unpack_scalefactors(
        &mut self,
        bits: &mut Bits<'_>,
        header: &Header,
    ) -> Result<(), DecodeError> {
        let delta_bits = bits.read(3) as usize;
        let coded = self.coded_count;
        let groups = if self.kind == Kind::Secondary
            || header.group_count == 0
            || header.version <= VERSION_2
        {
            0
        } else {
            header.group_count
        };
        let count = coded + groups;
        if delta_bits >= 6 {
            for factor in &mut self.scalefactors[..count] {
                *factor = bits.read(6) as u8;
            }
        } else if delta_bits > 0 {
            let escape = (1i32 << delta_bits) - 1;
            let mut value = bits.read(6) as i32;
            self.scalefactors[0] = value as u8;
            for factor in &mut self.scalefactors[1..count] {
                let delta = bits.read(delta_bits) as i32;
                if delta == escape {
                    value = bits.read(6) as i32;
                } else {
                    value += delta - (escape >> 1);
                    if !(0..64).contains(&value) {
                        return Err(malformed("scale factor out of range"));
                    }
                }
                *factor = value as u8;
            }
        } else {
            self.scalefactors = [0; BANDS];
        }
        for i in (1..=groups).rev() {
            self.scalefactors[BANDS - 1 - groups + i] = self.scalefactors[coded - 1 + i];
        }
        Ok(())
    }

    fn unpack_intensity(
        &mut self,
        bits: &mut Bits<'_>,
        header: &Header,
    ) -> Result<(), DecodeError> {
        if self.kind != Kind::Secondary {
            if header.version <= VERSION_2 {
                let groups = header.group_count;
                for factor in &mut self.scalefactors[BANDS - groups..] {
                    *factor = bits.read(6) as u8;
                }
            }
            return Ok(());
        }
        let first = bits.peek(4) as u8;
        if header.version <= VERSION_2 {
            self.intensity[0] = first;
            if first < 15 {
                bits.position += 4;
                for value in &mut self.intensity[1..] {
                    *value = bits.read(4) as u8;
                }
            }
            return Ok(());
        }
        bits.position += 4;
        if first >= 15 {
            self.intensity = [7; SUBFRAMES];
            return Ok(());
        }
        let delta_bits = bits.read(2) as usize;
        self.intensity[0] = first;
        if delta_bits == 3 {
            for value in &mut self.intensity[1..] {
                *value = bits.read(4) as u8;
            }
            return Ok(());
        }
        let escape = (2i32 << delta_bits) - 1;
        let mut value = i32::from(first);
        for slot in &mut self.intensity[1..] {
            let delta = bits.read(delta_bits + 1) as i32;
            if delta == escape {
                value = bits.read(4) as i32;
            } else {
                value += delta - (escape >> 1);
                if !(0..=15).contains(&value) {
                    return Err(malformed("intensity out of range"));
                }
            }
            *slot = value as u8;
        }
        Ok(())
    }

    fn calculate_resolution(&mut self, packed_noise: u32, header: &Header) {
        self.noise_count = 0;
        self.valid_count = 0;
        for i in 0..self.coded_count {
            let mut resolution = 0;
            let factor = i32::from(self.scalefactors[i]);
            if factor > 0 {
                let noise_level = (packed_noise.wrapping_add(i as u32) >> 8) as i32;
                let position = noise_level + 1 - ((5 * factor) >> 1);
                resolution = match position {
                    ..0 => 15,
                    0..=65 => INVERT[position as usize],
                    _ => 0,
                };
                resolution = resolution.clamp(header.min_resolution, header.max_resolution);
                if resolution < 1 {
                    self.noises[self.noise_count] = i as u8;
                    self.noise_count += 1;
                } else {
                    self.noises[BANDS - 1 - self.valid_count] = i as u8;
                    self.valid_count += 1;
                }
            }
            self.resolution[i] = resolution;
        }
        self.resolution[self.coded_count..].fill(0);
    }

    fn calculate_gain(&mut self) {
        for i in 0..self.coded_count {
            let scale = f32::from_bits(SCALING[self.scalefactors[i] as usize]);
            self.gain[i] = scale * f32::from_bits(RANGE[self.resolution[i] as usize]);
        }
    }

    fn dequantize(&mut self, bits: &mut Bits<'_>, subframe: usize) {
        let coded = self.coded_count;
        for i in 0..coded {
            let resolution = self.resolution[i] as usize;
            let max_bits = MAX_BIT[resolution] as usize;
            let code = bits.read(max_bits) as i32;
            let quantized = if resolution > 7 {
                // Sign in the lowest bit; zero has no sign bit.
                let value = (1 - ((code & 1) << 1)) * (code >> 1);
                if value == 0 {
                    bits.position -= 1;
                }
                value as f32
            } else {
                let index = (resolution << 4) + code as usize;
                bits.position = bits.position + READ_BIT[index] as usize - max_bits;
                f32::from(READ_VAL[index])
            };
            self.spectra[subframe][i] = self.gain[i] * quantized;
        }
        self.spectra[subframe][coded..].fill(0.0);
    }

    fn reconstruct_noise(&mut self, header: &Header, random: &mut u32, subframe: usize) {
        if header.min_resolution > 0 || self.valid_count == 0 || self.noise_count == 0 {
            return;
        }
        for i in 0..self.noise_count {
            *random = random.wrapping_mul(0x343FD).wrapping_add(0x269EC3);
            let out = ((*random >> 16) & 0x7FFF) as usize;
            let index = BANDS - self.valid_count + ((out * self.valid_count) >> 15);
            let target = self.noises[i] as usize;
            let source = self.noises[index] as usize;
            let difference =
                i32::from(self.scalefactors[target]) - i32::from(self.scalefactors[source]);
            let scale = f32::from_bits(SCALE_CONVERSION[(difference + 62).max(0) as usize]);
            self.spectra[subframe][target] = scale * self.spectra[subframe][source];
        }
    }

    fn reconstruct_high_frequency(&mut self, header: &Header, subframe: usize) {
        if header.bands_per_group == 0 || self.kind == Kind::Secondary {
            return;
        }
        let groups = header.group_count;
        let start = header.base_bands + header.stereo_bands;
        let mut high = start;
        let mut low = start as isize - 1;
        let limit = if header.version <= VERSION_2 {
            groups
        } else {
            groups >> 1
        };
        'groups: for group in 0..groups {
            let step = if group < limit { -1 } else { 1 };
            for _ in 0..header.bands_per_group {
                if high >= header.total_bands || low < 0 {
                    break 'groups;
                }
                let difference = i32::from(self.scalefactors[BANDS - groups + group])
                    - i32::from(self.scalefactors[low as usize]);
                let scale = f32::from_bits(SCALE_CONVERSION[(difference + 63).max(0) as usize]);
                self.spectra[subframe][high] = scale * self.spectra[subframe][low as usize];
                high += 1;
                low += step;
            }
        }
        self.spectra[subframe][high - 1] = 0.0;
    }

    /// The inverse transform of one subframe: a DCT-IV of the spectrum, then overlap with the
    /// previous subframe.
    fn transform(&mut self, subframe: usize) {
        let mut buffers = [self.spectra[subframe], [0.0; BANDS]];
        let mut source = 0;

        let (mut count1, mut count2) = (1, BANDS / 2);
        for _ in 0..TRANSFORM_STAGES {
            let from = buffers[source];
            let to = &mut buffers[1 - source];
            let (mut p, mut d1, mut d2) = (0, 0, count2);
            for _ in 0..count1 {
                for _ in 0..count2 {
                    let (a, b) = (from[p], from[p + 1]);
                    p += 2;
                    to[d1] = a + b;
                    to[d2] = a - b;
                    d1 += 1;
                    d2 += 1;
                }
                d1 += count2;
                d2 += count2;
            }
            source = 1 - source;
            count1 <<= 1;
            count2 >>= 1;
        }

        let (mut count1, mut count2) = (BANDS / 2, 1);
        for stage in 0..TRANSFORM_STAGES {
            let from = buffers[source];
            let to = &mut buffers[1 - source];
            let (mut s1, mut s2, mut d1, mut d2) = (0, count2, 0, count2 * 2 - 1);
            let mut twiddle = 0;
            for _ in 0..count1 {
                for _ in 0..count2 {
                    let (a, b) = (from[s1], from[s2]);
                    let sin = f32::from_bits(SIN[stage][twiddle]);
                    let cos = f32::from_bits(COS[stage][twiddle]);
                    twiddle += 1;
                    s1 += 1;
                    s2 += 1;
                    to[d1] = a * sin - b * cos;
                    to[d2] = a * cos + b * sin;
                    d1 += 1;
                    d2 = d2.wrapping_sub(1);
                }
                s1 += count2;
                s2 += count2;
                d1 += count2;
                d2 = d2.wrapping_add(count2 * 3);
            }
            source = 1 - source;
            count1 >>= 1;
            count2 <<= 1;
        }

        let dct = &buffers[source];
        let window = |i: usize| f32::from_bits(WINDOW[i]);
        let half = BANDS / 2;
        for i in 0..half {
            self.wave[subframe][i] = window(i) * dct[i + half] + self.previous[i];
            self.wave[subframe][i + half] =
                window(i + half) * dct[BANDS - 1 - i] - self.previous[i + half];
            self.previous[i] = window(BANDS - 1 - i) * dct[half - i - 1];
            self.previous[i + half] = window(half - i - 1) * dct[i];
        }
    }
}

/// Restores the upper bands of a pair from the first channel and the intensity of the second.
fn apply_intensity(pair: &mut [Channel], subframe: usize, header: &Header) {
    let [left, right, ..] = pair else { return };
    if left.kind != Kind::Primary {
        return;
    }
    let ratio_left = f32::from_bits(INTENSITY_RATIO[right.intensity[subframe] as usize]);
    let ratio_right = 2.0 - ratio_left;
    for band in header.base_bands..header.total_bands {
        let value = left.spectra[subframe][band];
        left.spectra[subframe][band] = value * ratio_left;
        right.spectra[subframe][band] = value * ratio_right;
    }
}

fn channel_kinds(header: &Header) -> [Kind; 2] {
    if header.stereo_bands > 0 && header.channels / header.track_count == 2 {
        [Kind::Primary, Kind::Secondary]
    } else {
        [Kind::Discrete; 2]
    }
}

fn check_frame(frame: &[u8]) -> Result<(), DecodeError> {
    if frame[..2] != [0xFF, 0xFF] {
        return Err(malformed("frame without sync"));
    }
    if crc16(frame) != 0 {
        return Err(malformed("frame checksum does not match"));
    }
    Ok(())
}

fn decode_frame(
    frame: &[u8],
    plain: &mut [u8],
    table: &[u8; 256],
    header: &Header,
    channels: &mut [Channel],
    random: &mut u32,
) -> Result<(), DecodeError> {
    check_frame(frame)?;
    for (out, &stored) in plain.iter_mut().zip(frame) {
        *out = table[usize::from(stored)];
    }
    let mut bits = Bits {
        data: plain,
        position: 16,
    };
    let noise_level = bits.read(9);
    let boundary = bits.read(7);
    let packed_noise = (noise_level << 8).wrapping_sub(boundary);
    for channel in channels.iter_mut() {
        channel.unpack_scalefactors(&mut bits, header)?;
        channel.unpack_intensity(&mut bits, header)?;
        channel.calculate_resolution(packed_noise, header);
        channel.calculate_gain();
    }
    for subframe in 0..SUBFRAMES {
        for channel in channels.iter_mut() {
            channel.dequantize(&mut bits, subframe);
        }
    }
    for subframe in 0..SUBFRAMES {
        for channel in channels.iter_mut() {
            channel.reconstruct_noise(header, random, subframe);
            channel.reconstruct_high_frequency(header, subframe);
        }
        if header.stereo_bands > 0 {
            for first in 0..channels.len() - 1 {
                apply_intensity(&mut channels[first..], subframe, header);
            }
        }
        for channel in channels.iter_mut() {
            channel.transform(subframe);
        }
    }
    Ok(())
}

/// The loop of a stream as `[start, end)` samples without the encoder delay, when its header
/// has one that lies within the stream.
fn loop_frames(header: &Header, length: usize) -> Option<[u64; 2]> {
    let looping = header.looping.as_ref()?;
    let start =
        (looping.start_frame * FRAME_SAMPLES + looping.start_delay).checked_sub(header.delay)?;
    let end = ((looping.end_frame + 1) * FRAME_SAMPLES)
        .checked_sub(looping.end_padding + header.delay)?;
    (start < end && end <= length).then_some([start as u64, end as u64])
}

/// What the header of a stream says about its audio.
pub(super) fn info(data: &[u8]) -> Result<super::HcaInfo, DecodeError> {
    parse_header(data).map(|header| header_info(&header))
}

fn header_info(header: &Header) -> super::HcaInfo {
    let total = header.frame_count * FRAME_SAMPLES;
    let length = total.saturating_sub(header.delay + header.padding);
    super::HcaInfo {
        channels: header.channels as u16,
        sample_rate: header.sample_rate,
        frames: total as u64,
        length: length as u64,
        loop_frames: loop_frames(header, length),
        bitrate: (header.frame_size as u64 * 8 * u64::from(header.sample_rate)
            / FRAME_SAMPLES as u64) as u32,
    }
}

/// A stream decoded one frame at a time.
pub(super) struct Frames<'a> {
    header: Header,
    table: [u8; 256],
    channels: Vec<Channel>,
    random: u32,
    plain: Vec<u8>,
    /// The frames still to decode.
    frames: std::slice::ChunksExact<'a, u8>,
    /// The sample of the first frame still to decode, counted from the start of the stream.
    start: usize,
    /// The samples kept, `[delay, total - padding)`.
    kept: std::ops::Range<usize>,
}

impl<'a> Frames<'a> {
    /// The frames of the stream `data`, which `key` decrypts.
    pub(super) fn new(data: &'a [u8], key: Option<HcaKey>) -> Result<Self, DecodeError> {
        let header = parse_header(data)?;
        let table = cipher_table(&header, key)?;
        let needed = header
            .frame_count
            .checked_mul(header.frame_size)
            .and_then(|frames| frames.checked_add(header.size))
            .ok_or_else(|| malformed("frame count is too large"))?;
        if data.len() < needed {
            return Err(DecodeError::Truncated {
                what: "HCA frames",
                needed,
                available: data.len(),
            });
        }
        let total = header.frame_count * FRAME_SAMPLES;
        if header.delay + header.padding > total {
            return Err(malformed("delay and padding exceed the stream"));
        }
        let kinds = channel_kinds(&header);
        let channels = (0..header.channels)
            .map(|i| {
                let stereo = if kinds[i] == Kind::Secondary {
                    0
                } else {
                    header.stereo_bands
                };
                Channel::new(kinds[i], header.base_bands + stereo)
            })
            .collect();
        Ok(Self {
            table,
            channels,
            random: 1,
            plain: vec![0; header.frame_size],
            frames: data[header.size..needed].chunks_exact(header.frame_size),
            start: 0,
            kept: header.delay..total - header.padding,
            header,
        })
    }

    pub(super) fn info(&self) -> super::HcaInfo {
        header_info(&self.header)
    }

    /// Checks the sync word and the checksum of every frame without decoding any.
    pub(super) fn check(&self) -> Result<(), DecodeError> {
        self.frames.clone().try_for_each(check_frame)
    }

    /// Decodes the next frame and appends its samples, interleaved by channel, without the
    /// encoder delay and padding it holds. `false` once every frame is decoded.
    pub(super) fn next(&mut self, samples: &mut Vec<i16>) -> Result<bool, DecodeError> {
        let Some(frame) = self.frames.next() else {
            return Ok(false);
        };
        decode_frame(
            frame,
            &mut self.plain,
            &self.table,
            &self.header,
            &mut self.channels,
            &mut self.random,
        )?;
        let start = self.start;
        self.start += FRAME_SAMPLES;
        let kept = self.kept.start.max(start)..self.kept.end.min(self.start);
        for at in kept {
            let (subframe, band) = ((at - start) / BANDS, (at - start) % BANDS);
            for channel in &self.channels {
                let scaled = (channel.wave[subframe][band] * 32767.0) as i32;
                samples.push(scaled.clamp(-32768, 32767) as i16);
            }
        }
        Ok(true)
    }
}

/// Decodes a stream to PCM, without its encoder delay and padding.
pub(super) fn decode(data: &[u8], key: Option<HcaKey>) -> Result<Pcm, DecodeError> {
    let mut frames = Frames::new(data, key)?;
    let info = frames.info();
    let mut samples = Vec::with_capacity(info.length as usize * usize::from(info.channels));
    while frames.next(&mut samples)? {}
    Pcm::new(info.sample_rate, info.channels, samples)
}
