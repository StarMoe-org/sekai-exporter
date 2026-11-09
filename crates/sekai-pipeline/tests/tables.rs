//! The table stage on assets and tables made here.

use std::io;

use sekai_catalog::{Key, MemoryStore, Store};
use sekai_convert::tables::{TableAsset, TableContext, TableConverter, TablePlan};
use sekai_convert::{Asset, Content, ConvertError, Member};
use sekai_format::{AssetDocument, MemberFile};
use sekai_pipeline::eval::{Evaluation, Output};
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::tables::{TableStage, evaluate};

/// Builds one asset per line of the table `rows`. A line names a bundle, whose first asset and
/// its member `m` the asset copies; a line `fail` is an asset error.
struct Copies;

struct Plan<'a> {
    cx: &'a dyn TableContext,
    names: Vec<String>,
    lines: Vec<String>,
}

impl TablePlan for Plan<'_> {
    fn names(&self) -> &[String] {
        &self.names
    }

    fn convert(&self, index: usize) -> Result<TableAsset, ConvertError> {
        let line = &self.lines[index];
        if line == "fail" {
            return Err(ConvertError::new("malformed", "a failing row"));
        }
        let source = self
            .cx
            .assets(line)
            .first()
            .expect("the bundle has an asset");
        assert_eq!(source.form, "text");
        Ok(TableAsset {
            asset: Asset {
                members: vec![Member {
                    role: "copy".to_owned(),
                    content: Content::File {
                        extension: "txt".to_owned(),
                        bytes: self.cx.member(&source.name, "m")?,
                    },
                }],
            },
            references: vec![source.name.clone(), source.name.clone()],
        })
    }
}

impl TableConverter for Copies {
    fn name(&self) -> &'static str {
        "copies"
    }

    fn version(&self) -> u32 {
        3
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        let text = String::from_utf8(cx.table("rows")?).expect("text");
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let names = (0..lines.len()).map(|i| format!("z/copy{i}")).collect();
        Ok(Box::new(Plan { cx, names, lines }))
    }
}

/// An evaluation with one asset `b/x` of the bundle `b`, whose member `m` holds `content`.
fn version(store: &MemoryStore, content: &[u8]) -> Evaluation {
    let member = store.put(content, "txt").unwrap();
    let document = AssetDocument {
        format: "asset".to_owned(),
        version: 1,
        name: "b/x".to_owned(),
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
    Evaluation {
        outputs: vec![Output {
            name: "b/x".to_owned(),
            bundle: "b".to_owned(),
            form: "text",
            document: store.put(&bytes, "json").unwrap(),
            error: None,
        }],
        ..Evaluation::default()
    }
}

fn run(store: &MemoryStore, evaluation: &mut Evaluation, rows: &'static str) -> io::Result<()> {
    let converters: Vec<Box<dyn TableConverter>> = vec![Box::new(Copies)];
    let read = move |name: &str| match name {
        "rows" => Ok(rows.as_bytes().to_vec()),
        other => Err(io::Error::other(format!("no table {other}"))),
    };
    evaluate(
        &TableStage {
            converters: &converters,
            read: &read,
            store,
            recorder: &NoRecorder,
            threads: 2,
        },
        evaluation,
    )
}

fn read(store: &MemoryStore, key: &Key) -> AssetDocument {
    AssetDocument::from_bytes(&store.read(key).unwrap()).unwrap()
}

#[test]
fn assets_are_built_from_rows_and_the_assets_of_the_version() {
    let store = MemoryStore::default();
    let mut evaluation = version(&store, b"hello");
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
        let mut evaluation = version(&store, content);
        run(&store, &mut evaluation, "b\n").unwrap();
        read(&store, &evaluation.outputs[1].document).fingerprint
    };
    assert_eq!(fingerprint(b"one"), fingerprint(b"one"));
    assert_ne!(fingerprint(b"one"), fingerprint(b"two"));
}

#[test]
fn a_row_that_cannot_be_built_is_an_asset_error() {
    let store = MemoryStore::default();
    let mut evaluation = version(&store, b"hello");
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
        fn plan<'a>(
            &self,
            cx: &'a dyn TableContext,
        ) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
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
        },
        &mut Evaluation::default(),
    )
    .expect_err("the run fails");
    assert!(error.to_string().contains("unreachable host"), "{error}");
}
