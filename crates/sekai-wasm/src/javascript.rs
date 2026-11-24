//! The interface for JavaScript.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use js_sys::{Function, Uint8Array};
use sekai_pipeline::{PlannedAsset, RootKey};
use sekai_unity::{ByteSource, Chunk};
use serde::Serialize;
use wasm_bindgen::prelude::*;
use web_sys::{File, FileReaderSync};

use crate::explorer::Explorer as Session;
use crate::{MemoryPayloads, hex};

fn failed(error: JsValue) -> io::Error {
    io::Error::other(format!("{error:?}"))
}

/// Evaluates bundles given as parallel lists of names and bytes. `on_file(path, bytes)` receives
/// each file of the store and `on_asset(row)` each row of the list of assets. Returns the report.
#[wasm_bindgen]
pub fn evaluate(
    names: Vec<String>,
    bundles: Vec<Uint8Array>,
    on_file: &Function,
    on_asset: &Function,
) -> Result<String, JsError> {
    let bundles = names
        .into_iter()
        .zip(bundles.iter().map(Uint8Array::to_vec))
        .collect();
    let report = super::run(
        bundles,
        &mut |key, bytes| {
            let path = JsValue::from(key.relative_path());
            let bytes = JsValue::from(Uint8Array::from(bytes));
            on_file
                .call2(&JsValue::NULL, &path, &bytes)
                .map(drop)
                .map_err(failed)
        },
        &mut |output| {
            on_asset
                .call1(&JsValue::NULL, &JsValue::from(super::row(output)))
                .map(drop)
                .map_err(failed)
        },
    )?;
    Ok(report.to_string())
}

/// Writes the object documents of bundles given as parallel lists of names and bytes.
/// `on_file(path, bytes)` receives each file, at the path the native `objects` command writes it
/// to. Returns the bundles that failed, one `<name>\t<reason>` line each.
#[wasm_bindgen]
pub fn objects(
    names: Vec<String>,
    bundles: Vec<Uint8Array>,
    on_file: &Function,
) -> Result<String, JsError> {
    let bundles = names
        .into_iter()
        .zip(bundles.iter().map(Uint8Array::to_vec))
        .collect();
    let errors = super::objects(bundles, &mut |path, bytes| {
        let path = JsValue::from(path);
        let bytes = JsValue::from(Uint8Array::from(bytes));
        on_file
            .call2(&JsValue::NULL, &path, &bytes)
            .map(drop)
            .map_err(failed)
    })?;
    Ok(errors
        .iter()
        .map(|(name, reason)| format!("{name}\t{reason}\n"))
        .collect())
}

/// A set of bundles kept open, read from `File`s as questions need them.
///
/// Files are read with `FileReaderSync`, so an explorer works only in a worker. Results that list
/// things are JSON text; path IDs in them are decimal strings, as in object documents.
#[wasm_bindgen]
pub struct Explorer(Session);

#[wasm_bindgen]
impl Explorer {
    /// Opens the bundles `files`, named by `names`, the two lists in the same order.
    #[wasm_bindgen(constructor)]
    pub fn new(names: Vec<String>, files: Vec<File>) -> Result<Explorer, JsError> {
        let held = Held::new(files);
        let bundles = names.into_iter().enumerate().map(|(index, name)| {
            let file: Arc<dyn ByteSource> = Arc::new(held.file(index));
            (name, file)
        });
        Ok(Explorer(Session::open(bundles.collect())?))
    }

    /// The bundles that were indexed, in name order.
    pub fn bundles(&self) -> Vec<String> {
        self.0.bundles().map(str::to_owned).collect()
    }

    /// The bundles that could not be opened or indexed, as `[[name, reason], …]`.
    pub fn failed(&self) -> Result<String, JsError> {
        Ok(serde_json::to_string(self.0.failed())?)
    }

    /// The object document of `bundle`. `on_payload(sha256, bytes)` receives each payload too
    /// large to inline, by its SHA-256 in lowercase hexadecimal.
    pub fn document(&self, bundle: &str, on_payload: &Function) -> Result<Vec<u8>, JsError> {
        let mut payloads = MemoryPayloads::default();
        let document = self
            .0
            .document(bundle, &mut payloads)
            .map_err(|e| JsError::new(&e))?;
        for (sha256, bytes) in &payloads.0 {
            let sha256 = JsValue::from(hex(sha256));
            let bytes = JsValue::from(Uint8Array::from(bytes.as_slice()));
            on_payload
                .call2(&JsValue::NULL, &sha256, &bytes)
                .map_err(failed)?;
        }
        Ok(document)
    }

    /// The assets `bundle` plans, in name order, as
    /// `[{"name", "form", "path_id", "file"?} | {"name", "form", "file"}, …]`: an asset starts
    /// at an object, named by its path ID and, in a bundle of several serialized files, its
    /// file, or at a file of the bundle that is not a serialized file.
    pub fn assets(&self, bundle: &str) -> Result<String, JsError> {
        let assets = self.0.assets(bundle)?;
        let rows: Vec<AssetRow<'_>> = assets.iter().map(AssetRow::from).collect();
        Ok(serde_json::to_string(&rows)?)
    }

    /// The asset document of the asset `name`, converted if it has not been. `undefined` if no
    /// bundle has such an asset or its root holds none.
    pub fn convert(&self, name: &str) -> Result<Option<Vec<u8>>, JsError> {
        Ok(self.0.convert(name)?)
    }

    /// The objects the conversion of the asset `name` may read, its root first, as the
    /// `sources` of asset documents list objects. `undefined` if no bundle has such an asset.
    pub fn closure(&self, name: &str) -> Result<Option<String>, JsError> {
        let closure = self.0.closure(name)?;
        Ok(closure.map(|c| serde_json::to_string(&c)).transpose()?)
    }

    /// A file of a converted asset, by its name `<sha256>.<extension>`, as asset documents name
    /// their members.
    pub fn file(&self, name: &str) -> Result<Vec<u8>, JsError> {
        Ok(self.0.file(&name.parse()?)?)
    }
}

/// An asset in the list [`Explorer::assets`] returns.
#[derive(Serialize)]
struct AssetRow<'a> {
    name: &'a str,
    form: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<&'a str>,
}

impl<'a> From<&'a PlannedAsset> for AssetRow<'a> {
    fn from(asset: &'a PlannedAsset) -> Self {
        let (path_id, file) = match &asset.root {
            RootKey::Object { file, path_id } => (Some(path_id.to_string()), file.as_deref()),
            RootKey::File { file } => (None, Some(file.as_str())),
        };
        AssetRow {
            name: &asset.name,
            form: asset.form,
            path_id,
            file,
        }
    }
}

thread_local! {
    /// The files of every open explorer, by the number of their [`Held`].
    static FILES: RefCell<BTreeMap<u32, Vec<File>>> = RefCell::default();
}

/// The files of one explorer, held until the last of them is dropped. Byte sources must be
/// shareable across threads, which JavaScript values are not, so they name their file by this
/// number and its position.
struct Held(u32);

impl Held {
    fn new(files: Vec<File>) -> Arc<Held> {
        let number = FILES.with_borrow_mut(|held| {
            let number = held.last_key_value().map_or(0, |(last, _)| last + 1);
            held.insert(number, files);
            number
        });
        Arc::new(Held(number))
    }

    /// The file at `index`.
    fn file(self: &Arc<Self>, index: usize) -> HeldFile {
        let len = FILES.with_borrow(|files| files[&self.0][index].size()) as u64;
        HeldFile {
            held: self.clone(),
            index,
            len,
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        FILES.with_borrow_mut(|held| held.remove(&self.0));
    }
}

/// One held file, read a range at a time.
struct HeldFile {
    held: Arc<Held>,
    index: usize,
    len: u64,
}

impl HeldFile {
    fn read_range(&self, start: f64, end: f64) -> Result<Vec<u8>, JsValue> {
        let blob = FILES.with_borrow(|files| {
            files[&self.held.0][self.index].slice_with_f64_and_f64(start, end)
        })?;
        let buffer = FileReaderSync::new()?.read_as_array_buffer(&blob)?;
        Ok(Uint8Array::new(&buffer).to_vec())
    }
}

impl ByteSource for HeldFile {
    fn len(&self) -> u64 {
        self.len
    }

    fn read(&self, offset: u64, len: usize) -> sekai_unity::Result<Chunk<'_>> {
        let available = self.len.saturating_sub(offset);
        if len as u64 > available {
            return Err(sekai_unity::Error::Truncated {
                what: "file",
                offset,
                needed: len as u64,
                available,
            });
        }
        let end = offset + len as u64;
        let bytes = self
            .read_range(offset as f64, end as f64)
            .map_err(|error| sekai_unity::Error::Read {
                path: FILES.with_borrow(|files| files[&self.held.0][self.index].name()),
                message: format!("{error:?}"),
            })?;
        Ok(Chunk::Owned(bytes))
    }
}
