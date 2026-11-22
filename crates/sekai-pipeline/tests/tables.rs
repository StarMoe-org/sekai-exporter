//! The table stage on assets and tables made here.

use std::collections::BTreeMap;
use std::io;

use sekai_catalog::{Key, MemoryStore, Store};
use sekai_convert::tables::{
    AssetEntry, Shared, SharedEntries, TableAsset, TableContext, TableConverter, TablePlan,
};
use sekai_convert::{Asset, Content, ConvertError, Member};
use sekai_format::{AssetDocument, MemberFile};
use sekai_pipeline::eval::{Evaluation, Output};
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::state::TableOutput;
use sekai_pipeline::tables::{
    Built, TableStage, VersionAssets, build, evaluate, evaluation_identity,
};

/// Builds one asset per line of the table `rows`. A line names a bundle, whose first asset and
/// its member `m` the asset copies; a line `fail` is an asset error.
#[derive(Default)]
struct Copies {
    revision: u32,
}

struct Plan {
    names: Vec<String>,
    /// The line of each asset.
    lines: Shared<String, String>,
}

impl TablePlan for Plan {
    fn names(&self) -> &[String] {
        &self.names
    }

    fn convert(&self, index: usize, cx: &dyn TableContext) -> Result<TableAsset, ConvertError> {
        let line = self
            .lines
            .get(&self.names[index], cx)
            .expect("every asset has a line");
        if line == "fail" {
            return Err(ConvertError::new("malformed", "a failing row"));
        }
        let source = cx.assets(line).first().expect("the bundle has an asset");
        assert_eq!(source.form, "text");
        Ok(TableAsset {
            asset: Asset {
                members: vec![Member {
                    role: "copy".to_owned(),
                    content: Content::File {
                        extension: "txt".to_owned(),
                        bytes: cx.member(&source.name, "m")?,
                    },
                }],
            },
            references: vec![source.name.clone(), source.name.clone()],
        })
    }

    fn reusable(&self) -> bool {
        true
    }

    fn shared(&self) -> Vec<&dyn SharedEntries> {
        vec![&self.lines]
    }
}

impl TableConverter for Copies {
    fn name(&self) -> &'static str {
        "copies"
    }

    fn version(&self) -> u32 {
        3
    }

    fn revision(&self) -> u32 {
        self.revision
    }

    fn plan(&self, cx: &dyn TableContext) -> Result<Box<dyn TablePlan>, ConvertError> {
        let text = String::from_utf8(cx.table("rows")?).expect("text");
        let lines: BTreeMap<String, String> = text
            .lines()
            .enumerate()
            .map(|(i, line)| (format!("z/copy{i}"), line.to_owned()))
            .collect();
        Ok(Box::new(Plan {
            names: lines.keys().cloned().collect(),
            lines: Shared::new("lines", lines),
        }))
    }
}

/// An evaluation with one asset `<bundle>/x` of each bundle of `bundles`, whose member `m` holds
/// the content given with the bundle.
fn version(store: &MemoryStore, bundles: &[(&str, &[u8])]) -> Evaluation {
    let outputs = bundles.iter().map(|(bundle, content)| {
        let member = store.put(content, "txt").unwrap();
        let name = format!("{bundle}/x");
        let document = AssetDocument {
            format: "asset".to_owned(),
            version: 1,
            name: name.clone(),
            form: "text".to_owned(),
            form_version: 1,
            fingerprint: "f".to_owned(),
            sources: Vec::new(),
            resources: Vec::new(),
            members: vec![MemberFile {
                role: "m".to_owned(),
                sha256: member.sha256.iter().map(|b| format!("{b:02x}")).collect(),
                size: content.len() as u64,
                extension: "txt".to_owned(),
                media_type: "text/plain".to_owned(),
            }],
            references: Vec::new(),
            error: None,
        };
        let bytes = document.to_bytes().unwrap();
        Output {
            name,
            bundle: (*bundle).to_owned(),
            form: "text",
            document: store.put(&bytes, "json").unwrap(),
            error: None,
        }
    });
    Evaluation {
        outputs: outputs.collect(),
        ..Evaluation::default()
    }
}

/// A stage of `copies` whose table `rows` is `rows`, after a run that built `previous`.
fn with_stage<T>(
    store: &MemoryStore,
    copies: Copies,
    rows: &'static str,
    previous: &[TableOutput],
    run: impl FnOnce(&TableStage<'_>) -> T,
) -> T {
    let converters: Vec<Box<dyn TableConverter>> = vec![Box::new(copies)];
    let read = move |name: &str| match name {
        "rows" => Ok(rows.as_bytes().to_vec()),
        other => Err(io::Error::other(format!("no table {other}"))),
    };
    run(&TableStage {
        converters: &converters,
        read: &read,
        store,
        recorder: &NoRecorder,
        threads: 2,
        previous,
    })
}

fn run(store: &MemoryStore, evaluation: &mut Evaluation, rows: &'static str) -> io::Result<()> {
    with_stage(store, Copies::default(), rows, &[], |stage| {
        evaluate(stage, evaluation)
    })
}

/// The assets that a stage with the table `rows` builds from `evaluation`, after a run that
/// built `previous`.
fn built(
    store: &MemoryStore,
    evaluation: &Evaluation,
    rows: &'static str,
    previous: &[Built],
) -> Vec<Built> {
    built_by(Copies::default(), store, evaluation, rows, previous)
}

/// The assets that a stage of `copies` with the table `rows` builds from `evaluation`, after a
/// run that built `previous`.
fn built_by(
    copies: Copies,
    store: &MemoryStore,
    evaluation: &Evaluation,
    rows: &'static str,
    previous: &[Built],
) -> Vec<Built> {
    let mut by_bundle: BTreeMap<String, Vec<AssetEntry>> = BTreeMap::new();
    for output in &evaluation.outputs {
        let entry = AssetEntry {
            name: output.name.clone(),
            form: output.form,
        };
        by_bundle
            .entry(output.bundle.clone())
            .or_default()
            .push(entry);
    }
    let document = |name: &str| {
        let output = evaluation.outputs.iter().find(|output| output.name == name);
        Ok(output.map(|output| output.document.clone()))
    };
    let version = VersionAssets {
        by_bundle,
        document: &document,
    };
    let previous: Vec<TableOutput> = previous.iter().map(TableOutput::of).collect();
    with_stage(store, copies, rows, &previous, |stage| {
        build(stage, &version)
    })
    .unwrap()
}

fn read(store: &MemoryStore, key: &Key) -> AssetDocument {
    AssetDocument::from_bytes(&store.read(key).unwrap()).unwrap()
}

/// The name of each asset of `built` that was kept from the previous run.
fn kept(built: &[Built]) -> Vec<&str> {
    let kept = built.iter().filter(|built| built.kept);
    kept.map(|built| built.output.name.as_str()).collect()
}

/// The name and document of each asset of `built`.
fn documents(built: &[Built]) -> Vec<(&str, &Key)> {
    let outputs = built.iter().map(|built| &built.output);
    outputs
        .map(|output| (output.name.as_str(), &output.document))
        .collect()
}

#[test]
fn assets_are_built_from_rows_and_the_assets_of_the_version() {
    let store = MemoryStore::default();
    let mut evaluation = version(&store, &[("b", b"hello")]);
    run(&store, &mut evaluation, "b\n").unwrap();
    let names: Vec<&str> = evaluation.outputs.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["b/x", "z/copy0"]);
    assert_eq!(evaluation.converted, 1);
    let document = read(&store, &evaluation.outputs[1].document);
    assert_eq!(document.form, "copies");
    assert_eq!(document.form_version, 3);
    assert!(document.sources.is_empty());
    assert_eq!(document.references, ["b/x"]);
    assert_eq!(document.members[0].role, "copy");
    assert_eq!(document.members[0].media_type, "text/plain");
}

#[test]
fn the_fingerprint_follows_the_output() {
    let fingerprint = |content: &[u8]| {
        let store = MemoryStore::default();
        let mut evaluation = version(&store, &[("b", content)]);
        run(&store, &mut evaluation, "b\n").unwrap();
        read(&store, &evaluation.outputs[1].document).fingerprint
    };
    assert_eq!(fingerprint(b"one"), fingerprint(b"one"));
    assert_ne!(fingerprint(b"one"), fingerprint(b"two"));
}

#[test]
fn a_row_that_cannot_be_built_is_an_asset_error() {
    let store = MemoryStore::default();
    let mut evaluation = version(&store, &[("b", b"hello")]);
    run(&store, &mut evaluation, "b\nfail\n").unwrap();
    assert_eq!(evaluation.outputs[2].error.as_deref(), Some("malformed"));
    let document = read(&store, &evaluation.outputs[2].document);
    assert!(document.members.is_empty());
    assert_eq!(document.error.unwrap().code, "malformed");
}

#[test]
fn a_table_that_cannot_be_read_fails_the_run() {
    struct NoTable;
    impl TableConverter for NoTable {
        fn name(&self) -> &'static str {
            "none"
        }
        fn version(&self) -> u32 {
            1
        }
        fn plan(&self, cx: &dyn TableContext) -> Result<Box<dyn TablePlan>, ConvertError> {
            cx.table("absent")?;
            unreachable!("the table cannot be read")
        }
    }
    let store = MemoryStore::default();
    let converters: Vec<Box<dyn TableConverter>> = vec![Box::new(NoTable)];
    let read = |_: &str| Err(io::Error::other("unreachable host"));
    let error = evaluate(
        &TableStage {
            converters: &converters,
            read: &read,
            store: &store,
            recorder: &NoRecorder,
            threads: 1,
            previous: &[],
        },
        &mut Evaluation::default(),
    )
    .expect_err("the run fails");
    assert!(error.to_string().contains("unreachable host"), "{error}");
}

#[test]
fn assets_whose_reads_give_what_they_gave_are_kept() {
    let store = MemoryStore::default();
    let evaluation = version(&store, &[("b", b"one"), ("c", b"two")]);
    let first = built(&store, &evaluation, "b\nc\nfail\n", &[]);
    assert!(kept(&first).is_empty());
    assert!(first.iter().all(|built| built.trace.is_some()));
    let second = built(&store, &evaluation, "b\nc\nfail\n", &first);
    assert_eq!(kept(&second), ["z/copy0", "z/copy1", "z/copy2"]);
    assert_eq!(documents(&second), documents(&first));
    assert_eq!(second[2].output.error.as_deref(), Some("malformed"));
}

#[test]
fn only_the_assets_whose_reads_changed_are_built_again() {
    let store = MemoryStore::default();
    let before = version(&store, &[("b", b"one"), ("c", b"two")]);
    let first = built(&store, &before, "b\nc\n", &[]);

    let after = version(&store, &[("b", b"one"), ("c", b"three")]);
    let second = built(&store, &after, "b\nc\n", &first);
    assert_eq!(kept(&second), ["z/copy0"]);
    let whole = built(&store, &after, "b\nc\n", &[]);
    assert_eq!(documents(&second), documents(&whole));

    let third = built(&store, &after, "b\nb\n", &second);
    assert_eq!(kept(&third), ["z/copy0"]);
    let whole = built(&store, &after, "b\nb\n", &[]);
    assert_eq!(documents(&third), documents(&whole));
}

#[test]
fn a_new_revision_builds_every_asset_again_in_the_same_form() {
    let store = MemoryStore::default();
    let evaluation = version(&store, &[("b", b"one"), ("c", b"two")]);
    let first = built(
        &store,
        &evaluation,
        "b
c
",
        &[],
    );
    let revised = || Copies { revision: 1 };
    let second = built_by(
        revised(),
        &store,
        &evaluation,
        "b
c
",
        &first,
    );
    assert!(kept(&second).is_empty());
    assert_eq!(documents(&second), documents(&first));
    assert_eq!(read(&store, &second[0].output.document).form_version, 3);
    let third = built_by(
        revised(),
        &store,
        &evaluation,
        "b
c
",
        &second,
    );
    assert_eq!(kept(&third), ["z/copy0", "z/copy1"]);
}

#[test]
fn the_identity_of_a_converter_names_its_revision_once_it_has_one() {
    let identity = |copies: Copies| {
        let converters: Vec<Box<dyn TableConverter>> = vec![Box::new(copies)];
        evaluation_identity(&converters)["table copies"].clone()
    };
    assert_eq!(identity(Copies::default()), "copies 3");
    assert_eq!(identity(Copies { revision: 2 }), "copies 3 revision 2");
}
