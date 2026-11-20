//! The player data of client packages.

#![cfg(feature = "client")]

use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;

use sekai_game::client::{PLAYER_BUNDLE, STREAMING_ASSETS_BUNDLE, bundles, bundles_from};
use sekai_unity::{Bundle, ByteSource, NoBlockCache};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const DATA: &str = "assets/bin/Data/";

/// An entry of a container: its path, whether it is a serialized file, and its bytes.
type Entry = (String, bool, Vec<u8>);

/// The bytes of the fixture bundle `name`, whose `AssetBundle` object names it `name`.
fn bundle_fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sekai-unity/tests/fixtures/lz4")
        .join(name);
    std::fs::read(path).unwrap()
}

fn fixture() -> Vec<u8> {
    bundle_fixture("fixture_data")
}

fn entries(container: Arc<Vec<u8>>) -> Vec<Entry> {
    let source: Arc<dyn ByteSource> = container;
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
    let (_, _, bytes) = entries(Arc::new(fixture()))
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

/// The player data of `package`, the first of its bundles.
fn player(package: &[u8]) -> std::io::Result<Arc<Vec<u8>>> {
    let mut bundles = bundles(package)?;
    assert_eq!(bundles[0].name, PLAYER_BUNDLE);
    Ok(bundles.swap_remove(0).bytes)
}

#[test]
fn a_container_gives_the_entries_of_the_player_bundle() {
    let apk = archive(&[(&data("data.unity3d"), &fixture())]);
    assert_eq!(entries(player(&apk).unwrap()), entries(Arc::new(fixture())));
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
    assert_eq!(entries(player(&package).unwrap()), expected);
}

#[test]
fn a_file_without_all_its_parts_is_an_error() {
    let apk = archive(&[(&data("level0.split1"), &serialized_file())]);
    let error = player(&apk).unwrap_err();
    assert!(error.to_string().contains("level0"), "{error}");
}

#[test]
fn a_file_in_two_packages_is_an_error() {
    let file = serialized_file();
    let apk = archive(&[(&data("level0"), &file)]);
    let package = archive(&[("base.apk", &apk), ("pack.apk", &apk)]);
    assert!(player(&package).is_err());
}

#[test]
fn a_package_without_player_data_is_an_error() {
    let apk = archive(&[("res/icon.png", b"image")]);
    assert!(player(&apk).is_err());
}

#[test]
fn nested_apks_in_subdirectories_and_uppercase_extensions_are_unpacked() {
    let file = serialized_file();
    let base = archive(&[(&data("level0"), &file)]);
    let pack = archive(&[(&data("level0.resS"), b"streamed")]);
    let xapk = archive(&[("apks/Base.APK", &base), ("splits/asset_pack.apk", &pack)]);

    let expected = [
        ("level0", true, file),
        ("level0.resS", false, b"streamed".to_vec()),
    ]
    .map(|(path, serialized, bytes)| (path.to_owned(), serialized, bytes));
    assert_eq!(
        entries(
            bundles_from(std::io::Cursor::new(&xapk))
                .unwrap()
                .remove(0)
                .bytes
        ),
        expected
    );
}

#[test]
fn deeply_nested_apks_are_rejected() {
    let inner = archive(&[(&data("level0"), &serialized_file())]);
    let mid = archive(&[("inner.apk", &inner)]);
    let outer = archive(&[("mid.apk", &mid)]);
    let err = player(&outer).unwrap_err();
    assert!(err.to_string().contains("depth"), "{err}");
}

#[test]
fn the_cue_sheets_of_the_streaming_assets_are_a_second_bundle() {
    let base = archive(&[
        (&data("level0"), &serialized_file()),
        ("assets/dic", b"dictionary"),
        ("assets/ad/index.js", b"script"),
    ]);
    let pack = archive(&[
        ("assets/Menu.acb", b"cue sheet"),
        ("assets/Menu.awb", b"wave bank"),
        ("assets/Project.acf", b"project"),
        ("assets/sub/Movie.usm", b"movie"),
        ("assets/sub/Title.ACB", b"title"),
    ]);
    let package = archive(&[("base.apk", &base), ("pack.apk", &pack)]);

    let mut bundles = bundles(&package).unwrap();
    let names: Vec<_> = bundles.iter().map(|bundle| bundle.name.as_str()).collect();
    assert_eq!(names, [PLAYER_BUNDLE, STREAMING_ASSETS_BUNDLE]);
    let expected = [
        ("Menu.acb", b"cue sheet".as_slice()),
        ("sub/Title.ACB", b"title"),
    ]
    .map(|(path, bytes)| (path.to_owned(), false, bytes.to_vec()));
    assert_eq!(entries(bundles.pop().unwrap().bytes), expected);
}

#[test]
fn a_package_without_cue_sheets_has_only_the_player_data() {
    let apk = archive(&[(&data("level0"), &serialized_file())]);
    let bundles = bundles(&apk).unwrap();
    let names: Vec<_> = bundles.iter().map(|bundle| bundle.name.as_str()).collect();
    assert_eq!(names, [PLAYER_BUNDLE]);
}

/// `bundle` with the header of a downloaded bundle: a marker, then the first 128 bytes with five
/// of every eight inverted.
fn as_downloaded(bundle: &[u8]) -> Vec<u8> {
    let inverted = bundle.iter().enumerate().map(|(i, &b)| {
        let inverts = i < 128 && i % 8 < 5;
        if inverts { !b } else { b }
    });
    [0x10, 0, 0, 0].into_iter().chain(inverted).collect()
}

#[test]
fn the_unity_bundles_of_the_streaming_assets_are_bundles_under_their_own_names() {
    let scene = bundle_fixture("fixture_scene");
    let apk = archive(&[
        (&data("level0"), &serialized_file()),
        ("assets/Menu.acb", b"cue sheet"),
        ("assets/data/9f/c3a1", &as_downloaded(&scene)),
        ("assets/Font/font.ab", &fixture()),
        ("assets/Font/readme.txt", b"text"),
    ]);

    let bundles = bundles(&apk).unwrap();
    let names: Vec<_> = bundles.iter().map(|bundle| bundle.name.as_str()).collect();
    let expected = [
        PLAYER_BUNDLE,
        STREAMING_ASSETS_BUNDLE,
        "fixture_data",
        "fixture_scene",
    ];
    assert_eq!(names, expected);
    assert_eq!(
        entries(bundles[2].bytes.clone()),
        entries(Arc::new(fixture()))
    );
    assert_eq!(entries(bundles[3].bytes.clone()), entries(Arc::new(scene)));
}

#[test]
fn two_bundles_of_one_name_are_an_error() {
    let apk = archive(&[
        (&data("level0"), &serialized_file()),
        ("assets/Font/font.ab", &fixture()),
        ("assets/Font/copy.ab", &as_downloaded(&fixture())),
    ]);
    let error = bundles(&apk).unwrap_err();
    assert!(error.to_string().contains("fixture_data"), "{error}");
}
