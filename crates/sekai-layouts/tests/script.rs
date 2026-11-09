//! Tables of script layouts.

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;
use sekai_layouts::{PlayerLayouts, ScriptTable, VersionTable};
use sekai_unity::{ClassLayouts, TypeTreeNode};

const VERSION: &str = "2022.3.62f2";
const HEADER_NODES: usize = 2;

fn gzipped(text: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

/// A line of a table with one `int` field called `name`.
fn script_line(class: &str, name: &str) -> String {
    format!(
        r#"{{"assembly":"Assembly-CSharp","class":"{class}","nodes":[{{"type":"int","name":"{name}","level":1,"byteSize":4,"typeFlags":0,"version":1,"metaFlag":16384}}]}}"#
    )
}

fn u16_bytes(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn u32_bytes(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn node(out: &mut Vec<u8>, type_name: u16, name: u16, subnodes: &[u16]) {
    u16_bytes(out, type_name);
    u16_bytes(out, name);
    u32_bytes(out, 4);
    u16_bytes(out, 1);
    out.push(0);
    u32_bytes(out, 0);
    u16_bytes(out, subnodes.len() as u16);
    for subnode in subnodes {
        u16_bytes(out, *subnode);
    }
}

fn string(out: &mut Vec<u8>, text: &str) {
    out.push(text.len() as u8);
    out.extend_from_slice(text.as_bytes());
}

/// A table with a `MonoBehaviour` of a root and one field, from Unity 2017.
fn classes() -> VersionTable {
    let mut blob = vec![0; 8];
    u32_bytes(&mut blob, 0);
    u32_bytes(&mut blob, 1);
    u32_bytes(&mut blob, 114);
    u32_bytes(&mut blob, 1);
    blob.extend_from_slice(&(2017u64 << 48).to_le_bytes());
    blob.push(1);
    blob.extend_from_slice(&[0; 4]);
    blob.push(0x80);
    u16_bytes(&mut blob, 0);
    u32_bytes(&mut blob, 0);
    u32_bytes(&mut blob, 2);
    node(&mut blob, 0, 1, &[1]);
    node(&mut blob, 2, 3, &[]);
    u32_bytes(&mut blob, 4);
    for text in ["MonoBehaviour", "Base", "int", "m_Header"] {
        string(&mut blob, text);
    }
    let mut file = Vec::new();
    u32_bytes(&mut file, 0x2A4B_5054);
    file.extend_from_slice(&[2, 0, 0, 0, 0, 0, 0, 0]);
    u32_bytes(&mut file, blob.len() as u32);
    u32_bytes(&mut file, blob.len() as u32);
    file.extend_from_slice(&blob);
    VersionTable::from_tpk(&file).unwrap()
}

fn layouts(table: &str) -> PlayerLayouts {
    let scripts = ScriptTable::from_jsonl_gz(&gzipped(table)).unwrap();
    PlayerLayouts::new(classes(), Some(scripts))
}

fn names(nodes: &[TypeTreeNode]) -> Vec<&str> {
    nodes.iter().map(|node| node.name.as_str()).collect()
}

#[test]
fn the_fields_of_a_script_follow_the_engine_header() {
    let layouts = layouts(&script_line("Game.Dialog", "buttons"));
    let tree = layouts
        .script_tree("Assembly-CSharp/Game.Dialog", VERSION)
        .unwrap();
    assert_eq!(names(tree.nodes()), ["Base", "m_Header", "buttons"]);
    let field = &tree.nodes()[HEADER_NODES];
    assert_eq!(field.level, 1);
    assert_eq!(field.index, HEADER_NODES as i32);
    assert_eq!(field.byte_size, 4);
    assert!(field.aligned());
}

#[test]
fn a_script_without_fields_or_not_in_the_table_has_no_tree() {
    let failed = r#"{"assembly":"Assembly-CSharp","class":"Game.Failed","error":"no layout"}"#;
    let layouts = layouts(failed);
    assert!(
        layouts
            .script_tree("Assembly-CSharp/Game.Failed", VERSION)
            .is_none()
    );
    assert!(
        layouts
            .script_tree("Assembly-CSharp/Game.Absent", VERSION)
            .is_none()
    );
}

#[test]
fn the_identity_follows_both_tables() {
    let one = layouts(&script_line("Game.Dialog", "one"));
    let two = layouts(&script_line("Game.Dialog", "two"));
    let without = PlayerLayouts::new(classes(), None);
    assert_ne!(one.identity(), two.identity());
    assert_ne!(one.identity(), without.identity());
    assert_eq!(
        one.identity(),
        layouts(&script_line("Game.Dialog", "one")).identity()
    );
}

#[test]
fn a_table_that_is_not_gzip_is_an_error() {
    assert!(ScriptTable::from_jsonl_gz(b"not gzip").is_err());
}
