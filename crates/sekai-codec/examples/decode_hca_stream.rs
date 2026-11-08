//! Decodes HCA streams from standard input, for comparing with a reference decoder.
//!
//! Each request is a little-endian `u32` length and that many bytes. Each reply is a status byte:
//! 0, then little-endian `u32` sample rate, `u32` channel count, `u32` length in bytes and the
//! interleaved 16-bit samples, or 1, then a little-endian `u32` length and the error text.

use std::io::{self, Read, Write};

use sekai_codec::cri::decode_hca;

fn main() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut length = [0; 4];
    while input.read_exact(&mut length).is_ok() {
        let mut data = vec![0; u32::from_le_bytes(length) as usize];
        input.read_exact(&mut data)?;
        match decode_hca(&data, None) {
            Ok(pcm) => {
                output.write_all(&[0])?;
                output.write_all(&pcm.sample_rate().to_le_bytes())?;
                output.write_all(&u32::from(pcm.channels()).to_le_bytes())?;
                let bytes: Vec<u8> = pcm
                    .samples()
                    .iter()
                    .flat_map(|sample| sample.to_le_bytes())
                    .collect();
                output.write_all(&(bytes.len() as u32).to_le_bytes())?;
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
    Ok(())
}
