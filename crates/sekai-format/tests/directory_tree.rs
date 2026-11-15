//! Directory trees: each directory once, with its files and subdirectories in name order, and
//! back to the files they were built from.

use sekai_format::directory_tree::{DirectoryTree, TreeFile};
use serde_json::json;

fn file(path: &str, size: u64, media_type: &str) -> TreeFile {
    TreeFile {
        path: path.to_owned(),
        size,
        media_type: media_type.to_owned(),
    }
}

/// Files in path order, where `a-b/` comes before `a.txt` and `a/`, which come before `b.png`.
fn files() -> Vec<TreeFile> {
    vec![
        file("a-b/x.png", 4, "image/png"),
        file("a.txt", 1, "text/plain"),
        file("a/c/d.json", 2, "application/json"),
        file("a/z.png", 3, "image/png"),
        file("b.png", 5, "image/png"),
    ]
}

fn tree(files: Vec<TreeFile>) -> DirectoryTree {
    DirectoryTree::new("storage", "v1", "r1", files).unwrap()
}

#[test]
fn a_tree_lists_each_directory_once_in_name_order() {
    let document = serde_json::to_value(tree(files())).unwrap();
    let expected = json!({
        "format": "directory-tree",
        "version": 1,
        "view": "storage",
        "published_version": "v1",
        "revision": "r1",
        "files": 5,
        "media_types": ["image/png", "text/plain", "application/json"],
        "root": {
            "name": "",
            "directories": [
                {
                    "name": "a",
                    "directories": [
                        { "name": "c", "files": { "names": ["d.json"], "sizes": [2], "media_types": [2] } }
                    ],
                    "files": { "names": ["z.png"], "sizes": [3], "media_types": [0] }
                },
                { "name": "a-b", "files": { "names": ["x.png"], "sizes": [4], "media_types": [0] } }
            ],
            "files": { "names": ["a.txt", "b.png"], "sizes": [1, 5], "media_types": [1, 0] }
        }
    });
    assert_eq!(document, expected);
}

#[test]
fn a_tree_gives_back_the_files_it_was_built_from() {
    let built = tree(files());
    let read = DirectoryTree::from_json(&serde_json::to_vec(&built).unwrap()).unwrap();
    assert_eq!(read, built);
    assert_eq!(read.files().unwrap(), files());
    let empty = tree(Vec::new());
    assert_eq!(empty.files().unwrap(), Vec::new());
}

#[test]
fn files_out_of_order_or_without_names_make_no_tree() {
    let mut reversed = files();
    reversed.reverse();
    let twice = vec![
        file("a.txt", 1, "text/plain"),
        file("a.txt", 1, "text/plain"),
    ];
    for files in [
        reversed,
        twice,
        vec![file("a//b.txt", 1, "text/plain")],
        vec![file("a/", 1, "text/plain")],
        vec![file("", 1, "text/plain")],
    ] {
        assert!(DirectoryTree::new("storage", "v1", "r1", files).is_err());
    }
}

#[test]
fn a_tree_that_does_not_hold_what_it_says_gives_no_files() {
    let broken: [fn(&mut DirectoryTree); 7] = [
        |tree| tree.files += 1,
        |tree| tree.root.files.sizes.truncate(1),
        |tree| tree.root.files.media_types[0] = 9,
        |tree| tree.root.files.names[0] = "a/b".to_owned(),
        |tree| tree.root.directories[0].name = String::new(),
        |tree| tree.root.name = "root".to_owned(),
        |tree| {
            let copy = tree.root.directories[0].clone();
            tree.root.directories.push(copy);
        },
    ];
    for (n, break_it) in broken.into_iter().enumerate() {
        let mut tree = tree(files());
        break_it(&mut tree);
        assert!(tree.files().is_err(), "case {n}");
    }
}

#[test]
fn another_format_is_refused() {
    let mut document = serde_json::to_value(tree(files())).unwrap();
    document["version"] = json!(2);
    assert!(DirectoryTree::from_json(&serde_json::to_vec(&document).unwrap()).is_err());
}
