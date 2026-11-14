//! Rows in key order cut into chunks whose ends depend only on the rows, and patches of such a
//! sequence: a change to some rows rewrites only the chunks around them, and every other chunk is
//! the one a cut of the whole sequence gives.
//!
//! A chunk ends after a row that [`Row::ends_chunk`] marks, and after its [`Row::MOST`]th row
//! otherwise. Because a chunk end depends only on the rows since the previous end, the chunks
//! before a change and the chunks after the first end past it are the same as before the change.

use std::collections::BTreeMap;
use std::iter::Peekable;
use std::mem;

use sha2::{Digest, Sha256};

/// A row of a chunked sequence.
pub trait Row {
    /// What orders the rows; no two rows of a sequence have the same key.
    type Key: Ord + Clone;

    /// A chunk ends after this many rows in any case.
    const MOST: usize;

    /// The key of the row.
    fn key(&self) -> &Self::Key;

    /// Whether a chunk ends after this row, usually [`ends_after`] of the bytes of its key.
    fn ends_chunk(&self) -> bool;
}

/// Whether a chunk whose rows number `average` on average ends after a row named by `bytes`:
/// whether the first four bytes of their SHA-256, read as a big-endian number, are a multiple of
/// `average`.
pub fn ends_after(bytes: &[u8], average: u32) -> bool {
    let digest = Sha256::digest(bytes);
    u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]).is_multiple_of(average)
}

/// A chunk of a patched sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Patched<R> {
    /// The chunk at this position of the sequence before the patch, unchanged.
    Kept(usize),
    /// A chunk with these rows.
    New(Vec<R>),
}

/// `rows`, in key order, cut into chunks.
pub fn cut<R: Row>(rows: Vec<R>) -> Vec<Vec<R>> {
    let mut cutter = Cutter::default();
    let mut chunks: Vec<Vec<R>> = rows
        .into_iter()
        .filter_map(|row| cutter.push(row))
        .collect();
    chunks.extend(cutter.rest());
    chunks
}

/// The chunks of a sequence after `edits`, which put a row under its key (`Some`) or remove the
/// row of the key (`None`). The sequence before the patch has chunks whose first keys are
/// `firsts`, and `load` gives the rows of the chunk at a position. Only the chunks that an edit
/// falls in, and those that follow them up to the next chunk end, are loaded.
pub fn patch<R: Row, E>(
    firsts: &[R::Key],
    mut load: impl FnMut(usize) -> Result<Vec<R>, E>,
    edits: BTreeMap<R::Key, Option<R>>,
) -> Result<Vec<Patched<R>>, E> {
    let mut edits = edits.into_iter().peekable();
    let mut cutter = Cutter::default();
    let mut patched = Vec::new();
    for (position, next) in (0..firsts.len()).map(|p| (p, firsts.get(p + 1))) {
        let before_next = |key: &R::Key| next.is_none_or(|next| key < next);
        let touched = edits.peek().is_some_and(|(key, _)| before_next(key));
        if !touched && cutter.is_empty() {
            patched.push(Patched::Kept(position));
            continue;
        }
        let rows = load(position)?;
        let mine = take_while(&mut edits, before_next);
        for row in merged(rows, mine) {
            patched.extend(cutter.push(row).map(Patched::New));
        }
    }
    // With no chunks before the patch, every edit falls in the first chunk.
    for row in merged(Vec::new(), edits.collect()) {
        patched.extend(cutter.push(row).map(Patched::New));
    }
    patched.extend(cutter.rest().map(Patched::New));
    Ok(patched)
}

/// The rows of the chunk being cut.
struct Cutter<R>(Vec<R>);

impl<R> Default for Cutter<R> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<R: Row> Cutter<R> {
    /// Adds `row`, and gives the chunk it ends.
    fn push(&mut self, row: R) -> Option<Vec<R>> {
        let ends = row.ends_chunk();
        self.0.push(row);
        (ends || self.0.len() == R::MOST).then(|| mem::take(&mut self.0))
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The last chunk, which no row ended.
    fn rest(self) -> Option<Vec<R>> {
        (!self.0.is_empty()).then_some(self.0)
    }
}

/// The edits of `edits` whose keys `before` accepts, taken from its front.
fn take_while<K, V>(
    edits: &mut Peekable<impl Iterator<Item = (K, V)>>,
    before: impl Fn(&K) -> bool,
) -> Vec<(K, V)> {
    let mut taken = Vec::new();
    while let Some(edit) = edits.next_if(|(key, _)| before(key)) {
        taken.push(edit);
    }
    taken
}

/// `rows` with `edits` applied, both in key order.
fn merged<R: Row>(rows: Vec<R>, edits: Vec<(R::Key, Option<R>)>) -> Vec<R> {
    let mut rows = rows.into_iter().peekable();
    let mut out = Vec::new();
    for (key, edit) in edits {
        while let Some(row) = rows.next_if(|row| row.key() < &key) {
            out.push(row);
        }
        rows.next_if(|row| row.key() == &key);
        out.extend(edit);
    }
    out.extend(rows);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Item {
        key: u32,
        value: u32,
    }

    impl Row for Item {
        type Key = u32;

        const MOST: usize = 8192;

        fn key(&self) -> &u32 {
            &self.key
        }

        fn ends_chunk(&self) -> bool {
            ends_after(&self.key.to_be_bytes(), 64)
        }
    }

    /// A row that never ends a chunk, so that chunks end at [`Row::MOST`].
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Long(u32);

    impl Row for Long {
        type Key = u32;

        const MOST: usize = 16;

        fn key(&self) -> &u32 {
            &self.0
        }

        fn ends_chunk(&self) -> bool {
            false
        }
    }

    /// A small deterministic generator.
    struct Random(u64);

    impl Random {
        fn below(&mut self, n: u32) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) % u64::from(n)) as u32
        }
    }

    /// The chunks of `patched`, with kept chunks taken from `before`.
    fn resolve<R: Clone>(patched: Vec<Patched<R>>, before: &[Vec<R>]) -> Vec<Vec<R>> {
        patched
            .into_iter()
            .map(|chunk| match chunk {
                Patched::Kept(position) => before[position].clone(),
                Patched::New(rows) => rows,
            })
            .collect()
    }

    /// Patches `rows` with `edits`, checks the result against a cut of the edited rows, and
    /// returns how many chunks were loaded.
    fn check<R: Row + Clone + PartialEq + std::fmt::Debug>(
        rows: Vec<R>,
        edits: BTreeMap<R::Key, Option<R>>,
    ) -> usize {
        let before = cut(rows.clone());
        let firsts: Vec<R::Key> = before.iter().map(|c| c[0].key().clone()).collect();
        let mut loaded = 0;
        let load = |position: usize| -> Result<Vec<R>, ()> {
            loaded += 1;
            Ok(before[position].clone())
        };
        let patched = patch(&firsts, load, edits.clone()).unwrap();
        let mut expected: BTreeMap<R::Key, R> =
            rows.into_iter().map(|r| (r.key().clone(), r)).collect();
        for (key, edit) in edits {
            match edit {
                Some(row) => expected.insert(key, row),
                None => expected.remove(&key),
            };
        }
        assert_eq!(
            resolve(patched, &before),
            cut(expected.into_values().collect())
        );
        loaded
    }

    fn items(keys: impl Iterator<Item = u32>) -> Vec<Item> {
        keys.map(|key| Item { key, value: 0 }).collect()
    }

    #[test]
    fn a_patch_gives_the_chunks_of_a_cut_of_the_patched_rows() {
        let mut random = Random(7);
        for round in 0..200 {
            let count = random.below(3000);
            let rows = items((0..count).map(|k| k * 4));
            let mut edits = BTreeMap::new();
            for _ in 0..random.below(12) {
                let key = random.below(count * 4 + 8);
                let edit = (random.below(3) > 0).then_some(Item { key, value: round });
                edits.insert(key, edit);
            }
            check(rows, edits);
        }
    }

    #[test]
    fn a_change_loads_only_the_chunks_around_it() {
        let rows = items((0..100_000).map(|k| k * 2));
        let chunks = cut(rows.clone()).len();
        let edit = Item {
            key: 100_001,
            value: 1,
        };
        let loaded = check(rows, BTreeMap::from([(edit.key, Some(edit))]));
        assert!(chunks > 1000);
        assert_eq!(loaded, 1);
    }

    #[test]
    fn removing_a_chunk_end_joins_the_chunk_to_the_next() {
        let rows = items(0..20_000);
        // The end of a chunk that has other rows, so that the rest of the chunk joins the next.
        let end = rows
            .windows(2)
            .find(|pair| !pair[0].ends_chunk() && pair[1].ends_chunk())
            .unwrap()[1]
            .key;
        let loaded = check(rows, BTreeMap::from([(end, None)]));
        assert_eq!(loaded, 2);
    }

    #[test]
    fn chunks_without_ends_are_cut_at_the_most_rows() {
        let rows: Vec<Long> = (0..3 * 16 + 5).map(|k| Long(k * 2)).collect();
        assert_eq!(
            cut(rows.clone()).iter().map(Vec::len).collect::<Vec<_>>(),
            [16, 16, 16, 5]
        );
        check(rows.clone(), BTreeMap::from([(1, Some(Long(1)))]));
        check(rows.clone(), BTreeMap::from([(0, None)]));
        check(rows, BTreeMap::from([(65, Some(Long(65)))]));
    }

    #[test]
    fn edits_of_an_empty_sequence_are_cut() {
        let edits = items(0..500)
            .into_iter()
            .map(|r| (r.key, Some(r)))
            .collect();
        check(Vec::new(), edits);
        check(Vec::new(), BTreeMap::from([(3, None::<Item>)]));
    }
}
