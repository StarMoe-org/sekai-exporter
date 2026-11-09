//! Decodes textures read from standard input and writes the results to standard output, for
//! comparing the decoders with reference implementations (`tools/compare-texture`).
//!
//! Request: little-endian `format: i32`, `width: u32`, `height: u32`, `length: u32`, then the
//! data. Reply: one status byte (0 decoded, 1 failed) and, when decoded, the
//! `width * height * 4` bytes of the surface, or for an HDR format the `width * height * 8` bytes
//! of its little-endian binary16 values; when failed, a little-endian `length: u32` and the
//! UTF-8 error.

use std::io::{self, Read, Write};

use sekai_codec::{DecodeError, texture};

/// The bytes of the decoded image.
fn decode(format: i32, width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>, DecodeError> {
    if texture::is_hdr(format) {
        let surface = texture::decode_hdr(format, width, height, data)?;
        return Ok(surface
            .pixels()
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect());
    }
    texture::decode(format, width, height, data).map(|surface| surface.into_pixels())
}

fn main() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::BufWriter::new(io::stdout().lock());
    let mut header = [0; 16];
    let mut data = Vec::new();
    loop {
        match input.read_exact(&mut header) {
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            result => result?,
        }
        let field = |i: usize| u32::from_le_bytes(header[i..i + 4].try_into().expect("four bytes"));
        data.resize(field(12) as usize, 0);
        input.read_exact(&mut data)?;
        match decode(field(0) as i32, field(4), field(8), &data) {
            Ok(bytes) => {
                output.write_all(&[0])?;
                output.write_all(&bytes)?;
            }
            Err(error) => {
                let text = error.to_string();
                output.write_all(&[1])?;
                output.write_all(&(text.len() as u32).to_le_bytes())?;
                output.write_all(text.as_bytes())?;
            }
        }
        output.flush()?;
    }
}
