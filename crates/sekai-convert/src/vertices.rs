//! The vertex data of meshes: its channels, split into streams.

use crate::read::{array, count, integer, malformed, unsupported};
use crate::{ConvertError, Value};

pub(crate) const CHANNEL_POSITION: usize = 0;
/// The vertex format of a 32-bit float.
pub(crate) const FLOAT: u8 = 0;
/// The alignment of each vertex stream after the first.
const STREAM_ALIGNMENT: usize = 16;

/// Where the values of a vertex channel are and how they are stored.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Channel {
    stream: usize,
    offset: usize,
    pub(crate) format: u8,
    pub(crate) dimension: usize,
    /// The bytes of one value.
    size: usize,
}

pub(crate) fn channels(vertex_data: &Value) -> Result<Vec<Option<Channel>>, ConvertError> {
    array(vertex_data, "m_Channels")?
        .iter()
        .map(channel)
        .collect()
}

/// A stored channel, or `None` when the mesh does not have it.
fn channel(stored: &Value) -> Result<Option<Channel>, ConvertError> {
    // The low four bits are the number of components.
    let dimension = count(stored, "dimension")? & 15;
    if dimension == 0 {
        return Ok(None);
    }
    let format = u8::try_from(integer(stored, "format")?)
        .map_err(|_| malformed("a vertex format is out of range"))?;
    let component = match format {
        0 | 10 | 11 => 4,
        1 | 4 | 5 | 8 | 9 => 2,
        2 | 3 | 6 | 7 => 1,
        _ => return Err(malformed(format!("vertex format {format}"))),
    };
    Ok(Some(Channel {
        stream: count(stored, "stream")?,
        offset: count(stored, "offset")?,
        format,
        dimension,
        size: component * dimension,
    }))
}

/// The vertex data of a mesh, split into its streams.
pub(crate) struct Vertices<'a> {
    data: &'a [u8],
    /// The number of vertices.
    pub(crate) count: usize,
    channels: Vec<Option<Channel>>,
    /// The start and stride of each stream.
    streams: Vec<(usize, usize)>,
}

impl<'a> Vertices<'a> {
    pub(crate) fn new(
        data: &'a [u8],
        count: usize,
        channels: Vec<Option<Channel>>,
    ) -> Result<Self, ConvertError> {
        let stream_count = channels.iter().flatten().map(|c| c.stream + 1).max();
        let mut streams = Vec::new();
        let mut start = 0;
        for stream in 0..stream_count.unwrap_or(0) {
            let stride = channels
                .iter()
                .flatten()
                .filter(|c| c.stream == stream)
                .map(|c| c.offset + c.size)
                .max()
                .unwrap_or(0);
            let end = stride
                .checked_mul(count)
                .and_then(|size| size.checked_add(start))
                .filter(|&end| end <= data.len())
                .ok_or_else(|| {
                    malformed(format!("vertex stream {stream} lies outside its data"))
                })?;
            streams.push((start, stride));
            start = end.next_multiple_of(STREAM_ALIGNMENT);
        }
        Ok(Self {
            data,
            count,
            channels,
            streams,
        })
    }

    pub(crate) fn channel(&self, index: usize) -> Option<Channel> {
        self.channels.get(index).copied().flatten()
    }

    /// The bytes of each value of a present channel.
    pub(crate) fn values(&self, channel: Channel) -> impl Iterator<Item = &'a [u8]> + use<'a> {
        let (start, stride) = self.streams[channel.stream];
        let data = self.data;
        (0..self.count).map(move |i| &data[start + i * stride + channel.offset..][..channel.size])
    }

    /// The values of a channel of `N` 32-bit floats, or `None` when it is absent.
    pub(crate) fn floats<const N: usize>(
        &self,
        index: usize,
        what: &str,
    ) -> Result<Option<Vec<[f32; N]>>, ConvertError> {
        let Some(channel) = self.channel(index) else {
            return Ok(None);
        };
        if channel.format != FLOAT || channel.dimension != N {
            return Err(unsupported(format!(
                "{what} with format {} and {} components",
                channel.format, channel.dimension
            )));
        }
        Ok(Some(self.values(channel).map(float_array).collect()))
    }
}

/// The 32-bit floats at the start of `bytes`, zero where `bytes` ends.
pub(crate) fn float_array<const N: usize>(bytes: &[u8]) -> [f32; N] {
    let mut out = [0.0; N];
    for (value, chunk) in out.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *value = f32::from_le_bytes(*chunk);
    }
    out
}
