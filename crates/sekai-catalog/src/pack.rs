//! Packs: many objects in one stream, so that they move between stores in one transfer.
//!
//! A pack is a sequence of objects. Each is a line with its extension and its length in bytes,
//! separated by a space, followed by its bytes. An object is stored under the key of its bytes, so
//! a pack names no keys, and what it holds cannot be stored under a key it does not match.

use std::io::{self, BufRead, Write};

use crate::store::{Key, Store};

/// Writes the objects `keys` of `store` to `out` as a pack.
pub fn write<'k>(
    store: &dyn Store,
    keys: impl IntoIterator<Item = &'k Key>,
    out: &mut impl Write,
) -> io::Result<()> {
    for key in keys {
        let bytes = store.read(key)?;
        writeln!(out, "{} {}", key.extension, bytes.len())?;
        out.write_all(&bytes)?;
    }
    Ok(())
}

/// Stores every object of the pack `input` in `store` and returns how many there were.
pub fn read(input: &mut impl BufRead, store: &dyn Store) -> io::Result<usize> {
    let mut count = 0;
    let mut line = String::new();
    while input.read_line(&mut line)? > 0 {
        let (extension, length) = header(&line)?;
        let mut bytes = vec![0; length];
        input.read_exact(&mut bytes)?;
        store.put(&bytes, extension)?;
        count += 1;
        line.clear();
    }
    Ok(count)
}

/// The extension and the length of the object that the header `line` announces.
fn header(line: &str) -> io::Result<(&str, usize)> {
    line.strip_suffix('\n')
        .and_then(|line| line.split_once(' '))
        .and_then(|(extension, length)| Some((extension, length.parse().ok()?)))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("a pack announces an object as {line:?}"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    #[test]
    fn a_pack_carries_its_objects_to_another_store() {
        let from = MemoryStore::default();
        let keys = [
            from.put(b"{\"a\":1}\n", "json").unwrap(),
            from.put(b"", "bin").unwrap(),
            from.put(b"two\nlines", "txt").unwrap(),
        ];
        let mut pack = Vec::new();
        write(&from, &keys, &mut pack).unwrap();

        let to = MemoryStore::default();
        assert_eq!(read(&mut pack.as_slice(), &to).unwrap(), 3);
        for key in &keys {
            assert_eq!(to.read(key).unwrap(), from.read(key).unwrap());
        }
    }

    #[test]
    fn a_damaged_pack_is_an_error() {
        let store = MemoryStore::default();
        for damaged in [&b"json\n{}"[..], b"json 5\n{}", b"json 2"] {
            let error = read(&mut &damaged[..], &store).unwrap_err();
            assert!(
                [io::ErrorKind::InvalidData, io::ErrorKind::UnexpectedEof].contains(&error.kind()),
                "{damaged:?}: {error}"
            );
        }
    }
}
