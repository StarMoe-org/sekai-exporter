//! A cursor over the bytes of a TPK file that never reads out of bounds.

use crate::error::TpkError;

/// Reads little-endian numbers and length-prefixed strings.
pub(crate) struct Buffer<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Buffer<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Buffer { data, position: 0 }
    }

    /// Reads a count of items, which cannot be negative.
    pub(crate) fn count(&mut self) -> Result<usize, TpkError> {
        usize::try_from(self.i32()?).map_err(|_| TpkError::new("a count is negative"))
    }

    pub(crate) fn skip(&mut self, n: usize) -> Result<(), TpkError> {
        self.bytes(n).map(|_| ())
    }

    /// Skips a list of items of `size` bytes, stored after their count.
    pub(crate) fn skip_list(&mut self, size: usize) -> Result<(), TpkError> {
        let count = self.count()?;
        self.skip(count.saturating_mul(size))
    }

    pub(crate) fn byte(&mut self) -> Result<u8, TpkError> {
        let [byte] = self.array()?;
        Ok(byte)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, TpkError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, TpkError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(crate) fn i32(&mut self) -> Result<i32, TpkError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, TpkError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// Reads a string: its length in groups of 7 bits, then its UTF-8 bytes.
    pub(crate) fn string(&mut self) -> Result<String, TpkError> {
        let (mut length, mut shift) = (0usize, 0u32);
        loop {
            let byte = self.byte()?;
            length |= usize::from(byte & 0x7F)
                .checked_shl(shift)
                .ok_or_else(|| TpkError::new("a string length is too long"))?;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        String::from_utf8(self.bytes(length)?.to_vec())
            .map_err(|_| TpkError::new("a string is not UTF-8"))
    }

    /// Reads the next `n` bytes.
    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8], TpkError> {
        let bytes = self
            .position
            .checked_add(n)
            .and_then(|end| self.data.get(self.position..end))
            .ok_or_else(|| {
                TpkError::new(format!(
                    "the data ends before {n} bytes at offset {}",
                    self.position
                ))
            })?;
        self.position += n;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], TpkError> {
        let mut array = [0; N];
        array.copy_from_slice(self.bytes(N)?);
        Ok(array)
    }
}
