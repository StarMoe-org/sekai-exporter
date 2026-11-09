//! The reader of TPK files, on files written by the tests.

use sekai_layouts::VersionTable;
use sekai_unity::{ClassLayouts, TypeTreeNode};

const CLASS: i32 = 21;
const NO_COMPRESSION: u8 = 0;
const LZMA: u8 = 2;

/// The number a table stores the final release `major.0.0f<release>` as.
fn version(major: u64, release: u64) -> u64 {
    (major << 48) | (3 << 8) | release
}

fn u16s(out: &mut Vec<u8>, values: &[u16]) {
    for value in values {
        out.extend(value.to_le_bytes());
    }
}

/// The entry of a class from `from` on: `Some(root)` when the class exists, the root being absent
/// when it has no release layout.
fn class_entry(out: &mut Vec<u8>, from: u64, class: Option<Option<u16>>) {
    out.extend(from.to_le_bytes());
    out.push(u8::from(class.is_some()));
    if let Some(root) = class {
        u16s(out, &[0, 0]);
        out.push(0x40 | if root.is_some() { 0x80 } else { 0 });
        u16s(out, &[9]); // the editor root
        if let Some(root) = root {
            u16s(out, &[root]);
        }
    }
}

fn node(out: &mut Vec<u8>, type_name: u16, name: u16, version: u16, subnodes: &[u16]) {
    u16s(out, &[type_name, name]);
    out.extend((i32::from(type_name) * 10).to_le_bytes());
    u16s(out, &[version]);
    out.push(1);
    out.extend(0x4000u32.to_le_bytes());
    u16s(out, &[subnodes.len() as u16]);
    u16s(out, subnodes);
}

/// A blob of format `format` with one class: at 2000 and 2002 root node 1, at 2001 no release
/// layout and from 2003 no class. Node 0 is unused, and node 1's first child comes after its second.
fn blob(format: u8) -> Vec<u8> {
    let mut out = 0i64.to_le_bytes().to_vec();
    out.extend(1i32.to_le_bytes());
    out.extend(version(2000, 1).to_le_bytes());
    out.extend(1i32.to_le_bytes());
    out.extend(CLASS.to_le_bytes());
    out.extend(4i32.to_le_bytes());
    class_entry(&mut out, version(2000, 1), Some(Some(1)));
    class_entry(&mut out, version(2001, 1), Some(None));
    class_entry(&mut out, version(2002, 1), Some(Some(1)));
    class_entry(&mut out, version(2003, 1), None);
    out.extend(1i32.to_le_bytes());
    if format == 1 {
        out.extend(version(2000, 1).to_le_bytes());
        out.push(1);
        out.extend(1i32.to_le_bytes());
        u16s(&mut out, &[0]);
    } else {
        out.extend(version(2000, 1).to_le_bytes());
        out.extend(1i32.to_le_bytes());
        u16s(&mut out, &[0, 0]);
    }
    out.extend(5i32.to_le_bytes());
    node(&mut out, 0, 0, 0, &[]);
    node(&mut out, 1, 2, 7, &[3, 2]);
    node(&mut out, 3, 4, 1, &[]);
    node(&mut out, 5, 6, 1, &[4]);
    node(&mut out, 7, 8, 1, &[]);
    let strings = [
        "unused", "Material", "Base", "int", "m_Last", "string", "m_Name", "Array", "data",
    ];
    out.extend((strings.len() as i32).to_le_bytes());
    for string in strings {
        out.push(string.len() as u8);
        out.extend(string.as_bytes());
    }
    out
}

fn file(format: u8, compression: u8, stored: &[u8], size: usize) -> Vec<u8> {
    let mut out = b"TPK*".to_vec();
    out.extend([format, compression, 0, 0, 0, 0, 0, 0]);
    out.extend((stored.len() as u32).to_le_bytes());
    out.extend((size as u32).to_le_bytes());
    out.extend(stored);
    out
}

/// `blob` as LZMA data without a stored length.
fn lzma(blob: &[u8]) -> Vec<u8> {
    let mut alone = Vec::new();
    lzma_rs::lzma_compress(&mut &blob[..], &mut alone).unwrap();
    [&alone[..5], &alone[13..]].concat()
}

fn table() -> VersionTable {
    let blob = blob(2);
    VersionTable::from_tpk(&file(2, NO_COMPRESSION, &blob, blob.len())).unwrap()
}

fn has_tree(unity_version: &str) -> bool {
    table().tree(CLASS, unity_version).is_some()
}

#[test]
fn every_format_and_compression_reads_the_same() {
    let expected = table().tree(CLASS, "2000.0.0f1").unwrap();
    for format in [1, 2] {
        let blob = blob(format);
        for (compression, stored) in [(NO_COMPRESSION, blob.clone()), (LZMA, lzma(&blob))] {
            let table =
                VersionTable::from_tpk(&file(format, compression, &stored, blob.len())).unwrap();
            assert_eq!(table.tree(CLASS, "2000.0.0f1").unwrap(), expected);
        }
    }
}

#[test]
fn the_nodes_are_in_preorder_with_their_values() {
    let tree = table().tree(CLASS, "2000.0.0f1").unwrap();
    let names: Vec<(u8, &str, &str)> = tree
        .nodes()
        .iter()
        .map(|n| (n.level, n.type_name.as_str(), n.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            (0, "Material", "Base"),
            (1, "string", "m_Name"),
            (2, "Array", "data"),
            (1, "int", "m_Last"),
        ]
    );
    assert_eq!(
        tree.nodes()[1],
        TypeTreeNode {
            version: 1,
            level: 1,
            type_flags: 1,
            type_name: "string".into(),
            name: "m_Name".into(),
            byte_size: 50,
            index: 1,
            meta_flags: 0x4000,
            ref_type_hash: 0,
        }
    );
}

#[test]
fn a_version_takes_the_last_entry_it_reaches() {
    assert!(!has_tree("1999.0.0f1"), "earlier than every entry");
    assert!(!has_tree("2000.0.0f0"), "just before the first entry");
    assert!(has_tree("2000.0.0f1"), "exactly the first entry");
    assert!(has_tree("2000.5.3f2"), "between two entries");
    assert!(!has_tree("2001.2.0f1"), "an entry without a release layout");
    assert!(has_tree("2002.1.0p3"), "the entry before the last");
    assert!(!has_tree("2500.0.0f1"), "after the class ended");
}

#[test]
fn release_types_order_a_version() {
    assert!(!has_tree("2000.0.0a1"));
    assert!(!has_tree("2000.0.0b9"));
    assert!(has_tree("2000.0.0p1"));
    assert!(has_tree("2000.0.1"));
}

#[test]
fn an_unknown_class_or_an_unreadable_version_has_no_tree() {
    assert!(table().tree(114, "2000.0.0f1").is_none());
    assert!(!has_tree("not a version"));
    assert!(!has_tree("2000.0"));
}

fn error(file: &[u8]) -> String {
    VersionTable::from_tpk(file).unwrap_err().to_string()
}

#[test]
fn a_bad_file_says_what_is_wrong() {
    let blob = blob(2);
    let good = file(2, NO_COMPRESSION, &blob, blob.len());
    let mut magic = good.clone();
    magic[0] = 0;
    assert!(error(&magic).contains("TPK*"));
    assert!(error(&good[..good.len() - 1]).contains("ends before"));
    assert!(error(&good[..3]).contains("ends before"));
    assert!(error(&file(2, 3, &blob, blob.len())).contains("compression type 3"));
    assert!(error(&file(3, 0, &blob, blob.len())).contains("format version 3"));
    assert!(error(&file(2, 0, &blob, blob.len() + 1)).contains("bytes instead of"));
    let mut json = good.clone();
    json[6] = 3;
    assert!(error(&json).contains("data type 3"));
}
