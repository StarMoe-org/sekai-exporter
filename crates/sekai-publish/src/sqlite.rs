//! Deterministic SQLite 3 publication index (`store/<aa>/<sha256>.sqlite`).
//!
//! Built bottom-up on fixed 4096-byte pages with no freelist pages, so HTTP `Range` clients
//! (`sql.js-httpvfs`, `wa-sqlite`) resolve directory listings, exact path lookups, bundle/form
//! filters, and full asset metadata in 1–3 page reads without thousands of JSON sidecars.

use std::collections::{BTreeMap, BTreeSet};

use sekai_catalog::LocalStore;
use sekai_catalog::store::parse_hex;

use crate::layout::Object;
use crate::plan::{File, Plan, PublishError, PublishedAsset};

const PAGE_SIZE: usize = 4096;
const USABLE_OVERFLOW: usize = PAGE_SIZE - 4;
const TABLE_MAX_LOCAL: usize = PAGE_SIZE - 35; // 4061
const INDEX_MAX_LOCAL: usize = ((PAGE_SIZE - 12) * 64 / 255) - 23; // 1002
const MIN_LOCAL: usize = ((PAGE_SIZE - 12) * 32 / 255) - 23; // 489
const SCHEMA_VERSION: i64 = 2;

#[derive(Clone)]
enum SqlValue {
    Null,
    Int(i64),
    Text(String),
    Blob(Vec<u8>),
}

/// Builds the `index.sqlite` database of `plan` and writes it to `store`.
pub(crate) fn build(store: &LocalStore, plan: &Plan) -> Result<Object, PublishError> {
    let wrap_doc = PublishError::wrap("documents");
    let rows = prepare_rows(plan);

    let mut db = DbWriter::new();
    let roots = Roots {
        meta: write_meta(&mut db, plan),
        bundles: write_bundles(&mut db, &rows.bundles),
        forms: write_forms(&mut db, &rows.forms),
        directories: write_directories(&mut db, &rows.directories),
        assets: write_assets(&mut db, &rows.assets),
        files: write_files(&mut db, &rows.files),
    };
    let sqlite_bytes = db.finalize(&schema_entries(&roots));
    let key = store.put(&sqlite_bytes, "sqlite").map_err(&wrap_doc)?;
    Ok(Object {
        key,
        size: sqlite_bytes.len() as u64,
        media_type: "application/vnd.sqlite3".to_owned(),
    })
}

/// The rows of every table and index, each in the order it is written.
struct Rows {
    bundles: Vec<BundleRow>,
    forms: Vec<FormRow>,
    directories: Vec<DirectoryRow>,
    assets: Vec<AssetRow>,
    /// Sorted by the primary key `(dir, file_name)`.
    files: Vec<FileRow>,
}

/// The bundle, form, asset and file rows of a plan.
struct Catalog {
    bundles: Vec<BundleRow>,
    forms: Vec<FormRow>,
    assets: Vec<AssetRow>,
    files: Vec<FileRow>,
}

struct BundleRow {
    id: i64,
    name: String,
    asset_count: i64,
    file_count: i64,
    total_size: i64,
}

struct FormRow {
    id: i64,
    name: String,
    form_version: i64,
    asset_count: i64,
    total_size: i64,
}

struct DirectoryRow {
    parent: String,
    name: String,
    path: String,
    file_count: i64,
    total_size: i64,
}

struct AssetRow {
    id: i64,
    bundle_id: i64,
    name: String,
    form_id: i64,
    fingerprint: Vec<u8>,
    total_size: i64,
    error_code: Option<String>,
    error_message: Option<String>,
    sources_json: String,
    resources_json: String,
    references_json: String,
}

struct FileRow {
    dir: String,
    file_name: String,
    path: String,
    bundle_id: i64,
    asset_id: i64,
    role: String,
    sha256: [u8; 32],
    extension: String,
    size: i64,
    media_type: String,
}

/// Root pages of every table and index, as the schema lists them.
struct Roots {
    meta: u32,
    bundles: NamedRoots,
    forms: NamedRoots,
    directories: u32,
    assets: AssetRoots,
    files: FileRoots,
}

/// Root pages of a dictionary table and its unique name index.
struct NamedRoots {
    table: u32,
    name_index: u32,
}

/// Root pages of the `assets` table and its indexes.
struct AssetRoots {
    table: u32,
    name_index: u32,
    bundle_index: u32,
    form_index: u32,
    error_index: u32,
}

/// Root pages of the `files` table and its indexes.
struct FileRoots {
    table: u32,
    path_index: u32,
    asset_index: u32,
}

/// Asset, file and size totals of the assets in one bundle or form.
#[derive(Default)]
struct Tally {
    assets: i64,
    files: i64,
    size: i64,
}

impl Tally {
    /// Counts one asset that owns `files`.
    fn add_asset(&mut self, files: &[&File]) {
        self.assets += 1;
        self.files += files.len() as i64;
        self.size = self.size.saturating_add(files_size(files));
    }
}

/// Derives the rows of every table from the plan's member files and assets.
fn prepare_rows(plan: &Plan) -> Rows {
    let member_files: Vec<&File> = plan
        .files
        .iter()
        .filter(|file| file.role != "document")
        .collect();
    let files_by_asset = group_by_asset(&member_files);
    let catalog = if plan.assets.is_empty() {
        fallback_catalog(plan, &member_files, &files_by_asset)
    } else {
        asset_catalog(plan, &files_by_asset)
    };

    let directories = directory_rows(&catalog.files);
    let mut files = catalog.files;
    files.sort_by(|a, b| {
        a.dir
            .cmp(&b.dir)
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    Rows {
        bundles: catalog.bundles,
        forms: catalog.forms,
        directories,
        assets: catalog.assets,
        files,
    }
}

/// Groups the member files by the name of their asset, keeping their path order.
fn group_by_asset<'a>(member_files: &[&'a File]) -> BTreeMap<&'a str, Vec<&'a File>> {
    let mut grouped: BTreeMap<&'a str, Vec<&'a File>> = BTreeMap::new();
    for &file in member_files {
        grouped.entry(file.asset.as_str()).or_default().push(file);
    }
    grouped
}

/// Rows for a plan that lists asset documents, with bundles and forms numbered by name.
fn asset_catalog(plan: &Plan, files_by_asset: &BTreeMap<&str, Vec<&File>>) -> Catalog {
    let mut bundle_tallies: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut form_tallies: BTreeMap<&str, (i64, Tally)> = BTreeMap::new();
    for asset in &plan.assets {
        let owned = owned_files(files_by_asset, &asset.name);
        bundle_tallies
            .entry(asset.bundle.as_str())
            .or_default()
            .add_asset(owned);
        let (_, form) = form_tallies
            .entry(asset.metadata.form.as_str())
            .or_insert((i64::from(asset.metadata.form_version), Tally::default()));
        if asset.metadata.error.is_none() {
            form.add_asset(owned);
        }
    }

    let bundle_ids = row_ids(bundle_tallies.keys().copied());
    let form_ids = row_ids(form_tallies.keys().copied());
    let bundles: Vec<BundleRow> = bundle_tallies
        .into_iter()
        .enumerate()
        .map(|(position, (name, tally))| BundleRow {
            id: row_id(position),
            name: name.to_owned(),
            asset_count: tally.assets,
            file_count: tally.files,
            total_size: tally.size,
        })
        .collect();
    let forms: Vec<FormRow> = form_tallies
        .into_iter()
        .enumerate()
        .map(|(position, (name, (form_version, tally)))| FormRow {
            id: row_id(position),
            name: name.to_owned(),
            form_version,
            asset_count: tally.assets,
            total_size: tally.size,
        })
        .collect();

    let mut assets = Vec::with_capacity(plan.assets.len());
    let mut files: Vec<FileRow> = Vec::new();
    for (position, asset) in plan.assets.iter().enumerate() {
        let asset_id = row_id(position);
        let owned = owned_files(files_by_asset, &asset.name);
        let bundle_id = bundle_ids[asset.bundle.as_str()];
        let form_id = form_ids[asset.metadata.form.as_str()];
        files.extend(
            owned
                .iter()
                .map(|&file| file_row(file, bundle_id, asset_id)),
        );
        assets.push(asset_row(
            asset,
            asset_id,
            bundle_id,
            form_id,
            files_size(owned),
        ));
    }
    Catalog {
        bundles,
        forms,
        assets,
        files,
    }
}

/// Rows for a plan without asset documents, such as a materialized view. Every asset sits in
/// the empty bundle and form, and the form totals stay zero.
fn fallback_catalog(
    plan: &Plan,
    member_files: &[&File],
    files_by_asset: &BTreeMap<&str, Vec<&File>>,
) -> Catalog {
    let names: BTreeSet<&str> = member_files
        .iter()
        .map(|&file| file.asset.as_str())
        .chain(plan.failures.iter().map(|failure| failure.name.as_str()))
        .collect();
    let failure_codes: BTreeMap<&str, &str> = plan
        .failures
        .iter()
        .map(|failure| (failure.name.as_str(), failure.code.as_str()))
        .collect();

    let mut bundle = Tally::default();
    let mut assets = Vec::with_capacity(names.len());
    let mut files: Vec<FileRow> = Vec::new();
    for (position, name) in names.into_iter().enumerate() {
        let asset_id = row_id(position);
        let owned = owned_files(files_by_asset, name);
        bundle.add_asset(owned);
        files.extend(owned.iter().map(|&file| file_row(file, 1, asset_id)));
        assets.push(AssetRow {
            id: asset_id,
            bundle_id: 1,
            name: name.to_owned(),
            form_id: 1,
            fingerprint: Vec::new(),
            total_size: files_size(owned),
            error_code: failure_codes.get(name).map(|code| (*code).to_owned()),
            error_message: None,
            sources_json: "[]".to_owned(),
            resources_json: "[]".to_owned(),
            references_json: "[]".to_owned(),
        });
    }
    Catalog {
        bundles: vec![BundleRow {
            id: 1,
            name: String::new(),
            asset_count: bundle.assets,
            file_count: bundle.files,
            total_size: bundle.size,
        }],
        forms: vec![FormRow {
            id: 1,
            name: String::new(),
            form_version: 0,
            asset_count: 0,
            total_size: 0,
        }],
        assets,
        files,
    }
}

/// The `assets` row of an asset document, with the ids of its bundle and form.
fn asset_row(asset: &PublishedAsset, id: i64, bundle_id: i64, form_id: i64, size: i64) -> AssetRow {
    let metadata = &asset.metadata;
    let (error_code, error_message) = match &metadata.error {
        Some(error) => (Some(error.code.clone()), Some(error.message.clone())),
        None => (None, None),
    };
    AssetRow {
        id,
        bundle_id,
        name: asset.name.clone(),
        form_id,
        fingerprint: parse_hex(&metadata.fingerprint)
            .map(|arr| arr.to_vec())
            .unwrap_or_else(|| metadata.fingerprint.as_bytes().to_vec()),
        total_size: size,
        error_code,
        error_message,
        sources_json: json_array(serde_json::to_string(&metadata.sources)),
        resources_json: json_array(serde_json::to_string(&metadata.resources)),
        references_json: json_array(serde_json::to_string(&metadata.references)),
    }
}

/// The JSON text of a list, or `[]` when it cannot be encoded.
fn json_array(encoded: serde_json::Result<String>) -> String {
    encoded.unwrap_or_else(|_| "[]".to_owned())
}

/// The `files` row of a member file that belongs to an asset.
fn file_row(file: &File, bundle_id: i64, asset_id: i64) -> FileRow {
    let (dir, file_name) = split_dir_file(&file.path);
    FileRow {
        dir: dir.to_owned(),
        file_name: file_name.to_owned(),
        path: file.path.clone(),
        bundle_id,
        asset_id,
        role: file.role.clone(),
        sha256: file.key.sha256,
        extension: file.key.extension.clone(),
        size: sql_size(file.size),
        media_type: file.media_type.clone(),
    }
}

/// The member files of the asset `name`, in path order.
fn owned_files<'a>(
    files_by_asset: &'a BTreeMap<&str, Vec<&'a File>>,
    name: &str,
) -> &'a [&'a File] {
    files_by_asset.get(name).map(Vec::as_slice).unwrap_or(&[])
}

/// The total size of `files` as an SQLite integer.
fn files_size(files: &[&File]) -> i64 {
    files
        .iter()
        .map(|file| sql_size(file.size))
        .fold(0, i64::saturating_add)
}

/// A byte count as an SQLite integer, saturating at `i64::MAX`.
fn sql_size(size: u64) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}

/// The row id of the entry at `position` in a sorted dictionary. Ids start at 1.
fn row_id(position: usize) -> i64 {
    (position + 1) as i64
}

/// The row id of each name, numbered in the order given.
fn row_ids<'a>(names: impl Iterator<Item = &'a str>) -> BTreeMap<&'a str, i64> {
    names
        .enumerate()
        .map(|(position, name)| (name, row_id(position)))
        .collect()
}

/// One summary row for each directory that holds a file, counting the files below it.
fn directory_rows(files: &[FileRow]) -> Vec<DirectoryRow> {
    let mut totals: BTreeMap<(String, String), DirectoryRow> = BTreeMap::new();
    for file in files.iter().filter(|file| !file.dir.is_empty()) {
        for dir in ancestor_dirs(&file.dir) {
            let (parent, name) = split_dir_file(dir);
            let row = totals
                .entry((parent.to_owned(), name.to_owned()))
                .or_insert_with(|| DirectoryRow {
                    parent: parent.to_owned(),
                    name: name.to_owned(),
                    path: dir.to_owned(),
                    file_count: 0,
                    total_size: 0,
                });
            row.file_count += 1;
            row.total_size = row.total_size.saturating_add(file.size);
        }
    }
    totals.into_values().collect()
}

/// The directory `dir` followed by each of its ancestors, up to the top-level directory.
fn ancestor_dirs(dir: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(dir), |cursor| {
        let (parent, _) = split_dir_file(cursor);
        (!parent.is_empty()).then_some(parent)
    })
}

/// The references to `rows`, ordered by `key`.
fn sorted_refs<'a, T, K: Ord>(rows: &'a [T], key: impl Fn(&'a T) -> K) -> Vec<&'a T> {
    let mut refs: Vec<&T> = rows.iter().collect();
    refs.sort_by_key(|row| key(*row));
    refs
}

/// Writes a rowid table with one row per item, keyed by `rowid` and encoded by `encode`.
fn write_table<T>(
    db: &mut DbWriter,
    rows: &[T],
    rowid: impl Fn(&T) -> i64,
    encode: impl Fn(&T) -> Vec<u8>,
) -> u32 {
    let payloads: Vec<(i64, Vec<u8>)> = rows.iter().map(|row| (rowid(row), encode(row))).collect();
    db.build_table_btree(&payloads, None)
}

/// Writes an index with one entry per item, encoded by `encode`.
fn write_index<T>(db: &mut DbWriter, rows: &[T], encode: impl Fn(&T) -> Vec<u8>) -> u32 {
    let payloads: Vec<Vec<u8>> = rows.iter().map(encode).collect();
    db.build_index_btree(&payloads)
}

/// Writes the `meta` table, whose rows are in key order.
fn write_meta(db: &mut DbWriter, plan: &Plan) -> u32 {
    let mut entries = vec![
        ("format", "sqlite-index".to_owned()),
        ("published_version", plan.version.clone()),
        ("revision", plan.revision.clone()),
        ("schema_version", SCHEMA_VERSION.to_string()),
    ];
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let payloads: Vec<Vec<u8>> = entries
        .into_iter()
        .map(|(key, value)| encode_record(&[SqlValue::Text(key.to_owned()), SqlValue::Text(value)]))
        .collect();
    db.build_index_btree(&payloads)
}

/// Writes the `bundles` table and its unique name index.
fn write_bundles(db: &mut DbWriter, bundles: &[BundleRow]) -> NamedRoots {
    let table = write_table(
        db,
        bundles,
        |row| row.id,
        |row| {
            encode_record(&[
                SqlValue::Null,
                SqlValue::Text(row.name.clone()),
                SqlValue::Int(row.asset_count),
                SqlValue::Int(row.file_count),
                SqlValue::Int(row.total_size),
            ])
        },
    );
    let name_index = write_index(db, bundles, |row| {
        encode_record(&[SqlValue::Text(row.name.clone()), SqlValue::Int(row.id)])
    });
    NamedRoots { table, name_index }
}

/// Writes the `forms` table and its unique name index.
fn write_forms(db: &mut DbWriter, forms: &[FormRow]) -> NamedRoots {
    let table = write_table(
        db,
        forms,
        |row| row.id,
        |row| {
            encode_record(&[
                SqlValue::Null,
                SqlValue::Text(row.name.clone()),
                SqlValue::Int(row.form_version),
                SqlValue::Int(row.asset_count),
                SqlValue::Int(row.total_size),
            ])
        },
    );
    let name_index = write_index(db, forms, |row| {
        encode_record(&[SqlValue::Text(row.name.clone()), SqlValue::Int(row.id)])
    });
    NamedRoots { table, name_index }
}

/// Writes the `directories` table, keyed by `(parent, name)`.
fn write_directories(db: &mut DbWriter, directories: &[DirectoryRow]) -> u32 {
    write_index(db, directories, |row| {
        encode_record(&[
            SqlValue::Text(row.parent.clone()),
            SqlValue::Text(row.name.clone()),
            SqlValue::Text(row.path.clone()),
            SqlValue::Int(row.file_count),
            SqlValue::Int(row.total_size),
        ])
    })
}

/// Writes the `assets` table and its name, bundle, form and error indexes.
fn write_assets(db: &mut DbWriter, assets: &[AssetRow]) -> AssetRoots {
    let table = write_table(db, assets, |row| row.id, asset_record);
    // The asset rows are already in name order.
    let name_index = write_index(db, assets, |row| {
        encode_record(&[SqlValue::Text(row.name.clone()), SqlValue::Int(row.id)])
    });
    let by_bundle = sorted_refs(assets, |row| (row.bundle_id, &row.name, row.id));
    let bundle_index = write_index(db, &by_bundle, |row| {
        encode_record(&[
            SqlValue::Int(row.bundle_id),
            SqlValue::Text(row.name.clone()),
            SqlValue::Int(row.id),
        ])
    });
    let by_form = sorted_refs(assets, |row| (row.form_id, &row.name, row.id));
    let form_index = write_index(db, &by_form, |row| {
        encode_record(&[
            SqlValue::Int(row.form_id),
            SqlValue::Text(row.name.clone()),
            SqlValue::Int(row.id),
        ])
    });
    let by_error = sorted_refs(assets, |row| (row.error_code.as_deref(), &row.name, row.id));
    let error_index = write_index(db, &by_error, |row| {
        encode_record(&[
            nullable_text(&row.error_code),
            SqlValue::Text(row.name.clone()),
            SqlValue::Int(row.id),
        ])
    });
    AssetRoots {
        table,
        name_index,
        bundle_index,
        form_index,
        error_index,
    }
}

/// The record of an `assets` row.
fn asset_record(row: &AssetRow) -> Vec<u8> {
    encode_record(&[
        SqlValue::Null,
        SqlValue::Int(row.bundle_id),
        SqlValue::Text(row.name.clone()),
        SqlValue::Int(row.form_id),
        SqlValue::Blob(row.fingerprint.clone()),
        SqlValue::Int(row.total_size),
        nullable_text(&row.error_code),
        nullable_text(&row.error_message),
        SqlValue::Text(row.sources_json.clone()),
        SqlValue::Text(row.resources_json.clone()),
        SqlValue::Text(row.references_json.clone()),
    ])
}

/// A text column that is `NULL` when the value is absent.
fn nullable_text(value: &Option<String>) -> SqlValue {
    value
        .as_ref()
        .map_or(SqlValue::Null, |text| SqlValue::Text(text.clone()))
}

/// Writes the `files` table and its path and asset indexes.
fn write_files(db: &mut DbWriter, files: &[FileRow]) -> FileRoots {
    let table = write_index(db, files, |row| {
        encode_record(&[
            SqlValue::Text(row.dir.clone()),
            SqlValue::Text(row.file_name.clone()),
            SqlValue::Text(row.path.clone()),
            SqlValue::Int(row.bundle_id),
            SqlValue::Int(row.asset_id),
            SqlValue::Text(row.role.clone()),
            SqlValue::Blob(row.sha256.to_vec()),
            SqlValue::Text(row.extension.clone()),
            SqlValue::Int(row.size),
            SqlValue::Text(row.media_type.clone()),
        ])
    });
    let by_path = sorted_refs(files, |row| (&row.path, &row.dir, &row.file_name));
    let path_index = write_index(db, &by_path, |row| {
        encode_record(&[
            SqlValue::Text(row.path.clone()),
            SqlValue::Text(row.dir.clone()),
            SqlValue::Text(row.file_name.clone()),
        ])
    });
    let by_asset = sorted_refs(files, |row| {
        (row.asset_id, &row.role, &row.dir, &row.file_name)
    });
    let asset_index = write_index(db, &by_asset, |row| {
        encode_record(&[
            SqlValue::Int(row.asset_id),
            SqlValue::Text(row.role.clone()),
            SqlValue::Text(row.dir.clone()),
            SqlValue::Text(row.file_name.clone()),
        ])
    });
    FileRoots {
        table,
        path_index,
        asset_index,
    }
}

/// The `sqlite_schema` entries: kind, name, table name, root page and `CREATE` statement.
fn schema_entries(
    roots: &Roots,
) -> Vec<(&'static str, &'static str, &'static str, u32, &'static str)> {
    vec![
        (
            "table",
            "meta",
            "meta",
            roots.meta,
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID",
        ),
        (
            "table",
            "bundles",
            "bundles",
            roots.bundles.table,
            "CREATE TABLE bundles (id INTEGER PRIMARY KEY, name TEXT NOT NULL, asset_count INTEGER NOT NULL, file_count INTEGER NOT NULL, total_size INTEGER NOT NULL)",
        ),
        (
            "index",
            "idx_bundles_name",
            "bundles",
            roots.bundles.name_index,
            "CREATE UNIQUE INDEX idx_bundles_name ON bundles (name)",
        ),
        (
            "table",
            "forms",
            "forms",
            roots.forms.table,
            "CREATE TABLE forms (id INTEGER PRIMARY KEY, name TEXT NOT NULL, form_version INTEGER NOT NULL, asset_count INTEGER NOT NULL, total_size INTEGER NOT NULL)",
        ),
        (
            "index",
            "idx_forms_name",
            "forms",
            roots.forms.name_index,
            "CREATE UNIQUE INDEX idx_forms_name ON forms (name)",
        ),
        (
            "table",
            "directories",
            "directories",
            roots.directories,
            "CREATE TABLE directories (parent TEXT NOT NULL, name TEXT NOT NULL, path TEXT NOT NULL, file_count INTEGER NOT NULL, total_size INTEGER NOT NULL, PRIMARY KEY (parent, name)) WITHOUT ROWID",
        ),
        (
            "table",
            "assets",
            "assets",
            roots.assets.table,
            "CREATE TABLE assets (id INTEGER PRIMARY KEY, bundle_id INTEGER NOT NULL, name TEXT NOT NULL, form_id INTEGER NOT NULL, fingerprint BLOB NOT NULL, total_size INTEGER NOT NULL, error_code TEXT, error_message TEXT, sources_json TEXT NOT NULL, resources_json TEXT NOT NULL, references_json TEXT NOT NULL)",
        ),
        (
            "index",
            "idx_assets_name",
            "assets",
            roots.assets.name_index,
            "CREATE UNIQUE INDEX idx_assets_name ON assets (name)",
        ),
        (
            "index",
            "idx_assets_bundle",
            "assets",
            roots.assets.bundle_index,
            "CREATE INDEX idx_assets_bundle ON assets (bundle_id, name)",
        ),
        (
            "index",
            "idx_assets_form",
            "assets",
            roots.assets.form_index,
            "CREATE INDEX idx_assets_form ON assets (form_id, name)",
        ),
        (
            "index",
            "idx_assets_error",
            "assets",
            roots.assets.error_index,
            "CREATE INDEX idx_assets_error ON assets (error_code, name)",
        ),
        (
            "table",
            "files",
            "files",
            roots.files.table,
            "CREATE TABLE files (dir TEXT NOT NULL, file_name TEXT NOT NULL, path TEXT NOT NULL, bundle_id INTEGER NOT NULL, asset_id INTEGER NOT NULL, role TEXT NOT NULL, sha256 BLOB NOT NULL, extension TEXT NOT NULL, size INTEGER NOT NULL, media_type TEXT NOT NULL, PRIMARY KEY (dir, file_name)) WITHOUT ROWID",
        ),
        (
            "index",
            "idx_files_path",
            "files",
            roots.files.path_index,
            "CREATE UNIQUE INDEX idx_files_path ON files (path)",
        ),
        (
            "index",
            "idx_files_asset",
            "files",
            roots.files.asset_index,
            "CREATE INDEX idx_files_asset ON files (asset_id, role)",
        ),
    ]
}

fn split_dir_file(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

struct DbWriter {
    pages: Vec<Vec<u8>>,
}

impl DbWriter {
    fn new() -> Self {
        // Reserve page 1 for `sqlite_schema`.
        Self {
            pages: vec![vec![0u8; PAGE_SIZE]],
        }
    }

    fn alloc_page(&mut self) -> u32 {
        self.pages.push(vec![0u8; PAGE_SIZE]);
        u32::try_from(self.pages.len()).expect("sqlite page count fits u32")
    }

    fn write_overflow(&mut self, tail: &[u8]) -> u32 {
        let chunks: Vec<&[u8]> = tail.chunks(USABLE_OVERFLOW).collect();
        let mut next_page = 0u32;
        for chunk in chunks.into_iter().rev() {
            let pg = self.alloc_page();
            let buf = &mut self.pages[(pg - 1) as usize];
            buf[..4].copy_from_slice(&next_page.to_be_bytes());
            buf[4..4 + chunk.len()].copy_from_slice(chunk);
            next_page = pg;
        }
        next_page
    }

    fn make_table_leaf_cell(&mut self, rowid: i64, payload: &[u8]) -> Vec<u8> {
        let p = payload.len();
        let mut cell = Vec::with_capacity(p.min(TABLE_MAX_LOCAL) + 22);
        put_varint(&mut cell, p as u64);
        put_varint(&mut cell, rowid as u64);
        if p <= TABLE_MAX_LOCAL {
            cell.extend_from_slice(payload);
        } else {
            let k = MIN_LOCAL + ((p - MIN_LOCAL) % USABLE_OVERFLOW);
            let local = if k <= TABLE_MAX_LOCAL { k } else { MIN_LOCAL };
            let ovf = self.write_overflow(&payload[local..]);
            cell.extend_from_slice(&payload[..local]);
            cell.extend_from_slice(&ovf.to_be_bytes());
        }
        cell
    }

    fn make_index_cell_body(&mut self, payload: &[u8]) -> Vec<u8> {
        let p = payload.len();
        let mut body = Vec::with_capacity(p.min(INDEX_MAX_LOCAL) + 14);
        put_varint(&mut body, p as u64);
        if p <= INDEX_MAX_LOCAL {
            body.extend_from_slice(payload);
        } else {
            let k = MIN_LOCAL + ((p - MIN_LOCAL) % USABLE_OVERFLOW);
            let local = if k <= INDEX_MAX_LOCAL { k } else { MIN_LOCAL };
            let ovf = self.write_overflow(&payload[local..]);
            body.extend_from_slice(&payload[..local]);
            body.extend_from_slice(&ovf.to_be_bytes());
        }
        body
    }

    fn write_slotted_page(
        &mut self,
        pg: u32,
        flag: u8,
        cells: &[Vec<u8>],
        right_child: Option<u32>,
        header_offset: usize,
    ) {
        let buf = &mut self.pages[(pg - 1) as usize];
        let hdr_len = if right_child.is_some() { 12 } else { 8 };
        let used: usize = cells.iter().map(|cell| 2 + cell.len()).sum();
        assert!(
            header_offset + hdr_len + used <= PAGE_SIZE,
            "{} cells of {used} bytes overflow page {pg}",
            cells.len()
        );
        let mut content = PAGE_SIZE;
        let mut ptrs = Vec::with_capacity(cells.len());
        for cell in cells {
            content -= cell.len();
            buf[content..content + cell.len()].copy_from_slice(cell);
            ptrs.push(u16::try_from(content).expect("page size is 4096"));
        }

        buf[header_offset] = flag;
        buf[header_offset + 1..header_offset + 3].copy_from_slice(&0u16.to_be_bytes());
        let cell_count = u16::try_from(cells.len()).expect("cell count fits u16");
        buf[header_offset + 3..header_offset + 5].copy_from_slice(&cell_count.to_be_bytes());
        let content_field = u16::try_from(content).unwrap_or(0);
        buf[header_offset + 5..header_offset + 7].copy_from_slice(&content_field.to_be_bytes());
        buf[header_offset + 7] = 0;
        if let Some(rc) = right_child {
            buf[header_offset + 8..header_offset + 12].copy_from_slice(&rc.to_be_bytes());
        }
        let mut pos = header_offset + hdr_len;
        for ptr in ptrs {
            buf[pos..pos + 2].copy_from_slice(&ptr.to_be_bytes());
            pos += 2;
        }
    }

    fn build_table_btree(&mut self, rows: &[(i64, Vec<u8>)], fixed_root: Option<u32>) -> u32 {
        let root_hdr_off = if fixed_root == Some(1) { 100 } else { 0 };
        let cells: Vec<(i64, Vec<u8>)> = rows
            .iter()
            .map(|(rowid, payload)| (*rowid, self.make_table_leaf_cell(*rowid, payload)))
            .collect();

        let root_avail = PAGE_SIZE - root_hdr_off - 8;
        let total: usize = cells.iter().map(|(_, c)| c.len() + 2).sum();
        if total <= root_avail {
            let pg = fixed_root.unwrap_or_else(|| self.alloc_page());
            let raw_cells: Vec<Vec<u8>> = cells.into_iter().map(|(_, c)| c).collect();
            self.write_slotted_page(pg, 0x0d, &raw_cells, None, root_hdr_off);
            return pg;
        }

        let leaf_avail = PAGE_SIZE - 8;
        let mut leaves: Vec<(u32, i64)> = Vec::new();
        let mut cur: Vec<(i64, Vec<u8>)> = Vec::new();
        let mut cur_used = 0usize;
        for (rowid, cell) in cells {
            let need = cell.len() + 2;
            if !cur.is_empty() && cur_used + need > leaf_avail {
                let max_id = cur.last().unwrap().0;
                let raw: Vec<Vec<u8>> = cur.into_iter().map(|(_, c)| c).collect();
                let pg = self.alloc_page();
                self.write_slotted_page(pg, 0x0d, &raw, None, 0);
                leaves.push((pg, max_id));
                cur = Vec::new();
                cur_used = 0;
            }
            cur.push((rowid, cell));
            cur_used += need;
        }
        let max_id = cur.last().unwrap().0;
        let raw: Vec<Vec<u8>> = cur.into_iter().map(|(_, c)| c).collect();
        let pg = self.alloc_page();
        self.write_slotted_page(pg, 0x0d, &raw, None, 0);
        leaves.push((pg, max_id));

        self.finish_table_interior(leaves, fixed_root)
    }

    fn finish_table_interior(
        &mut self,
        mut children: Vec<(u32, i64)>,
        fixed_root: Option<u32>,
    ) -> u32 {
        let root_hdr_off = if fixed_root == Some(1) { 100 } else { 0 };
        loop {
            let right_pg = children.last().unwrap().0;
            let icells: Vec<Vec<u8>> = children[..children.len() - 1]
                .iter()
                .map(|(cpg, max_id)| make_table_interior_cell(*cpg, *max_id))
                .collect();
            let root_avail = PAGE_SIZE - root_hdr_off - 12;
            let total: usize = icells.iter().map(|c| c.len() + 2).sum();
            if total <= root_avail {
                let pg = fixed_root.unwrap_or_else(|| self.alloc_page());
                self.write_slotted_page(pg, 0x05, &icells, Some(right_pg), root_hdr_off);
                return pg;
            }

            let int_avail = PAGE_SIZE - 12;
            let mut next_level = Vec::new();
            let mut i = 0usize;
            let n = children.len();
            while i < n {
                let mut j = i;
                let mut used = 0usize;
                while j + 1 < n {
                    let cell = make_table_interior_cell(children[j].0, children[j].1);
                    let need = cell.len() + 2;
                    if j > i && used + need > int_avail {
                        break;
                    }
                    // Never leave a single orphan child (`0` cells) for the final interior page:
                    // only take `n - 2` when `n - 1` will also fit on this same page.
                    if j + 3 == n {
                        let next_cell =
                            make_table_interior_cell(children[j + 1].0, children[j + 1].1);
                        if used + need + next_cell.len() + 2 > int_avail {
                            break;
                        }
                    }
                    used += need;
                    j += 1;
                }
                if j == i && i + 1 == n && !next_level.is_empty() {
                    // Unreachable given cell size <= 15 bytes, guarded defensively.
                    break;
                }
                let sub = &children[i..=j];
                let sub_cells: Vec<Vec<u8>> = sub[..sub.len() - 1]
                    .iter()
                    .map(|(cpg, mid)| make_table_interior_cell(*cpg, *mid))
                    .collect();
                let pg = self.alloc_page();
                self.write_slotted_page(pg, 0x05, &sub_cells, Some(sub.last().unwrap().0), 0);
                next_level.push((pg, sub.last().unwrap().1));
                i = j + 1;
            }
            children = next_level;
        }
    }

    fn build_index_btree(&mut self, payloads: &[Vec<u8>]) -> u32 {
        let bodies: Vec<Vec<u8>> = payloads
            .iter()
            .map(|p| self.make_index_cell_body(p))
            .collect();
        let leaf_avail = PAGE_SIZE - 8;
        let total: usize = bodies.iter().map(|b| b.len() + 2).sum();
        if total <= leaf_avail {
            let pg = self.alloc_page();
            self.write_slotted_page(pg, 0x0a, &bodies, None, 0);
            return pg;
        }

        let mut leaves = Vec::new();
        let mut promoted = Vec::new();
        let mut cur: Vec<Vec<u8>> = Vec::new();
        let mut cur_used = 0usize;
        let n = bodies.len();
        let mut i = 0usize;
        while i < n {
            let need = bodies[i].len() + 2;
            if !cur.is_empty() && cur_used + need > leaf_avail {
                if i == n - 1 {
                    let sep = cur.pop().expect("at least two cells fit on a leaf");
                    let pg = self.alloc_page();
                    self.write_slotted_page(pg, 0x0a, &cur, None, 0);
                    leaves.push(pg);
                    promoted.push(sep);
                    cur = vec![bodies[i].clone()];
                    cur_used = need;
                    i += 1;
                    continue;
                }
                let pg = self.alloc_page();
                self.write_slotted_page(pg, 0x0a, &cur, None, 0);
                leaves.push(pg);
                promoted.push(bodies[i].clone());
                cur = Vec::new();
                cur_used = 0;
                i += 1;
                continue;
            }
            cur.push(bodies[i].clone());
            cur_used += need;
            i += 1;
        }
        let pg = self.alloc_page();
        self.write_slotted_page(pg, 0x0a, &cur, None, 0);
        leaves.push(pg);

        self.finish_index_interior(leaves, promoted)
    }

    fn finish_index_interior(&mut self, mut children: Vec<u32>, mut seps: Vec<Vec<u8>>) -> u32 {
        let int_avail = PAGE_SIZE - 12;
        loop {
            let icells: Vec<Vec<u8>> = children[..seps.len()]
                .iter()
                .zip(&seps)
                .map(|(cpg, sep)| make_index_interior_cell(*cpg, sep))
                .collect();
            let total: usize = icells.iter().map(|c| c.len() + 2).sum();
            if total <= int_avail {
                let pg = self.alloc_page();
                self.write_slotted_page(pg, 0x02, &icells, Some(*children.last().unwrap()), 0);
                return pg;
            }

            let mut next_children = Vec::new();
            let mut next_seps = Vec::new();
            let mut cur_indices: Vec<usize> = Vec::new();
            let mut cur_used = 0usize;
            let m = seps.len();
            let mut k = 0usize;
            while k < m {
                let need = icells[k].len() + 2;
                if !cur_indices.is_empty() && cur_used + need > int_avail {
                    if k == m - 1 {
                        let prev = cur_indices
                            .pop()
                            .expect("at least two cells fit on an interior page");
                        let left_cells: Vec<Vec<u8>> =
                            cur_indices.iter().map(|&idx| icells[idx].clone()).collect();
                        let pg = self.alloc_page();
                        self.write_slotted_page(pg, 0x02, &left_cells, Some(children[prev]), 0);
                        next_children.push(pg);
                        next_seps.push(seps[prev].clone());
                        cur_indices = vec![k];
                        cur_used = need;
                        k += 1;
                        continue;
                    }
                    let left_cells: Vec<Vec<u8>> =
                        cur_indices.iter().map(|&idx| icells[idx].clone()).collect();
                    let pg = self.alloc_page();
                    self.write_slotted_page(pg, 0x02, &left_cells, Some(children[k]), 0);
                    next_children.push(pg);
                    next_seps.push(seps[k].clone());
                    cur_indices = Vec::new();
                    cur_used = 0;
                    k += 1;
                    continue;
                }
                cur_indices.push(k);
                cur_used += need;
                k += 1;
            }
            let right_cells: Vec<Vec<u8>> =
                cur_indices.iter().map(|&idx| icells[idx].clone()).collect();
            let pg = self.alloc_page();
            self.write_slotted_page(pg, 0x02, &right_cells, Some(*children.last().unwrap()), 0);
            next_children.push(pg);
            children = next_children;
            seps = next_seps;
        }
    }

    fn finalize(mut self, schema: &[(&str, &str, &str, u32, &str)]) -> Vec<u8> {
        let schema_rows: Vec<(i64, Vec<u8>)> = schema
            .iter()
            .enumerate()
            .map(|(idx, (kind, name, tbl_name, rootpage, sql))| {
                (
                    (idx + 1) as i64,
                    encode_record(&[
                        SqlValue::Text((*kind).to_owned()),
                        SqlValue::Text((*name).to_owned()),
                        SqlValue::Text((*tbl_name).to_owned()),
                        SqlValue::Int(i64::from(*rootpage)),
                        SqlValue::Text((*sql).to_owned()),
                    ]),
                )
            })
            .collect();
        self.build_table_btree(&schema_rows, Some(1));

        let page_count = u32::try_from(self.pages.len()).expect("page count fits u32");
        let p1 = &mut self.pages[0];
        p1[..16].copy_from_slice(b"SQLite format 3\0");
        p1[16..18].copy_from_slice(&(PAGE_SIZE as u16).to_be_bytes());
        p1[18] = 1; // write version: rollback journal (immutable static DB)
        p1[19] = 1; // read version
        p1[20] = 0; // reserved bytes per page
        p1[21] = 64; // maximum embedded payload fraction
        p1[22] = 32; // minimum embedded payload fraction
        p1[23] = 32; // leaf payload fraction
        p1[24..28].copy_from_slice(&1u32.to_be_bytes()); // file change counter
        p1[28..32].copy_from_slice(&page_count.to_be_bytes()); // database size in pages
        p1[32..36].copy_from_slice(&0u32.to_be_bytes()); // first freelist trunk page
        p1[36..40].copy_from_slice(&0u32.to_be_bytes()); // total freelist pages
        p1[40..44].copy_from_slice(&1u32.to_be_bytes()); // schema cookie
        p1[44..48].copy_from_slice(&4u32.to_be_bytes()); // schema format number = 4
        p1[56..60].copy_from_slice(&1u32.to_be_bytes()); // text encoding = 1 (UTF-8)
        p1[60..64].copy_from_slice(&(SCHEMA_VERSION as u32).to_be_bytes()); // user_version
        p1[92..96].copy_from_slice(&1u32.to_be_bytes()); // version-valid-for
        p1[96..100].copy_from_slice(&3_045_000u32.to_be_bytes()); // SQLite version

        let mut out = Vec::with_capacity(self.pages.len() * PAGE_SIZE);
        for page in self.pages {
            out.extend_from_slice(&page);
        }
        out
    }
}

fn make_table_interior_cell(left_child: u32, max_rowid: i64) -> Vec<u8> {
    let mut cell = Vec::with_capacity(13);
    cell.extend_from_slice(&left_child.to_be_bytes());
    put_varint(&mut cell, max_rowid as u64);
    cell
}

fn make_index_interior_cell(left_child: u32, body: &[u8]) -> Vec<u8> {
    let mut cell = Vec::with_capacity(4 + body.len());
    cell.extend_from_slice(&left_child.to_be_bytes());
    cell.extend_from_slice(body);
    cell
}

fn put_varint(out: &mut Vec<u8>, v: u64) {
    if v <= 0x7f {
        out.push(v as u8);
        return;
    }
    if v >= (1u64 << 56) {
        let mut buf = [0u8; 9];
        buf[8] = (v & 0xff) as u8;
        let mut cur = v >> 8;
        for slot in (0..8).rev() {
            buf[slot] = 0x80 | ((cur & 0x7f) as u8);
            cur >>= 7;
        }
        out.extend_from_slice(&buf);
        return;
    }
    let mut tmp = [0u8; 9];
    let mut len = 1usize;
    tmp[0] = (v & 0x7f) as u8;
    let mut cur = v >> 7;
    while cur > 0 {
        tmp[len] = 0x80 | ((cur & 0x7f) as u8);
        cur >>= 7;
        len += 1;
    }
    for idx in (0..len).rev() {
        out.push(tmp[idx]);
    }
}

fn varint_len(v: u64) -> usize {
    if v <= 0x7f {
        1
    } else if v >= (1u64 << 56) {
        9
    } else {
        let mut len = 1usize;
        let mut cur = v >> 7;
        while cur > 0 {
            cur >>= 7;
            len += 1;
        }
        len
    }
}

fn encode_value(value: &SqlValue, body: &mut Vec<u8>) -> u64 {
    match value {
        SqlValue::Null => 0,
        SqlValue::Int(0) => 8,
        SqlValue::Int(1) => 9,
        SqlValue::Int(v) => {
            let v = *v;
            if (-128..=127).contains(&v) {
                body.push(v as i8 as u8);
                1
            } else if (-32_768..=32_767).contains(&v) {
                body.extend_from_slice(&(v as i16).to_be_bytes());
                2
            } else if (-8_388_608..=8_388_607).contains(&v) {
                let b = v.to_be_bytes();
                body.extend_from_slice(&b[5..8]);
                3
            } else if (i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&v) {
                body.extend_from_slice(&(v as i32).to_be_bytes());
                4
            } else if (-(1i64 << 47)..=(1i64 << 47) - 1).contains(&v) {
                let b = v.to_be_bytes();
                body.extend_from_slice(&b[2..8]);
                5
            } else {
                body.extend_from_slice(&v.to_be_bytes());
                6
            }
        }
        SqlValue::Blob(bytes) => {
            body.extend_from_slice(bytes);
            12 + 2 * (bytes.len() as u64)
        }
        SqlValue::Text(text) => {
            body.extend_from_slice(text.as_bytes());
            13 + 2 * (text.len() as u64)
        }
    }
}

fn encode_record(cols: &[SqlValue]) -> Vec<u8> {
    let mut type_bytes = Vec::with_capacity(cols.len() * 2);
    let mut body = Vec::new();
    for col in cols {
        let serial = encode_value(col, &mut body);
        put_varint(&mut type_bytes, serial);
    }
    let mut header_size = (type_bytes.len() + 1) as u64;
    while varint_len(header_size) + type_bytes.len() != header_size as usize {
        header_size = (varint_len(header_size) + type_bytes.len()) as u64;
    }
    let mut out = Vec::with_capacity(header_size as usize + body.len());
    put_varint(&mut out, header_size);
    out.extend_from_slice(&type_bytes);
    out.extend_from_slice(&body);
    out
}
