//! The player data of client packages.

#![cfg(feature = "client")]

use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;

use sekai_game::client::player_bundle;
use sekai_unity::{Bundle, ByteSource, NoBlockCache};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const DATA: &str = "assets/bin/Data/";

/// An entry of a container: its path, whether it is a serialized file, and its bytes.
type Entry = (String, bool, Vec<u8>);

fn fixture() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sekai-unity/tests/fixtures/lz4/fixture_data");
    std::fs::read(path).unwrap()
}

fn entries(container: Vec<u8>) -> Vec<Entry> {
    let source: Arc<dyn ByteSource> = Arc::new(container);
    let bundle = Bundle::open(source, [0; 32]).unwrap();
    bundle
        .entries()
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let bytes = bundle
                .entry(i, &NoBlockCache)
                .read(0, entry.size as usize)
                .unwrap()
                .to_vec();
            (entry.path.clone(), entry.is_serialized(), bytes)
        })
        .collect()
}

/// The bytes of a serialized file.
fn serialized_file() -> Vec<u8> {
    let (_, _, bytes) = entries(fixture())
        .into_iter()
        .find(|(_, serialized, _)| *serialized)
        .unwrap();
    bytes
}

fn archive(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in members {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn data(name: &str) -> String {
    format!("{DATA}{name}")
}

#[test]
fn a_container_gives_the_entries_of_the_player_bundle() {
    let apk = archive(&[(&data("data.unity3d"), &fixture())]);
    assert_eq!(entries(player_bundle(&apk).unwrap()), entries(fixture()));
}

#[test]
fn loose_files_are_joined_across_the_apks_of_a_package() {
    let file = serialized_file();
    let (head, tail) = file.split_at(file.len() / 2);
    let base = archive(&[
        (&data("level0.split0"), head),
        (&data("level0.split1"), tail),
        (&data("boot.config"), b"key=value"),
        ("res/icon.png", b"image"),
    ]);
    let pack = archive(&[
        (&data("Resources/unity_builtin_extra"), &file),
        (&data("level0.resS"), b"large data"),
    ]);
    let package = archive(&[("base.apk", &base), ("pack.apk", &pack)]);

    let expected = [
        ("unity_builtin_extra", true, file.clone()),
        ("level0", true, file.clone()),
        ("level0.resS", false, b"large data".to_vec()),
    ]
    .map(|(path, serialized, bytes)| (path.to_owned(), serialized, bytes));
    assert_eq!(entries(player_bundle(&package).unwrap()), expected);
}

#[test]
fn a_file_without_all_its_parts_is_an_error() {
    let apk = archive(&[(&data("level0.split1"), &serialized_file())]);
    let error = player_bundle(&apk).unwrap_err();
    assert!(error.to_string().contains("level0"), "{error}");
}

#[test]
fn a_file_in_two_packages_is_an_error() {
    let file = serialized_file();
    let apk = archive(&[(&data("level0"), &file)]);
    let package = archive(&[("base.apk", &apk), ("pack.apk", &apk)]);
    assert!(player_bundle(&package).is_err());
}

#[test]
fn a_package_without_player_data_is_an_error() {
    let apk = archive(&[("res/icon.png", b"image")]);
    assert!(player_bundle(&apk).is_err());
}
