//! The binary chunk of a glTF file, its views and accessors, and the GLB container.

use super::read::unsupported;
use crate::ConvertError;
use sekai_format::gltf::{
    ARRAY_BUFFER, Accessor, Buffer, BufferView, Document, ELEMENT_ARRAY_BUFFER, FLOAT, Sparse,
    SparseIndices, SparseValues, UNSIGNED_BYTE, UNSIGNED_INT, UNSIGNED_SHORT,
};

/// The views and accessors of a file, and the bytes of its binary chunk.
#[derive(Debug, Default)]
pub(super) struct Buffers {
    bin: Vec<u8>,
    views: Vec<BufferView>,
    accessors: Vec<Accessor>,
}

/// The accessor type of an element of `n` components.
const fn kind(n: usize) -> &'static str {
    match n {
        1 => "SCALAR",
        2 => "VEC2",
        3 => "VEC3",
        4 => "VEC4",
        _ => "MAT4",
    }
}

impl Buffers {
    /// An accessor of 32-bit floats. Vertex attributes have a target, and positions their bounds.
    pub(super) fn floats<const N: usize>(
        &mut self,
        values: &[[f32; N]],
        target: Option<u32>,
        bounded: bool,
    ) -> usize {
        let view = self.view(
            values.iter().flatten().flat_map(|v| v.to_le_bytes()),
            target,
        );
        let (min, max) = if bounded {
            bounds(values)
        } else {
            Default::default()
        };
        self.push(Accessor {
            min,
            max,
            ..accessor(Some(view), FLOAT, N, values.len())
        })
    }

    /// An accessor of vertex colours stored as normalized bytes.
    pub(super) fn colors(&mut self, values: &[[u8; 4]]) -> usize {
        let view = self.view(values.iter().flatten().copied(), Some(ARRAY_BUFFER));
        self.push(Accessor {
            normalized: true,
            ..accessor(Some(view), UNSIGNED_BYTE, 4, values.len())
        })
    }

    /// An accessor of joint indices.
    pub(super) fn joints(&mut self, values: &[[u16; 4]]) -> usize {
        let view = self.view(
            values.iter().flatten().flat_map(|v| v.to_le_bytes()),
            Some(ARRAY_BUFFER),
        );
        self.push(accessor(Some(view), UNSIGNED_SHORT, 4, values.len()))
    }

    /// An accessor of triangle indices: 16-bit when every index is below 65535, which glTF
    /// reserves, and 32-bit otherwise.
    pub(super) fn indices(&mut self, values: &[u32]) -> usize {
        let narrow: Option<Vec<u16>> = values
            .iter()
            .map(|&i| u16::try_from(i).ok().filter(|&i| i != u16::MAX))
            .collect();
        let (view, component) = match narrow {
            Some(narrow) => (
                self.view(
                    narrow.iter().flat_map(|i| i.to_le_bytes()),
                    Some(ELEMENT_ARRAY_BUFFER),
                ),
                UNSIGNED_SHORT,
            ),
            None => (
                self.view(
                    values.iter().flat_map(|i| i.to_le_bytes()),
                    Some(ELEMENT_ARRAY_BUFFER),
                ),
                UNSIGNED_INT,
            ),
        };
        self.push(accessor(Some(view), component, 1, values.len()))
    }

    /// An accessor of `count` 3-vectors that are zero except at the given increasing indices.
    /// Morph target positions have their bounds.
    pub(super) fn sparse(
        &mut self,
        count: usize,
        stored: &[(u32, [f32; 3])],
        bounded: bool,
    ) -> usize {
        let (min, max) = if bounded {
            let mut values: Vec<[f32; 3]> = stored.iter().map(|&(_, v)| v).collect();
            if stored.len() < count {
                values.push([0.0; 3]);
            }
            bounds(&values)
        } else {
            Default::default()
        };
        let sparse = (!stored.is_empty()).then(|| {
            let indices = self.view(stored.iter().flat_map(|(i, _)| i.to_le_bytes()), None);
            let values = self.view(
                stored
                    .iter()
                    .flat_map(|(_, v)| v.iter().flat_map(|c| c.to_le_bytes())),
                None,
            );
            Sparse {
                count: stored.len(),
                indices: SparseIndices {
                    buffer_view: indices,
                    component_type: UNSIGNED_INT,
                },
                values: SparseValues {
                    buffer_view: values,
                },
            }
        });
        self.push(Accessor {
            min,
            max,
            sparse,
            ..accessor(None, FLOAT, 3, count)
        })
    }

    /// Appends a view of `bytes`, starting at a multiple of 4.
    fn view(&mut self, bytes: impl Iterator<Item = u8>, target: Option<u32>) -> usize {
        self.bin.resize(self.bin.len().next_multiple_of(4), 0);
        let byte_offset = self.bin.len();
        self.bin.extend(bytes);
        self.views.push(BufferView {
            buffer: 0,
            byte_offset,
            byte_length: self.bin.len() - byte_offset,
            target,
        });
        self.views.len() - 1
    }

    fn push(&mut self, accessor: Accessor) -> usize {
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    /// Moves the views and accessors into `document`, and returns the binary chunk.
    pub(super) fn finish(self, document: &mut Document) -> Vec<u8> {
        if !self.views.is_empty() {
            document.buffers.push(Buffer {
                byte_length: self.bin.len(),
            });
        }
        document.buffer_views = self.views;
        document.accessors = self.accessors;
        self.bin
    }
}

fn accessor(buffer_view: Option<usize>, component_type: u32, n: usize, count: usize) -> Accessor {
    Accessor {
        buffer_view,
        component_type,
        normalized: false,
        count,
        kind: kind(n),
        min: Vec::new(),
        max: Vec::new(),
        sparse: None,
    }
}

/// The smallest and largest value of each component, in the total order of floats, which puts
/// −0 below +0 on every platform.
fn bounds<const N: usize>(values: &[[f32; N]]) -> (Vec<f32>, Vec<f32>) {
    let mut min = [f32::INFINITY; N];
    let mut max = [f32::NEG_INFINITY; N];
    for value in values {
        for k in 0..N {
            if value[k].total_cmp(&min[k]).is_lt() {
                min[k] = value[k];
            }
            if value[k].total_cmp(&max[k]).is_gt() {
                max[k] = value[k];
            }
        }
    }
    (min.to_vec(), max.to_vec())
}

/// A GLB file: the JSON chunk padded with spaces and the binary chunk padded with zeros, when
/// there is one.
pub(super) fn container(json: &[u8], bin: &[u8]) -> Result<Vec<u8>, ConvertError> {
    let json_length = json.len().next_multiple_of(4);
    let bin_length = bin.len().next_multiple_of(4);
    let bin_chunk = if bin.is_empty() { 0 } else { 8 + bin_length };
    let total = 12 + 8 + json_length + bin_chunk;
    let length = |n: usize| {
        u32::try_from(n)
            .map(u32::to_le_bytes)
            .map_err(|_| unsupported("the model is larger than a GLB file can be"))
    };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&length(total)?);
    out.extend_from_slice(&length(json_length)?);
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(json);
    out.resize(20 + json_length, b' ');
    if !bin.is_empty() {
        out.extend_from_slice(&length(bin_length)?);
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(bin);
        out.resize(total, 0);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::bounds;

    #[test]
    fn bounds_order_signed_zeros() {
        let (min, max) = bounds(&[[0.0, 1.0], [-0.0, -2.0]]);
        assert_eq!(
            min.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            [(-0.0f32).to_bits(), (-2.0f32).to_bits()]
        );
        assert_eq!(
            max.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            [0.0f32.to_bits(), 1.0f32.to_bits()]
        );
    }
}
