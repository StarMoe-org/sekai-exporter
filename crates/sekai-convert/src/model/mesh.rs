//! Meshes, in the coordinates of glTF.

use std::collections::BTreeSet;

use super::read::{
    array, bytes, count, field, finite, flag, integer, malformed, number, string, unsigned,
    unsupported, vector3,
};
use crate::{Context, ConvertError, ObjectId, Value};

const CHANNEL_POSITION: usize = 0;
const CHANNEL_NORMAL: usize = 1;
const CHANNEL_TANGENT: usize = 2;
const CHANNEL_COLOR: usize = 3;
const CHANNEL_UV: std::ops::Range<usize> = 4..12;
const CHANNEL_WEIGHT: usize = 12;
const CHANNEL_BONE_INDEX: usize = 13;
/// The vertex format of a 32-bit float.
const FLOAT: u8 = 0;
/// The vertex format of an 8-bit normalized unsigned integer.
const UNORM8: u8 = 2;
/// The vertex format of a 32-bit unsigned integer.
const UINT32: u8 = 10;
/// The alignment of each vertex stream after the first.
const STREAM_ALIGNMENT: usize = 16;

/// A mesh in the coordinates of glTF.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Mesh {
    pub(super) name: String,
    pub(super) positions: Vec<[f32; 3]>,
    pub(super) normals: Option<Vec<[f32; 3]>>,
    pub(super) tangents: Option<Vec<[f32; 4]>>,
    pub(super) colors: Option<Colors>,
    /// The texture coordinate sets, from the first.
    pub(super) uvs: Vec<Vec<[f32; 2]>>,
    pub(super) skin: Option<Skin>,
    /// The triangle indices of each submesh.
    pub(super) submeshes: Vec<Vec<u32>>,
    /// The inverse bind matrix of each bone slot, in column order.
    pub(super) bind_poses: Vec<[f32; 16]>,
    /// The CRC-32 of each bone's path below the prefab root, as stored; it may be empty.
    pub(super) bone_hashes: Vec<u32>,
    /// One per blend shape channel.
    pub(super) targets: Vec<Target>,
}

/// The vertex colours of a mesh, as stored.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Colors {
    Float(Vec<[f32; 4]>),
    Bytes(Vec<[u8; 4]>),
}

/// The bones of each vertex.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Skin {
    pub(super) joints: Vec<[u16; 4]>,
    pub(super) weights: Vec<[f32; 4]>,
    /// The bone slots some vertex weighs on.
    pub(super) weighed: BTreeSet<usize>,
}

/// A blend shape channel and its frame.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Target {
    pub(super) name: String,
    /// The channel weight at which the frame applies fully.
    pub(super) full_weight: f32,
    /// By increasing vertex.
    pub(super) deltas: Vec<Delta>,
}

/// How a blend shape frame moves one vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Delta {
    pub(super) vertex: u32,
    pub(super) position: [f32; 3],
    pub(super) normal: [f32; 3],
    pub(super) tangent: [f32; 3],
}

/// Reads the mesh `id`.
pub(super) fn read(id: ObjectId, cx: &dyn Context) -> Result<Mesh, ConvertError> {
    let object = cx.object(id)?;
    let fields = &object.fields;
    if integer(
        field(field(fields, "m_CompressedMesh")?, "m_Vertices")?,
        "m_NumItems",
    )? != 0
    {
        return Err(unsupported("the mesh is compressed"));
    }
    if !array(field(fields, "m_VariableBoneCountWeights")?, "m_Data")?.is_empty() {
        return Err(unsupported("the mesh has variable bone weights"));
    }
    let vertex_data = field(fields, "m_VertexData")?;
    let vertex_count = count(vertex_data, "m_VertexCount")?;
    let stream = fields.get("m_StreamData").filter(|s| {
        s.get("path")
            .and_then(Value::as_str)
            .is_some_and(|p| !p.is_empty())
    });
    let streamed;
    let data = match stream {
        Some(s) => {
            streamed = cx.resource(
                id,
                string(s, "path")?,
                unsigned(s, "offset")?,
                unsigned(s, "size")?,
            )?;
            &*streamed
        }
        None => bytes(vertex_data, "m_DataSize")?,
    };
    let vertices = Vertices::new(data, vertex_count, channels(vertex_data)?)?;
    let positions = vertices
        .floats::<3>(CHANNEL_POSITION, "positions")?
        .ok_or_else(|| malformed("the mesh has no positions"))?
        .into_iter()
        .map(|p| finite(flip_x(p), "a position"))
        .collect::<Result<_, _>>()?;
    Ok(Mesh {
        name: string(fields, "m_Name")?.to_owned(),
        positions,
        normals: vertices
            .floats::<3>(CHANNEL_NORMAL, "normals")?
            .map(|n| n.into_iter().map(flip_x).collect()),
        tangents: vertices
            .floats::<4>(CHANNEL_TANGENT, "tangents")?
            .map(|t| t.into_iter().map(|[x, y, z, w]| [-x, y, z, -w]).collect()),
        colors: colors(&vertices)?,
        uvs: uvs(&vertices)?,
        skin: skin(&vertices)?,
        submeshes: submeshes(fields, vertex_count)?,
        bind_poses: array(fields, "m_BindPose")?
            .iter()
            .map(bind_pose)
            .collect::<Result<_, _>>()?,
        bone_hashes: bone_hashes(fields)?,
        targets: targets(field(fields, "m_Shapes")?, vertex_count)?,
    })
}

/// Where the values of a vertex channel are and how they are stored.
#[derive(Debug, Clone, Copy)]
struct Channel {
    stream: usize,
    offset: usize,
    format: u8,
    dimension: usize,
    /// The bytes of one value.
    size: usize,
}

fn channels(vertex_data: &Value) -> Result<Vec<Option<Channel>>, ConvertError> {
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
struct Vertices<'a> {
    data: &'a [u8],
    count: usize,
    channels: Vec<Option<Channel>>,
    /// The start and stride of each stream.
    streams: Vec<(usize, usize)>,
}

impl<'a> Vertices<'a> {
    fn new(
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

    fn channel(&self, index: usize) -> Option<Channel> {
        self.channels.get(index).copied().flatten()
    }

    /// The bytes of each value of a present channel.
    fn values(&self, channel: Channel) -> impl Iterator<Item = &'a [u8]> + use<'a> {
        let (start, stride) = self.streams[channel.stream];
        let data = self.data;
        (0..self.count).map(move |i| &data[start + i * stride + channel.offset..][..channel.size])
    }

    /// The values of a channel of `N` 32-bit floats, or `None` when it is absent.
    fn floats<const N: usize>(
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
fn float_array<const N: usize>(bytes: &[u8]) -> [f32; N] {
    let mut out = [0.0; N];
    for (value, chunk) in out.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *value = f32::from_le_bytes(*chunk);
    }
    out
}

fn flip_x([x, y, z]: [f32; 3]) -> [f32; 3] {
    [-x, y, z]
}

/// A bind pose `M` as `S M S` with `S = diag(-1, 1, 1, 1)`, in column order.
fn bind_pose(value: &Value) -> Result<[f32; 16], ConvertError> {
    let mut out = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            let stored = number(value, &format!("e{row}{column}"))?;
            // `S M S` negates the entries with exactly one of row and column 0.
            out[column * 4 + row] = if (row == 0) != (column == 0) {
                -stored
            } else {
                stored
            };
        }
    }
    finite(out, "a bind pose")
}

fn colors(vertices: &Vertices<'_>) -> Result<Option<Colors>, ConvertError> {
    let Some(channel) = vertices.channel(CHANNEL_COLOR) else {
        return Ok(None);
    };
    match (channel.format, channel.dimension) {
        (FLOAT, 4) => Ok(Some(Colors::Float(
            vertices.values(channel).map(float_array).collect(),
        ))),
        (UNORM8, 4) => Ok(Some(Colors::Bytes(
            vertices
                .values(channel)
                .map(|b| [b[0], b[1], b[2], b[3]])
                .collect(),
        ))),
        (format, dimension) => Err(unsupported(format!(
            "colours with format {format} and {dimension} components"
        ))),
    }
}

/// The texture coordinate sets, as stored.
fn uvs(vertices: &Vertices<'_>) -> Result<Vec<Vec<[f32; 2]>>, ConvertError> {
    let mut sets = Vec::new();
    let mut ended = false;
    for index in CHANNEL_UV {
        match vertices.floats::<2>(index, "texture coordinates")? {
            None => ended = true,
            Some(_) if ended => {
                return Err(unsupported(
                    "a set of texture coordinates follows a missing set",
                ));
            }
            Some(set) => sets.push(set),
        }
    }
    Ok(sets)
}

/// The bones of each vertex: up to four indices and weights, the rest zero. Indices without
/// weights bind a vertex to its first index. An index whose weight is zero is written as 0.
fn skin(vertices: &Vertices<'_>) -> Result<Option<Skin>, ConvertError> {
    let Some(indices) = vertices.channel(CHANNEL_BONE_INDEX) else {
        return Ok(None);
    };
    if indices.format != UINT32 || indices.dimension > 4 {
        return Err(unsupported(format!(
            "bone indices with format {} and {} components",
            indices.format, indices.dimension
        )));
    }
    let weights: Vec<[f32; 4]> = match vertices.channel(CHANNEL_WEIGHT) {
        Some(stored) if stored.format != FLOAT || stored.dimension > 4 => {
            return Err(unsupported(format!(
                "bone weights with format {} and {} components",
                stored.format, stored.dimension
            )));
        }
        Some(stored) => vertices.values(stored).map(float_array).collect(),
        None => vec![[1.0, 0.0, 0.0, 0.0]; vertices.count],
    };
    let mut joints = Vec::with_capacity(vertices.count);
    let mut weighed = BTreeSet::new();
    for (stored, weight) in vertices.values(indices).zip(&weights) {
        let mut vertex = [0u16; 4];
        for ((slot, chunk), &weight) in vertex.iter_mut().zip(stored.as_chunks::<4>().0).zip(weight)
        {
            if weight == 0.0 {
                continue;
            }
            let joint = u16::try_from(u32::from_le_bytes(*chunk))
                .map_err(|_| unsupported("a bone index does not fit in 16 bits"))?;
            *slot = joint;
            weighed.insert(usize::from(joint));
        }
        joints.push(vertex);
    }
    Ok(Some(Skin {
        joints,
        weights,
        weighed,
    }))
}

/// The stored bone name hashes, or none when the mesh does not store them.
fn bone_hashes(fields: &Value) -> Result<Vec<u32>, ConvertError> {
    let Some(stored) = fields.get("m_BoneNameHashes") else {
        return Ok(Vec::new());
    };
    stored
        .as_array()
        .ok_or_else(|| malformed("m_BoneNameHashes is not an array"))?
        .iter()
        .map(|hash| {
            hash.as_u64()
                .and_then(|hash| u32::try_from(hash).ok())
                .ok_or_else(|| malformed("a bone name hash is not a 32-bit unsigned integer"))
        })
        .collect()
}

/// The triangle indices of each submesh, with the winding reversed by the mirroring.
fn submeshes(fields: &Value, vertex_count: usize) -> Result<Vec<Vec<u32>>, ConvertError> {
    let buffer = bytes(fields, "m_IndexBuffer")?;
    let wide = integer(fields, "m_IndexFormat")? == 1;
    let size = if wide { 4 } else { 2 };
    array(fields, "m_SubMeshes")?
        .iter()
        .map(|submesh| {
            let topology = integer(submesh, "topology")?;
            if topology != 0 {
                return Err(unsupported(format!("a submesh has topology {topology}")));
            }
            let first = count(submesh, "firstByte")?;
            let length = count(submesh, "indexCount")?;
            let base = count(submesh, "baseVertex")?;
            if !length.is_multiple_of(3) {
                return Err(malformed(format!("a triangle list of {length} indices")));
            }
            let stored = length
                .checked_mul(size)
                .and_then(|n| n.checked_add(first))
                .and_then(|end| buffer.get(first..end))
                .ok_or_else(|| malformed("a submesh lies outside the index buffer"))?;
            let mut indices = stored
                .chunks_exact(size)
                .map(|b| {
                    let index = if wide {
                        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                    } else {
                        u32::from(u16::from_le_bytes([b[0], b[1]]))
                    };
                    usize::try_from(index)
                        .ok()
                        .and_then(|i| i.checked_add(base))
                        .filter(|&i| i < vertex_count)
                        .and_then(|i| u32::try_from(i).ok())
                        .ok_or_else(|| malformed("an index lies outside the vertices"))
                })
                .collect::<Result<Vec<u32>, _>>()?;
            for triangle in indices.as_chunks_mut::<3>().0 {
                triangle.swap(1, 2);
            }
            Ok(indices)
        })
        .collect()
}

/// One target per blend shape channel. Offsets the frame does not store are zero.
fn targets(shapes: &Value, vertex_count: usize) -> Result<Vec<Target>, ConvertError> {
    let vertices = array(shapes, "vertices")?;
    let frames = array(shapes, "shapes")?;
    let full_weights = array(shapes, "fullWeights")?;
    array(shapes, "channels")?
        .iter()
        .map(|channel| {
            let name = string(channel, "name")?;
            if integer(channel, "frameCount")? != 1 {
                return Err(unsupported(format!(
                    "the blend shape channel {name} does not have one frame"
                )));
            }
            let index = count(channel, "frameIndex")?;
            let frame = frames
                .get(index)
                .ok_or_else(|| malformed("a blend shape frame lies outside the frames"))?;
            let full_weight = full_weights
                .get(index)
                .and_then(Value::as_f64)
                .ok_or_else(|| malformed("a blend shape frame has no full weight"))?
                as f32;
            let first = count(frame, "firstVertex")?;
            let stored = count(frame, "vertexCount")?
                .checked_add(first)
                .and_then(|end| vertices.get(first..end))
                .ok_or_else(|| malformed("a blend shape frame lies outside its vertices"))?;
            let has_normals = flag(frame, "hasNormals")?;
            let has_tangents = flag(frame, "hasTangents")?;
            let mut deltas = stored
                .iter()
                .map(|v| {
                    let vertex = count(v, "index")?;
                    if vertex >= vertex_count {
                        return Err(malformed(
                            "a blend shape moves a vertex that does not exist",
                        ));
                    }
                    let offset = |name, stored| {
                        Ok::<_, ConvertError>(if stored {
                            flip_x(vector3(v, name)?)
                        } else {
                            [0.0; 3]
                        })
                    };
                    Ok(Delta {
                        vertex: u32::try_from(vertex)
                            .map_err(|_| malformed("a vertex index is out of range"))?,
                        position: finite(
                            flip_x(vector3(v, "vertex")?),
                            "a blend shape position offset",
                        )?,
                        normal: offset("normal", has_normals)?,
                        tangent: offset("tangent", has_tangents)?,
                    })
                })
                .collect::<Result<Vec<_>, ConvertError>>()?;
            deltas.sort_by_key(|d| d.vertex);
            if deltas
                .windows(2)
                .any(|pair| pair[0].vertex == pair[1].vertex)
            {
                return Err(malformed("a blend shape frame moves a vertex twice"));
            }
            Ok(Target {
                name: name.to_owned(),
                full_weight,
                deltas,
            })
        })
        .collect()
}
