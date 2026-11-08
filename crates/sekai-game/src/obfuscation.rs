//! The header obfuscation of this game's bundles.
//!
//! Downloaded bundles start with a four-byte marker instead of the UnityFS signature. With the
//! marker `10 00 00 00`, the first 128 bytes after it are inverted in groups of eight: the first
//! five bytes of each group. With the marker `20 00 00 00`, the marker is the only change.
//! Bundles that already start with the UnityFS signature are stored as they are.

use std::fmt;

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
}
