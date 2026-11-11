//! Deterministic uncompressed POSIX `ustar` archives for composite multi-member assets.
//!
//! Every member's payload sits contiguously inside the archive at a recorded `(offset, size)` byte
//! range, so a browser viewer can either fetch the whole `.tar` in one `GET` or slice one member
//! directly via `Range: bytes=<offset>-<offset + size - 1>`.

const BLOCK: usize = 512;
const MAX_OCTAL_SIZE: u64 = 0o77_777_777_777;

/// One member's byte span inside a packed `.tar` archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarMember {
    /// The relative path of the member inside the archive.
    pub path: String,
    /// The byte offset of the member's payload from the start of the archive.
    pub offset: u64,
    /// The byte length of the member's payload.
    pub size: u64,
}

/// Packs `(path, bytes)` entries into a deterministic uncompressed `.tar` archive in caller order.
pub fn pack(inputs: &[(&str, &[u8])]) -> (Vec<u8>, Vec<TarMember>) {
    let estimated: usize = inputs
        .iter()
        .map(|(_, data)| BLOCK * 2 + data.len().next_multiple_of(BLOCK))
        .sum::<usize>()
        + BLOCK * 2;
    let mut out = Vec::with_capacity(estimated);
    let mut members = Vec::with_capacity(inputs.len());

    for (index, (path, data)) in inputs.iter().enumerate() {
        let size = data.len() as u64;
        let ustar_split = split_ustar_path(path);
        let need_pax = ustar_split.is_none() || size > MAX_OCTAL_SIZE;
        if need_pax {
            let mut pax = Vec::new();
            if ustar_split.is_none() {
                append_pax_record(&mut pax, "path", path);
            }
            if size > MAX_OCTAL_SIZE {
                append_pax_record(&mut pax, "size", &size.to_string());
            }
            let pax_name = format!("PaxHeader/{index}");
            write_header(&mut out, &pax_name, "", pax.len() as u64, b'x');
            write_padded(&mut out, &pax);
        }

        let (name, prefix) = ustar_split.unwrap_or_else(|| (fallback_ascii_name(path, index), ""));
        let header_size = size.min(MAX_OCTAL_SIZE);
        write_header(&mut out, &name, prefix, header_size, b'0');
        let offset = out.len() as u64;
        write_padded(&mut out, data);
        members.push(TarMember {
            path: (*path).to_owned(),
            offset,
            size,
        });
    }

    // Standard two-block (1024-byte) zero end-of-archive marker.
    out.resize(out.len() + BLOCK * 2, 0);
    (out, members)
}

/// Lists the regular members of a `.tar` archive with their payload `(offset, size)` spans.
pub fn entries(archive: &[u8]) -> Result<Vec<TarMember>, String> {
    if !archive.len().is_multiple_of(BLOCK) {
        return Err(format!(
            "tar archive length {} is not a multiple of {BLOCK}",
            archive.len()
        ));
    }
    let mut pos = 0usize;
    let mut pending_path: Option<String> = None;
    let mut pending_size: Option<u64> = None;
    let mut out = Vec::new();

    while pos + BLOCK <= archive.len() {
        let header = &archive[pos..pos + BLOCK];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        verify_checksum(header)?;
        let typeflag = header[156];
        let octal_size = parse_octal(&header[124..136])?;
        let size = pending_size.take().unwrap_or(octal_size);
        let payload_start = pos + BLOCK;
        let payload_len =
            usize::try_from(size).map_err(|_| format!("tar member size {size} overflows usize"))?;
        let padded = payload_len
            .checked_next_multiple_of(BLOCK)
            .ok_or_else(|| "tar member padded size overflow".to_owned())?;
        if payload_start
            .checked_add(padded)
            .is_none_or(|end| end > archive.len())
        {
            return Err("tar member extends past end of archive".to_owned());
        }
        let payload = &archive[payload_start..payload_start + payload_len];
        match typeflag {
            b'x' => {
                parse_pax(payload, &mut pending_path, &mut pending_size)?;
            }
            b'0' | 0 => {
                let path = match pending_path.take() {
                    Some(p) => p,
                    None => read_ustar_path(header)?,
                };
                out.push(TarMember {
                    path,
                    offset: payload_start as u64,
                    size,
                });
            }
            other => {
                return Err(format!("unsupported tar entry type 0x{other:02x}"));
            }
        }
        pos = payload_start + padded;
    }
    Ok(out)
}

fn split_ustar_path(path: &str) -> Option<(String, &str)> {
    if path.is_empty() || !path.is_ascii() {
        return None;
    }
    if path.len() <= 100 {
        return Some((path.to_owned(), ""));
    }
    for (idx, ch) in path.char_indices() {
        if ch == '/' {
            let prefix = &path[..idx];
            let name = &path[idx + 1..];
            if !prefix.is_empty() && prefix.len() <= 155 && !name.is_empty() && name.len() <= 100 {
                return Some((name.to_owned(), prefix));
            }
        }
    }
    None
}

fn fallback_ascii_name(path: &str, index: usize) -> String {
    let base = path.rsplit('/').next().unwrap_or("entry");
    let clean: String = base
        .chars()
        .map(|c| if c.is_ascii_graphic() { c } else { '_' })
        .take(80)
        .collect();
    if clean.is_empty() {
        format!("entry_{index}")
    } else {
        clean
    }
}

fn append_pax_record(out: &mut Vec<u8>, key: &str, value: &str) {
    let suffix = format!(" {key}={value}\n");
    let mut total = suffix.len() + 1;
    loop {
        let digits = total.to_string().len();
        let candidate = digits + suffix.len();
        if candidate == total {
            out.extend_from_slice(format!("{total}{suffix}").as_bytes());
            break;
        }
        total = candidate;
    }
}

fn write_header(out: &mut Vec<u8>, name: &str, prefix: &str, size: u64, typeflag: u8) {
    let start = out.len();
    out.resize(start + BLOCK, 0);
    let hdr = &mut out[start..start + BLOCK];

    hdr[..name.len()].copy_from_slice(name.as_bytes());
    hdr[100..108].copy_from_slice(b"0000644\0");
    hdr[108..116].copy_from_slice(b"0000000\0");
    hdr[116..124].copy_from_slice(b"0000000\0");
    let size_octal = format!("{size:011o}\0");
    hdr[124..136].copy_from_slice(size_octal.as_bytes());
    hdr[136..148].copy_from_slice(b"00000000000\0");
    hdr[148..156].fill(b' ');
    hdr[156] = typeflag;
    hdr[257..263].copy_from_slice(b"ustar\0");
    hdr[263..265].copy_from_slice(b"00");
    if !prefix.is_empty() {
        hdr[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    }

    let checksum: u32 = hdr.iter().map(|&b| u32::from(b)).sum();
    let chk = format!("{checksum:06o}\0 ");
    hdr[148..156].copy_from_slice(chk.as_bytes());
}

fn write_padded(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(data);
    let rem = data.len() % BLOCK;
    if rem != 0 {
        out.resize(out.len() + (BLOCK - rem), 0);
    }
}

fn verify_checksum(header: &[u8]) -> Result<(), String> {
    let expected = parse_octal(&header[148..156])?;
    let actual: u64 = header
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                32
            } else {
                u64::from(b)
            }
        })
        .sum();
    if actual != expected {
        return Err(format!(
            "tar header checksum mismatch: expected {expected:o}, got {actual:o}"
        ));
    }
    Ok(())
}

fn parse_octal(field: &[u8]) -> Result<u64, String> {
    let trimmed = field
        .iter()
        .copied()
        .skip_while(|&b| b == b' ')
        .take_while(|&b| b != 0 && b != b' ')
        .collect::<Vec<_>>();
    let s = std::str::from_utf8(&trimmed).map_err(|_| "invalid octal field".to_owned())?;
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).map_err(|_| format!("invalid octal value {s:?}"))
}

fn cstr(slice: &[u8]) -> Result<&str, String> {
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    std::str::from_utf8(&slice[..end]).map_err(|_| "invalid UTF-8 in ustar path".to_owned())
}

fn read_ustar_path(header: &[u8]) -> Result<String, String> {
    let name = cstr(&header[..100])?;
    let prefix = if &header[257..262] == b"ustar" {
        cstr(&header[345..500])?
    } else {
        ""
    };
    if prefix.is_empty() {
        Ok(name.to_owned())
    } else {
        Ok(format!("{prefix}/{name}"))
    }
}

fn parse_pax(
    mut data: &[u8],
    pending_path: &mut Option<String>,
    pending_size: &mut Option<u64>,
) -> Result<(), String> {
    while !data.is_empty() {
        let space = data
            .iter()
            .position(|&b| b == b' ')
            .ok_or_else(|| "malformed PAX record".to_owned())?;
        let len_str =
            std::str::from_utf8(&data[..space]).map_err(|_| "invalid PAX length".to_owned())?;
        let len: usize = len_str
            .parse()
            .map_err(|_| "invalid PAX length number".to_owned())?;
        if len <= space + 1 || len > data.len() || data[len - 1] != b'\n' {
            return Err("truncated PAX record".to_owned());
        }
        let kv = std::str::from_utf8(&data[space + 1..len - 1])
            .map_err(|_| "non-UTF-8 PAX record".to_owned())?;
        if let Some((k, v)) = kv.split_once('=') {
            match k {
                "path" => *pending_path = Some(v.to_owned()),
                "size" => {
                    *pending_size = Some(
                        v.parse::<u64>()
                            .map_err(|_| "invalid PAX size".to_owned())?,
                    );
                }
                _ => {}
            }
        }
        data = &data[len..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{entries, pack};

    #[test]
    fn pack_and_slice_round_trip_covering_ustar_and_pax_paths() {
        let long_dir = "very_long_segment_".repeat(12);
        let long_path = format!("{long_dir}/deep/motions/001.json");
        let utf8_path = "chara/司_live2d/model.moc3";
        let inputs: Vec<(&str, &[u8])> = vec![
            ("model.json", br#"{"version":3}"#),
            ("textures/00.webp", &[0x52, 0x49, 0x46, 0x46, 1, 2, 3, 4, 5]),
            (&long_path, &[0xaa; 1025]),
            (utf8_path, b"MOC3_PAYLOAD"),
        ];
        let (archive, members) = pack(&inputs);
        let (archive_again, _) = pack(&inputs);
        assert_eq!(archive, archive_again, "tar packing must be deterministic");

        let parsed = entries(&archive).unwrap();
        assert_eq!(parsed, members);
        for (member, (_, expected)) in members.iter().zip(&inputs) {
            let start = member.offset as usize;
            let end = start + member.size as usize;
            assert!(
                start.is_multiple_of(512),
                "payload offsets are 512-byte aligned"
            );
            assert_eq!(&archive[start..end], *expected);
        }
    }
}
