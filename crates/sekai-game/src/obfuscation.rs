//! The header obfuscation of this game's bundles.
//!
//! Downloaded bundles start with a four-byte marker instead of the UnityFS signature. With the
//! marker `10 00 00 00`, the first 128 bytes after it are inverted in groups of eight: the first
//! five bytes of each group. With the marker `20 00 00 00`, the marker is the only change.
//! Bundles that already start with the UnityFS signature are stored as they are.

use std::fmt;
use std::io::{self, Write};

use crate::sink::Sink;

const SIGNATURE: &[u8] = b"UnityFS\0";
const INVERTED: [u8; 4] = [0x10, 0, 0, 0];
const PREFIXED: [u8; 4] = [0x20, 0, 0, 0];
const INVERTED_LEN: usize = 128;

/// The bundle starts with none of the known markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownObfuscation {
    /// The first bytes of the bundle.
    pub head: Vec<u8>,
}

impl fmt::Display for UnknownObfuscation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown bundle header {:02x?}", self.head)
    }
}

impl std::error::Error for UnknownObfuscation {}

/// Restores a downloaded bundle to its UnityFS bytes, in place.
pub fn deobfuscate(data: &mut Vec<u8>) -> Result<(), UnknownObfuscation> {
    if data.starts_with(SIGNATURE) {
        return Ok(());
    }
    let marker = data.get(..4).unwrap_or_default();
    if marker == PREFIXED {
        data.drain(..4);
    } else if marker == INVERTED && data.len() >= 4 + INVERTED_LEN {
        data.drain(..4);
        for (i, b) in data[..INVERTED_LEN].iter_mut().enumerate() {
            if i % 8 < 5 {
                *b = !*b;
            }
        }
    } else {
        return Err(UnknownObfuscation {
            head: data.iter().take(8).copied().collect(),
        });
    }
    if !data.starts_with(SIGNATURE) {
        return Err(UnknownObfuscation {
            head: data.iter().take(8).copied().collect(),
        });
    }
    Ok(())
}

/// A writer that restores a bundle while it streams: only the head of a bundle is obfuscated.
pub struct Deobfuscating<W> {
    inner: W,
    head: Vec<u8>,
    released: bool,
}

/// The bytes that decide and contain the obfuscation.
const HEAD_LEN: usize = 4 + INVERTED_LEN;

impl<W: Write> Deobfuscating<W> {
    /// Writes the restored bundle to `inner`.
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            head: Vec::with_capacity(HEAD_LEN),
            released: false,
        }
    }

    fn release(&mut self) -> io::Result<()> {
        let mut head = std::mem::take(&mut self.head);
        deobfuscate(&mut head).map_err(io::Error::other)?;
        self.released = true;
        self.inner.write_all(&head)
    }

    /// Ends the bundle and returns the writer.
    pub fn finish(mut self) -> io::Result<W> {
        if !self.released {
            self.release()?;
        }
        Ok(self.inner)
    }
}

impl<W: Write> Write for Deobfuscating<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.released {
            return self.inner.write(bytes);
        }
        let taken = bytes.len().min(HEAD_LEN - self.head.len());
        self.head.extend_from_slice(&bytes[..taken]);
        if self.head.len() == HEAD_LEN {
            self.release()?;
        }
        Ok(taken)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<W: Sink> Sink for Deobfuscating<W> {
    fn reset(&mut self) -> io::Result<()> {
        self.head.clear();
        self.released = false;
        self.inner.reset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> Vec<u8> {
        let mut b = SIGNATURE.to_vec();
        b.resize(200, 7);
        b
    }

    #[test]
    fn inverts_five_of_every_eight_bytes() {
        let plain = bundle();
        let mut obfuscated = INVERTED.to_vec();
        obfuscated.extend(
            plain.iter().enumerate().map(
                |(i, &b)| {
                    if i < INVERTED_LEN && i % 8 < 5 { !b } else { b }
                },
            ),
        );
        deobfuscate(&mut obfuscated).unwrap();
        assert_eq!(obfuscated, plain);
    }

    #[test]
    fn strips_the_plain_marker_and_keeps_signed_bundles() {
        let plain = bundle();
        let mut prefixed = PREFIXED.to_vec();
        prefixed.extend_from_slice(&plain);
        deobfuscate(&mut prefixed).unwrap();
        assert_eq!(prefixed, plain);
        let mut same = plain.clone();
        deobfuscate(&mut same).unwrap();
        assert_eq!(same, plain);
        assert!(deobfuscate(&mut vec![1, 2, 3, 4, 5]).is_err());
    }

    impl Sink for Vec<u8> {
        fn reset(&mut self) -> io::Result<()> {
            self.clear();
            Ok(())
        }
    }

    fn obfuscated(plain: &[u8]) -> Vec<u8> {
        let mut bytes = INVERTED.to_vec();
        bytes.extend(
            plain.iter().enumerate().map(
                |(i, &b)| {
                    if i < INVERTED_LEN && i % 8 < 5 { !b } else { b }
                },
            ),
        );
        bytes
    }

    #[test]
    fn streaming_gives_the_same_bytes_at_any_split() {
        let plain = bundle();
        let bytes = obfuscated(&plain);
        for size in [1, 3, 131, 132, 133, bytes.len()] {
            let mut writer = Deobfuscating::new(Vec::new());
            for chunk in bytes.chunks(size) {
                writer.write_all(chunk).unwrap();
            }
            assert_eq!(writer.finish().unwrap(), plain, "chunks of {size}");
        }
    }

    #[test]
    fn a_short_bundle_is_released_at_the_end() {
        let mut bytes = PREFIXED.to_vec();
        bytes.extend_from_slice(SIGNATURE);
        let mut writer = Deobfuscating::new(Vec::new());
        writer.write_all(&bytes).unwrap();
        assert_eq!(writer.finish().unwrap(), SIGNATURE);
    }

    #[test]
    fn reset_starts_the_bundle_again() {
        let plain = bundle();
        let bytes = obfuscated(&plain);
        let mut writer = Deobfuscating::new(Vec::new());
        writer.write_all(&bytes[..150]).unwrap();
        writer.reset().unwrap();
        writer.write_all(&bytes).unwrap();
        assert_eq!(writer.finish().unwrap(), plain);
    }

    #[test]
    fn an_unknown_head_is_an_error_of_the_write() {
        let mut writer = Deobfuscating::new(Vec::new());
        assert!(writer.write_all(&[9; 200]).is_err());
    }
}
