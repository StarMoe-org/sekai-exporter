//! Lists what the reader finds in cue sheets, for comparing with a reference.
//!
//! Each argument is an ACB file. For each stream of its memory bank, one line is printed:
//! `stream <path> <offset> <length> <wave id> <cue names joined by ;>`. A line per sheet then
//! states `sheet <path> <cues> <waveforms> <streaming waveforms>`.

use std::fs;

use sekai_codec::cri::read_acb;

fn main() -> std::io::Result<()> {
    for path in std::env::args().skip(1) {
        let data = fs::read(&path)?;
        let acb = match read_acb(&data) {
            Ok(acb) => acb,
            Err(error) => {
                println!("error {path} {error}");
                continue;
            }
        };
        if let Some(bank) = &acb.memory_awb {
            for stream in &bank.streams {
                let mut names = Vec::new();
                for cue in &acb.cues {
                    let plays = cue
                        .waveforms
                        .iter()
                        .any(|&w| acb.waveforms[w].memory_awb_id == Some(stream.wave_id));
                    if plays {
                        names.extend(cue.cue_names.iter().cloned());
                    }
                }
                let offset = stream.data.as_ptr() as usize - data.as_ptr() as usize;
                let length = stream.data.len();
                let wave_id = stream.wave_id;
                println!(
                    "stream {path} {offset} {length} {wave_id} {}",
                    names.join(";")
                );
            }
        }
        let streaming = acb.waveforms.iter().filter(|w| w.streaming != 0).count();
        println!(
            "sheet {path} {} {} {streaming}",
            acb.cues.len(),
            acb.waveforms.len()
        );
    }
    Ok(())
}
