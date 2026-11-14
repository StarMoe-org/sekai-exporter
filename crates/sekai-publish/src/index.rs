//! Checks that the objects a publication refers to are in a store.

use std::io;

use sekai_catalog::{Key, Objects};

use crate::parallel;
use crate::plan::PublishError;

/// The first of `files` that is not among `objects`; `key` gives the key and size of a file. The
/// files are checked against listings of the objects, a store directory at a time on up to
/// `threads` threads.
pub(crate) fn missing<'a, T: Sync>(
    objects: &Objects<'_>,
    files: &'a [T],
    threads: usize,
    key: impl Fn(&T) -> (&Key, u64) + Sync,
) -> Result<Option<&'a T>, PublishError> {
    let mut by_prefix: Vec<(u8, Vec<usize>)> =
        (0..=u8::MAX).map(|prefix| (prefix, Vec::new())).collect();
    for (index, file) in files.iter().enumerate() {
        by_prefix[usize::from(key(file).0.sha256[0])].1.push(index);
    }
    let first_missing = parallel::map(&by_prefix, threads, |(prefix, indices)| {
        if indices.is_empty() {
            return Ok(None);
        }
        let sizes = objects.sizes(*prefix)?;
        Ok(indices.iter().copied().find(|&index| {
            let (key, size) = key(&files[index]);
            sizes.get(key) != Some(&size)
        }))
    });
    let first_missing = first_missing
        .into_iter()
        .collect::<io::Result<Vec<_>>>()
        .map_err(PublishError::wrap("verify"))?
        .into_iter()
        .flatten()
        .min();
    Ok(first_missing.map(|index| &files[index]))
}
