//! Reads every object of serialized files with the type trees of a TPK file and counts the
//! results by class.
//!
//! Usage: `class_coverage <table.tpk> <serialized file>...`
//!
//! Each line is `<class>\t<objects>\t<read>\t<error code>=<objects> ...`, by class name, and the
//! last line is `total\t<objects>\t<read>`.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;

use sekai_layouts::VersionTable;
use sekai_unity::class_id::class_name;
use sekai_unity::index::{FileLayouts, OpenFile};
use sekai_unity::read::IgnoreValues;
use sekai_unity::{FileSource, SerializedFile};

const READ: &str = "read";

/// How many objects of each class gave each result.
type Results = BTreeMap<String, BTreeMap<&'static str, usize>>;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let table = arguments
        .next()
        .ok_or("usage: class_coverage <table.tpk> <serialized file>...")?;
    let classes = VersionTable::from_tpk(&std::fs::read(table)?)?;
    let mut results = Results::new();
    for path in arguments {
        count_objects(Path::new(&path), &classes, &mut results)?;
    }
    let (mut objects, mut read) = (0, 0);
    for (class, codes) in &results {
        let class_objects: usize = codes.values().sum();
        let class_read = codes.get(READ).copied().unwrap_or(0);
        let errors: Vec<String> = codes
            .iter()
            .filter(|(code, _)| **code != READ)
            .map(|(code, count)| format!("{code}={count}"))
            .collect();
        println!(
            "{class}\t{class_objects}\t{class_read}\t{}",
            errors.join(" ")
        );
        objects += class_objects;
        read += class_read;
    }
    println!("total\t{objects}\t{read}");
    Ok(())
}

/// Reads every object of the file at `path` and adds its result to `results`.
fn count_objects(
    path: &Path,
    classes: &VersionTable,
    results: &mut Results,
) -> Result<(), Box<dyn Error>> {
    let source = FileSource::open(path)?;
    let file = SerializedFile::parse(&source)?;
    let layouts = FileLayouts::with_classes(&file, classes);
    let name = path.to_string_lossy();
    let open = OpenFile {
        path: &name,
        file: &file,
        source: &source,
        layouts: &layouts,
    };
    for (index, object) in file.objects.iter().enumerate() {
        let code = match open.read(index, &mut IgnoreValues) {
            Ok(()) => READ,
            Err(error) => error.code(),
        };
        let class =
            class_name(object.class_id).map_or_else(|| object.class_id.to_string(), str::to_owned);
        *results.entry(class).or_default().entry(code).or_default() += 1;
    }
    Ok(())
}
