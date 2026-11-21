//! `sekai-exporter script-table`: a table of the script layouts of a client package, from the
//! type trees that distribution bundles store and from other tables.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sekai_layouts::{ScriptFields, ScriptTable, write_script_table};
use sekai_pipeline::parallel;
use sekai_pipeline::stream::Inputs;
use sekai_unity::index::{FileLayouts, OpenFile};
use sekai_unity::{Bundle, ClassLayouts, Layout, NoBlockCache, SerializedFile, TypeTree, Visitor};

use crate::client;
use crate::export::{self, Source};

const MONO_BEHAVIOUR: i32 = 114;

struct Options {
    package: PathBuf,
    output: PathBuf,
    classes: PathBuf,
    tables: Vec<PathBuf>,
    bundles: Vec<PathBuf>,
    threads: usize,
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let mut positional = Vec::new();
    let mut classes = None;
    let mut tables = Vec::new();
    let mut bundles = Vec::new();
    let mut threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    while let Some(arg) = parser.next()? {
        match arg {
            Long("class-table") => classes = Some(PathBuf::from(parser.value()?)),
            Long("script-table") => tables.push(PathBuf::from(parser.value()?)),
            Long("bundles") => bundles.push(PathBuf::from(parser.value()?)),
            Long("threads") => threads = parser.value()?.parse()?,
            Value(v) => positional.push(PathBuf::from(v)),
            _ => return Err(arg.unexpected()),
        }
    }
    let [package, output]: [PathBuf; 2] = positional
        .try_into()
        .map_err(|_| lexopt::Error::from("expected <package> and <output>"))?;
    Ok(Options {
        package,
        output,
        classes: classes.ok_or("--class-table is required")?,
        tables,
        bundles,
        threads: threads.max(1),
    })
}

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    let options = parse(args)?;
    let package = client::source(&options.package, &options.classes, None)?;
    let player = PlayerData::open(&package)?;
    let candidates = gather(&options, &player, package.classes.as_ref())?;
    let scripts: Vec<(&String, &Vec<Use>)> = player.uses.iter().collect();
    let verdicts = parallel(scripts.len(), options.threads, |i| {
        let (script, uses) = scripts[i];
        candidates.judge(script, |fields| player.reads_exactly(uses, fields))
    });
    report(&scripts, &verdicts);
    let kept = scripts
        .iter()
        .zip(&verdicts)
        .filter_map(|((script, _), verdict)| match verdict {
            Verdict::Kept(candidate) => Some((script.as_str(), &candidate.fields)),
            _ => None,
        });
    let table = write_script_table(kept).map_err(|e| e.to_string())?;
    std::fs::write(&options.output, table)
        .map_err(|e| format!("{}: {e}", options.output.display()))?;
    Ok(true)
}

/// The layouts of the scripts of `player` in the tables and bundle directories of `options`.
fn gather<'p>(
    options: &Options,
    player: &'p PlayerData,
    classes: &dyn ClassLayouts,
) -> Result<Candidates<'p>, lexopt::Error> {
    let mut candidates = Candidates::for_player(player);
    for path in &options.tables {
        candidates.add_table(&client::read_table(path, ScriptTable::from_jsonl_gz)?, path);
    }
    for root in &options.bundles {
        candidates.add_bundles(&export::directory(root)?, classes, options.threads);
    }
    Ok(candidates)
}

/// Prints each script with the number of its objects and its verdict, then the counts.
fn report(scripts: &[(&String, &Vec<Use>)], verdicts: &[Verdict<'_>]) {
    for ((script, uses), verdict) in scripts.iter().zip(verdicts) {
        let objects: usize = uses.iter().map(|u| u.objects.len()).sum();
        println!("script {script} objects {objects}: {verdict}");
    }
    let kept = verdicts
        .iter()
        .filter(|v| matches!(v, Verdict::Kept(_)))
        .count();
    let left_out = verdicts.len() - kept;
    println!("scripts {} kept {kept} left out {left_out}", verdicts.len());
}

/// A bundle, open, with its serialized files and their layouts.
struct OpenBundle {
    name: String,
    bundle: Bundle,
    files: Vec<(usize, SerializedFile, FileLayouts)>,
}

/// A `MonoBehaviour` type of a file of a bundle whose script is known.
struct Behaviour<'b> {
    file: usize,
    type_index: usize,
    script: &'b str,
    /// The type tree the file stores for the type.
    tree: Option<&'b TypeTree>,
}

impl OpenBundle {
    /// Opens the bundle `name` of `inputs`, whose files take layouts and script names from
    /// `classes`.
    fn open(inputs: &dyn Inputs, name: &str, classes: &dyn ClassLayouts) -> Result<Self, String> {
        let input = inputs.open(name).map_err(|e| format!("{name}: {e}"))?;
        let opened = Bundle::open(input.source, input.sha256).and_then(|bundle| {
            let files = sekai_unity::open_files(&bundle, &NoBlockCache, classes)?;
            Ok(Self {
                name: name.to_owned(),
                bundle,
                files,
            })
        });
        opened.map_err(|e| format!("{name}: {e}"))
    }

    fn behaviours(&self) -> impl Iterator<Item = Behaviour<'_>> {
        let files = self.files.iter().enumerate();
        files.flat_map(|(file, (_, serialized, layouts))| {
            let types = serialized.types.iter().enumerate();
            types
                .filter(|(_, t)| t.class_id == MONO_BEHAVIOUR)
                .filter_map(move |(type_index, t)| {
                    Some(Behaviour {
                        file,
                        type_index,
                        script: layouts.script(type_index)?,
                        tree: t.tree.as_deref(),
                    })
                })
        })
    }

    /// Reads `objects` of the file at position `file` with `layout`, each to its last byte.
    fn reads_exactly(&self, file: usize, objects: &[usize], layout: &Layout) -> Result<(), String> {
        let (entry, serialized, layouts) = &self.files[file];
        let open = OpenFile {
            path: &self.bundle.entries()[*entry].path,
            file: serialized,
            source: &self.bundle.entry(*entry, &NoBlockCache),
            layouts,
        };
        for &object in objects {
            open.read_as(object, layout, &mut Skip).map_err(|e| {
                let path_id = serialized.objects[object].path_id;
                format!("{} {} object {path_id}: {e}", self.name, open.path)
            })?;
        }
        Ok(())
    }
}

/// A visitor that takes nothing, for reading an object only to see that it reads.
struct Skip;

impl Visitor for Skip {}

/// The bundles of a client package, open, with the objects of each script that they store
/// without a type tree.
struct PlayerData {
    bundles: Vec<OpenBundle>,
    uses: BTreeMap<String, Vec<Use>>,
    /// The engine header of a `MonoBehaviour` at each Unity version of the files with uses.
    headers: Headers,
}

/// Engine headers of a `MonoBehaviour`, by Unity version.
type Headers = BTreeMap<String, Arc<TypeTree>>;

/// The objects of one `MonoBehaviour` type of a file of the player data.
struct Use {
    bundle: usize,
    file: usize,
    objects: Vec<usize>,
}

impl PlayerData {
    fn open(package: &Source<'_>) -> Result<Self, String> {
        let open = |entry: &sekai_pipeline::stream::Entry| {
            OpenBundle::open(
                package.inputs.as_ref(),
                &entry.name,
                package.classes.as_ref(),
            )
        };
        let bundles = package
            .manifest
            .iter()
            .map(open)
            .collect::<Result<Vec<_>, _>>()?;
        let mut uses: BTreeMap<String, Vec<Use>> = BTreeMap::new();
        for (index, bundle) in bundles.iter().enumerate() {
            for behaviour in bundle.behaviours().filter(|b| b.tree.is_none()) {
                let objects = bundle.files[behaviour.file].1.objects.iter().enumerate();
                let objects = objects.filter(|(_, o)| o.type_index == behaviour.type_index);
                uses.entry(behaviour.script.to_owned())
                    .or_default()
                    .push(Use {
                        bundle: index,
                        file: behaviour.file,
                        objects: objects.map(|(object, _)| object).collect(),
                    });
            }
        }
        let headers = headers(&bundles, &uses, package.classes.as_ref())?;
        Ok(Self {
            bundles,
            uses,
            headers,
        })
    }

    /// Reads every object of `uses` with the engine header of its file and then `fields`, each
    /// to its last byte, and gives the first error.
    fn reads_exactly(&self, uses: &[Use], fields: &ScriptFields) -> Result<(), String> {
        let layouts = self.headers.iter().map(|(version, header)| {
            let tree = fields
                .tree(header)
                .ok_or("the fields do not make a type tree")?;
            let layout = Layout::compile(&tree).map_err(|e| e.to_string())?;
            Ok((version.as_str(), layout))
        });
        let layouts = layouts.collect::<Result<BTreeMap<_, _>, String>>()?;
        uses.iter().try_for_each(|u| {
            let bundle = &self.bundles[u.bundle];
            let layout = &layouts[bundle.files[u.file].1.unity_version.as_str()];
            bundle.reads_exactly(u.file, &u.objects, layout)
        })
    }
}

/// The engine header of a `MonoBehaviour` in `classes` at each Unity version of the files of
/// `bundles` that `uses` name.
fn headers(
    bundles: &[OpenBundle],
    uses: &BTreeMap<String, Vec<Use>>,
    classes: &dyn ClassLayouts,
) -> Result<Headers, String> {
    let files = uses
        .values()
        .flatten()
        .map(|u| &bundles[u.bundle].files[u.file].1);
    let versions: BTreeSet<&str> = files.map(|file| file.unity_version.as_str()).collect();
    let header = |version: &str| {
        let header = classes.tree(MONO_BEHAVIOUR, version);
        let header = header.map(|header| (version.to_owned(), header));
        header.ok_or_else(|| format!("the class table has no MonoBehaviour at Unity {version}"))
    };
    versions.into_iter().map(header).collect()
}

/// The layouts that each script of the player data may have.
struct Candidates<'p> {
    scripts: BTreeMap<String, Vec<Candidate>>,
    /// The engine headers that the stored type trees of the scripts may start with.
    headers: &'p Headers,
}

/// A layout of a script, and where it was found.
struct Candidate {
    fields: ScriptFields,
    /// The first table or bundle it was found in.
    source: String,
    /// How many tables and bundle files it was found in.
    found: usize,
}

impl<'p> Candidates<'p> {
    /// No layouts yet for the scripts of `player`.
    fn for_player(player: &'p PlayerData) -> Self {
        let scripts = player
            .uses
            .keys()
            .map(|script| (script.clone(), Vec::new()));
        Self {
            scripts: scripts.collect(),
            headers: &player.headers,
        }
    }

    /// Adds `fields` as a layout of `script` found in `source`, when the player data uses the
    /// script.
    fn add(&mut self, script: &str, fields: &ScriptFields, source: &str) {
        let Some(candidates) = self.scripts.get_mut(script) else {
            return;
        };
        match candidates.iter_mut().find(|c| &c.fields == fields) {
            Some(candidate) => candidate.found += 1,
            None => candidates.push(Candidate {
                fields: fields.clone(),
                source: source.to_owned(),
                found: 1,
            }),
        }
    }

    /// Adds the rows of `table`, the file at `path`.
    fn add_table(&mut self, table: &ScriptTable, path: &Path) {
        let source = path.display().to_string();
        for (script, fields) in table.iter() {
            self.add(script, fields, &source);
        }
    }

    /// Adds the type trees that the bundles of `directory` store for the scripts. A file that
    /// is not a bundle, or cannot be read, is reported and passed over.
    fn add_bundles(&mut self, directory: &Source<'_>, classes: &dyn ClassLayouts, threads: usize) {
        let names = &directory.manifest;
        let stored = parallel(names.len(), threads, |i| {
            let bundle = OpenBundle::open(directory.inputs.as_ref(), &names[i].name, classes)?;
            Ok::<_, String>(self.stored_fields(&bundle))
        });
        let mut failed = 0;
        for (entry, stored) in names.iter().zip(stored) {
            match stored {
                Ok(fields) => fields
                    .iter()
                    .for_each(|(script, fields)| self.add(script, fields, &entry.name)),
                Err(e) => {
                    failed += 1;
                    eprintln!("passed over {e}");
                }
            }
        }
        println!("bundle files {} passed over {failed}", names.len());
    }

    /// The fields of the scripts in question whose type trees `bundle` stores. Distribution
    /// bundles may not name their Unity version, so a tree is taken when it starts with an
    /// engine header of the player data.
    fn stored_fields(&self, bundle: &OpenBundle) -> Vec<(String, ScriptFields)> {
        let wanted = bundle
            .behaviours()
            .filter(|b| self.scripts.contains_key(b.script));
        wanted
            .filter_map(|behaviour| {
                let tree = behaviour.tree?;
                let of_tree = |header: &Arc<TypeTree>| ScriptFields::of_tree(tree, header);
                let fields = self.headers.values().find_map(of_tree)?;
                Some((behaviour.script.to_owned(), fields))
            })
            .collect()
    }

    /// The verdict on the layouts of `script`, by whether each `reads` every object of it.
    fn judge(
        &self,
        script: &str,
        reads: impl Fn(&ScriptFields) -> Result<(), String>,
    ) -> Verdict<'_> {
        let candidates = &self.scripts[script];
        let read: Vec<(&Candidate, Option<String>)> = candidates
            .iter()
            .map(|candidate| (candidate, reads(&candidate.fields).err()))
            .collect();
        let exact: Vec<&Candidate> = read
            .iter()
            .filter(|(_, error)| error.is_none())
            .map(|(candidate, _)| *candidate)
            .collect();
        match exact.as_slice() {
            [candidate] => Verdict::Kept(candidate),
            [] if candidates.is_empty() => Verdict::NoCandidate,
            [] => Verdict::NoneReads(
                read.into_iter()
                    .filter_map(|(candidate, error)| Some((candidate, error?)))
                    .collect(),
            ),
            several => Verdict::Ambiguous(several.len()),
        }
    }
}

/// What became of a script of the player data.
enum Verdict<'c> {
    /// The one layout that reads every object of the script.
    Kept(&'c Candidate),
    /// No table or bundle has a layout of the script.
    NoCandidate,
    /// No layout reads every object of the script; each with the first error it gave.
    NoneReads(Vec<(&'c Candidate, String)>),
    /// This many different layouts read every object of the script.
    Ambiguous(usize),
}

impl std::fmt::Display for Verdict<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Verdict::Kept(candidate) => write!(f, "kept, {}", Found(candidate)),
            Verdict::NoCandidate => write!(f, "left out, no table or bundle has a layout"),
            Verdict::NoneReads(failed) => {
                write!(f, "left out, no layout reads every object")?;
                failed
                    .iter()
                    .try_for_each(|(candidate, e)| write!(f, "; {}: {e}", Found(candidate)))
            }
            Verdict::Ambiguous(n) => write!(f, "left out, {n} layouts read every object"),
        }
    }
}

/// Where a layout was found, for the report.
struct Found<'c>(&'c Candidate);

impl std::fmt::Display for Found<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Candidate { source, found, .. } = self.0;
        match found - 1 {
            0 => write!(f, "found in {source}"),
            more => write!(f, "found in {source} and {more} more"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    const DIALOG: &str = "Assembly-CSharp/Game.Dialog";

    /// The fields of a script with one `int` field called `name`.
    fn fields(name: &str) -> ScriptFields {
        let line = format!(
            r#"{{"assembly":"Assembly-CSharp","class":"Game.Dialog","nodes":[{{"type":"int","name":"{name}","level":1,"byteSize":4,"typeFlags":0,"version":1,"metaFlag":0}}]}}"#
        );
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(line.as_bytes()).unwrap();
        let table = ScriptTable::from_jsonl_gz(&encoder.finish().unwrap()).unwrap();
        table.get(DIALOG).unwrap().clone()
    }

    fn candidates<'h>(headers: &'h Headers, layouts: &[(&str, &str)]) -> Candidates<'h> {
        let mut candidates = Candidates {
            scripts: BTreeMap::from([(DIALOG.to_owned(), Vec::new())]),
            headers,
        };
        for (name, source) in layouts {
            candidates.add(DIALOG, &fields(name), source);
        }
        candidates
    }

    /// The verdict on `DIALOG` when the layouts whose field is called `exact` read every object.
    fn verdict(layouts: &[(&str, &str)], exact: &[&str]) -> String {
        let headers = Headers::new();
        let candidates = candidates(&headers, layouts);
        let reads = |candidate: &ScriptFields| {
            let read = exact.iter().any(|name| &fields(name) == candidate);
            read.then_some(())
                .ok_or_else(|| "reads past the end".to_owned())
        };
        candidates.judge(DIALOG, reads).to_string()
    }

    #[test]
    fn a_layout_found_twice_is_one_candidate_and_other_scripts_are_passed_over() {
        let headers = Headers::new();
        let mut candidates = candidates(&headers, &[("one", "a"), ("one", "b"), ("two", "c")]);
        candidates.add("Assembly-CSharp/Game.Other", &fields("one"), "d");
        let found: Vec<(&str, usize)> = candidates.scripts[DIALOG]
            .iter()
            .map(|c| (c.source.as_str(), c.found))
            .collect();
        assert_eq!(found, [("a", 2), ("c", 1)]);
        assert_eq!(candidates.scripts.len(), 1);
    }

    #[test]
    fn a_script_is_kept_with_the_one_layout_that_reads_every_object() {
        let layouts = [("one", "a"), ("one", "b"), ("two", "c")];
        assert_eq!(verdict(&layouts, &["one"]), "kept, found in a and 1 more");
        assert_eq!(verdict(&layouts, &["two"]), "kept, found in c");
    }

    #[test]
    fn a_script_without_exactly_one_layout_that_reads_is_left_out_with_the_reason() {
        let layouts = [("one", "a"), ("two", "c")];
        assert_eq!(
            verdict(&[], &[]),
            "left out, no table or bundle has a layout"
        );
        assert_eq!(
            verdict(&layouts, &[]),
            "left out, no layout reads every object; found in a: reads past the end; \
             found in c: reads past the end"
        );
        assert_eq!(
            verdict(&layouts, &["one", "two"]),
            "left out, 2 layouts read every object"
        );
    }
}
