//! Byte order and the bounded reader used by every parser in this crate.

use crate::error::{Error, Result};

/// The byte order of stored values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endian {
    /// Least significant byte first.
    Little,
    /// Most significant byte first.
    Big,
}

/// A cursor over a byte slice that reads values in one byte order and never reads out of bounds.
///
/// Alignment is relative to the start of the slice: a reader created over one object's bytes
/// aligns as the engine does, relative to the start of the object.
#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    endian: Endian,
    what: &'static str,
}

macro_rules! read_int {
    ($name:ident, $ty:ty) => {
        pub(crate) fn $name(&mut self) -> Result<$ty> {
            let bytes = self.array::<{ size_of::<$ty>() }>()?;
            Ok(match self.endian {
                Endian::Little => <$ty>::from_le_bytes(bytes),
                Endian::Big => <$ty>::from_be_bytes(bytes),
            })
        }
    };
}

impl<'a> Reader<'a> {
    /// Creates a reader at the start of `data`. `what` names the structure in errors.
    pub(crate) fn new(data: &'a [u8], endian: Endian, what: &'static str) -> Self {
        Reader {
            data,
            pos: 0,
            endian,
            what,
        }
    }

    pub(crate) fn endian(&self) -> Endian {
        self.endian
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn len(&self) -> usize {
        self.data.len()
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub(crate) fn set_pos(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(self.truncated(pos - self.pos));
        }
        self.pos = pos;
        Ok(())
    }

    fn truncated(&self, needed: usize) -> Error {
        Error::Truncated {
            what: self.what,
            offset: self.pos as u64,
            needed: needed as u64,
            available: self.remaining() as u64,
        }
    }

    /// Returns the next `n` bytes and advances past them.
    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(self.truncated(n));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    pub(crate) fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    /// Advances to the next multiple of `n` from the start of the slice.
    pub(crate) fn align(&mut self, n: usize) -> Result<()> {
        let rem = self.pos % n;
        if rem != 0 {
            self.skip(n - rem)?;
        }
        Ok(())
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub(crate) fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }

    read_int!(u16, u16);
    read_int!(i16, i16);
    read_int!(u32, u32);
    read_int!(i32, i32);
    read_int!(u64, u64);
    read_int!(i64, i64);

    pub(crate) fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub(crate) fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }

    /// Reads bytes up to a zero byte and advances past the zero byte.
    pub(crate) fn cstr(&mut self) -> Result<&'a [u8]> {
        let rest = &self.data[self.pos..];
        let Some(end) = rest.iter().position(|&b| b == 0) else {
            return Err(Error::malformed(
                self.what,
                format!("unterminated string at offset {}", self.pos),
            ));
        };
        self.pos += end + 1;
        Ok(&rest[..end])
    }

    /// Reads a zero-terminated string that must be valid UTF-8.
    pub(crate) fn cstring(&mut self) -> Result<String> {
        let at = self.pos;
        let bytes = self.cstr()?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| Error::malformed(self.what, format!("string at offset {at} is not UTF-8")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_byte_orders() {
        let data = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(
            Reader::new(&data, Endian::Little, "t").u32().unwrap(),
            0x0403_0201
        );
        assert_eq!(
            Reader::new(&data, Endian::Big, "t").u32().unwrap(),
            0x0102_0304
        );
    }

    #[test]
    fn align_is_relative_to_the_slice() {
        let data = [0u8; 8];
        let mut r = Reader::new(&data[1..], Endian::Little, "t");
        r.skip(1).unwrap();
        r.align(4).unwrap();
        assert_eq!(r.pos(), 4);
    }

    #[test]
    fn short_reads_fail() {
        let data = [0u8; 3];
        let err = Reader::new(&data, Endian::Little, "t").u32().unwrap_err();
        assert_eq!(err.code(), "truncated_file");
    }
}
