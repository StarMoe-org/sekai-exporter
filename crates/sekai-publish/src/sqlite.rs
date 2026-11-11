//! Deterministic SQLite 3 publication index (`store/<aa>/<sha256>.sqlite`).
//!
//! Built bottom-up on fixed 4096-byte pages with no freelist pages, so HTTP `Range` clients
//! (`sql.js-httpvfs`, `wa-sqlite`) resolve directory listings, exact path lookups, bundle/form
//! filters, and full asset metadata in 1–3 page reads without thousands of JSON sidecars.

use std::collections::{BTreeMap, BTreeSet};

use sekai_catalog::LocalStore;
use sekai_catalog::store::parse_hex;

use crate::layout::Object;
use crate::plan::{File, Plan, PublishError};

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

    let member_files: Vec<&File> = plan
        .files
        .iter()
        .filter(|file| file.role != "document")
        .collect();

    // Group published member files by owning asset name (already in path order).
    let mut files_by_asset: BTreeMap<&str, Vec<&File>> = BTreeMap::new();
    for file in &member_files {
        files_by_asset
            .entry(file.asset.as_str())
            .or_default()
            .push(file);
    }

    // Build normalized bundle and form dictionaries in deterministic sorted order.
    let mut bundle_names = BTreeSet::new();
    let mut form_versions: BTreeMap<String, i64> = BTreeMap::new();
    if !plan.assets.is_empty() {
        for asset in &plan.assets {
            bundle_names.insert(asset.bundle.clone());
            form_versions
                .entry(asset.metadata.form.clone())
                .or_insert(i64::from(asset.metadata.form_version));
        }
    } else {
        bundle_names.insert(String::new());
        form_versions.insert(String::new(), 0);
    }

    let bundle_list: Vec<String> = bundle_names.into_iter().collect();
    let bundle_ids: BTreeMap<&str, i64> = bundle_list
        .iter()
        .enumerate()
        .map(|(idx, name)| (name.as_str(), (idx + 1) as i64))
        .collect();
    let form_list: Vec<(String, i64)> = form_versions.into_iter().collect();
    let form_ids: BTreeMap<&str, i64> = form_list
        .iter()
        .enumerate()
        .map(|(idx, (name, _))| (name.as_str(), (idx + 1) as i64))
        .collect();

    let mut bundle_stats: Vec<(i64, i64, i64)> = vec![(0, 0, 0); bundle_list.len()];
    let mut form_stats: Vec<(i64, i64)> = vec![(0, 0); form_list.len()];

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

    let mut asset_rows = Vec::new();
    let mut file_rows = Vec::new();

    if !plan.assets.is_empty() {
        for (idx, asset) in plan.assets.iter().enumerate() {
            let asset_id = (idx + 1) as i64;
            let bundle_id = bundle_ids[asset.bundle.as_str()];
            let form_id = form_ids[asset.metadata.form.as_str()];
            let owned_files = files_by_asset
                .get(asset.name.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let total_u64: u64 = owned_files.iter().map(|f| f.size).sum();
            let total_size = i64::try_from(total_u64).unwrap_or(i64::MAX);

            let b_slot = &mut bundle_stats[(bundle_id - 1) as usize];
            b_slot.0 += 1;
            b_slot.1 += owned_files.len() as i64;
            b_slot.2 = b_slot.2.saturating_add(total_size);

            if asset.metadata.error.is_none() {
                let f_slot = &mut form_stats[(form_id - 1) as usize];
                f_slot.0 += 1;
                f_slot.1 = f_slot.1.saturating_add(total_size);
            }

            for f in owned_files {
                let (dir, file_name) = split_dir_file(&f.path);
                file_rows.push(FileRow {
                    dir: dir.to_owned(),
                    file_name: file_name.to_owned(),
                    path: f.path.clone(),
                    bundle_id,
                    asset_id,
                    role: f.role.clone(),
                    sha256: f.key.sha256,
                    extension: f.key.extension.clone(),
                    size: i64::try_from(f.size).unwrap_or(i64::MAX),
                    media_type: f.media_type.clone(),
                });
            }

            let fp_bytes = parse_hex(&asset.metadata.fingerprint)
                .map(|arr| arr.to_vec())
                .unwrap_or_else(|| asset.metadata.fingerprint.as_bytes().to_vec());
            let (error_code, error_message) = match &asset.metadata.error {
                Some(err) => (Some(err.code.clone()), Some(err.message.clone())),
                None => (None, None),
            };
            let sources_json =
                serde_json::to_string(&asset.metadata.sources).unwrap_or_else(|_| "[]".to_owned());
            let resources_json = serde_json::to_string(&asset.metadata.resources)
                .unwrap_or_else(|_| "[]".to_owned());
            let references_json = serde_json::to_string(&asset.metadata.references)
                .unwrap_or_else(|_| "[]".to_owned());

            asset_rows.push(AssetRow {
                id: asset_id,
                bundle_id,
                name: asset.name.clone(),
                form_id,
                fingerprint: fp_bytes,
                total_size,
                error_code,
                error_message,
                sources_json,
                resources_json,
                references_json,
            });
        }
    } else {
        // Fallback for plans constructed without asset documents (such as materialized views).
        let mut names = BTreeSet::new();
        for f in &member_files {
            names.insert(f.asset.as_str());
        }
        for fail in &plan.failures {
            names.insert(fail.name.as_str());
        }
        let failure_map: BTreeMap<&str, &str> = plan
            .failures
            .iter()
            .map(|f| (f.name.as_str(), f.code.as_str()))
            .collect();
        for (idx, name) in names.into_iter().enumerate() {
            let asset_id = (idx + 1) as i64;
            let owned = files_by_asset.get(name).map(Vec::as_slice).unwrap_or(&[]);
            let total_size = owned
                .iter()
                .map(|f| i64::try_from(f.size).unwrap_or(i64::MAX))
                .fold(0i64, i64::saturating_add);
            bundle_stats[0].0 += 1;
            bundle_stats[0].1 += owned.len() as i64;
            bundle_stats[0].2 = bundle_stats[0].2.saturating_add(total_size);
            for f in owned {
                let (dir, file_name) = split_dir_file(&f.path);
                file_rows.push(FileRow {
                    dir: dir.to_owned(),
                    file_name: file_name.to_owned(),
                    path: f.path.clone(),
                    bundle_id: 1,
                    asset_id,
                    role: f.role.clone(),
                    sha256: f.key.sha256,
                    extension: f.key.extension.clone(),
                    size: i64::try_from(f.size).unwrap_or(i64::MAX),
                    media_type: f.media_type.clone(),
                });
            }
            let error_code = failure_map.get(name).map(|c| (*c).to_owned());
            asset_rows.push(AssetRow {
                id: asset_id,
                bundle_id: 1,
                name: name.to_owned(),
                form_id: 1,
                fingerprint: Vec::new(),
                total_size,
                error_code,
                error_message: None,
                sources_json: "[]".to_owned(),
                resources_json: "[]".to_owned(),
                references_json: "[]".to_owned(),
            });
        }
    }

    // Build recursive directory summary rows keyed by (parent, name).
    let mut dir_totals: BTreeMap<(String, String), (String, i64, i64)> = BTreeMap::new();
    for f in &file_rows {
        if f.dir.is_empty() {
            continue;
        }
        let mut cursor = f.dir.as_str();
        loop {
            let (parent, name) = split_dir_file(cursor);
            let entry = dir_totals
                .entry((parent.to_owned(), name.to_owned()))
                .or_insert_with(|| (cursor.to_owned(), 0, 0));
            entry.1 += 1;
            entry.2 = entry.2.saturating_add(f.size);
            if parent.is_empty() {
                break;
            }
            cursor = parent;
        }
    }

    // Sort `files` by WITHOUT ROWID PK `(dir, file_name)`.
    file_rows.sort_by(|a, b| {
        a.dir
            .cmp(&b.dir)
            .then_with(|| a.file_name.cmp(&b.file_name))
    });

    // Assemble SQLite 3 B-trees on 4096-byte pages.
    let mut db = DbWriter::new();

    // 1. `meta` (WITHOUT ROWID, PK `key`)
    let mut meta_entries = vec![
        ("format", "sqlite-index".to_owned()),
        ("published_version", plan.version.clone()),
        ("revision", plan.revision.clone()),
        ("schema_version", SCHEMA_VERSION.to_string()),
    ];
    meta_entries.sort_by(|a, b| a.0.cmp(b.0));
    let meta_root = db.build_index_btree(
        &meta_entries
            .into_iter()
            .map(|(k, v)| encode_record(&[SqlValue::Text(k.to_owned()), SqlValue::Text(v)]))
            .collect::<Vec<_>>(),
    );

    // 2. `bundles` + `idx_bundles_name`
    let bundles_table_rows: Vec<(i64, Vec<u8>)> = bundle_list
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            let id = (idx + 1) as i64;
            let (ac, fc, ts) = bundle_stats[idx];
            (
                id,
                encode_record(&[
                    SqlValue::Null,
                    SqlValue::Text(name.clone()),
                    SqlValue::Int(ac),
                    SqlValue::Int(fc),
                    SqlValue::Int(ts),
                ]),
            )
        })
        .collect();
    let bundles_root = db.build_table_btree(&bundles_table_rows, None);
    let idx_bundles_name_root = db.build_index_btree(
        &bundle_list
            .iter()
            .enumerate()
            .map(|(idx, name)| {
                encode_record(&[
                    SqlValue::Text(name.clone()),
                    SqlValue::Int((idx + 1) as i64),
                ])
            })
            .collect::<Vec<_>>(),
    );

    // 3. `forms` + `idx_forms_name`
    let forms_table_rows: Vec<(i64, Vec<u8>)> = form_list
        .iter()
        .enumerate()
        .map(|(idx, (name, ver))| {
            let id = (idx + 1) as i64;
            let (ac, ts) = form_stats[idx];
            (
                id,
                encode_record(&[
                    SqlValue::Null,
                    SqlValue::Text(name.clone()),
                    SqlValue::Int(*ver),
                    SqlValue::Int(ac),
                    SqlValue::Int(ts),
                ]),
            )
        })
        .collect();
    let forms_root = db.build_table_btree(&forms_table_rows, None);
    let idx_forms_name_root = db.build_index_btree(
        &form_list
            .iter()
            .enumerate()
            .map(|(idx, (name, _))| {
                encode_record(&[
                    SqlValue::Text(name.clone()),
                    SqlValue::Int((idx + 1) as i64),
                ])
            })
            .collect::<Vec<_>>(),
    );

    // 4. `directories` (WITHOUT ROWID, PK `(parent, name)`)
    let directories_root = db.build_index_btree(
        &dir_totals
            .into_iter()
            .map(|((parent, name), (path, fc, ts))| {
                encode_record(&[
                    SqlValue::Text(parent),
                    SqlValue::Text(name),
                    SqlValue::Text(path),
                    SqlValue::Int(fc),
                    SqlValue::Int(ts),
                ])
            })
            .collect::<Vec<_>>(),
    );

    // 5. `assets` + secondary indexes (`idx_assets_name`, `idx_assets_bundle`, `idx_assets_form`, `idx_assets_error`)
    let assets_table_rows: Vec<(i64, Vec<u8>)> = asset_rows
        .iter()
        .map(|row| {
            (
                row.id,
                encode_record(&[
                    SqlValue::Null,
                    SqlValue::Int(row.bundle_id),
                    SqlValue::Text(row.name.clone()),
                    SqlValue::Int(row.form_id),
                    SqlValue::Blob(row.fingerprint.clone()),
                    SqlValue::Int(row.total_size),
                    row.error_code
                        .as_ref()
                        .map_or(SqlValue::Null, |s| SqlValue::Text(s.clone())),
                    row.error_message
                        .as_ref()
                        .map_or(SqlValue::Null, |s| SqlValue::Text(s.clone())),
                    SqlValue::Text(row.sources_json.clone()),
                    SqlValue::Text(row.resources_json.clone()),
                    SqlValue::Text(row.references_json.clone()),
                ]),
            )
        })
        .collect();
    let assets_root = db.build_table_btree(&assets_table_rows, None);

    // `asset_rows` are already sorted by `name`.
    let idx_assets_name_root = db.build_index_btree(
        &asset_rows
            .iter()
            .map(|row| encode_record(&[SqlValue::Text(row.name.clone()), SqlValue::Int(row.id)]))
            .collect::<Vec<_>>(),
    );

    let mut by_bundle: Vec<&AssetRow> = asset_rows.iter().collect();
    by_bundle.sort_by(|a, b| {
        a.bundle_id
            .cmp(&b.bundle_id)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    let idx_assets_bundle_root = db.build_index_btree(
        &by_bundle
            .into_iter()
            .map(|row| {
                encode_record(&[
                    SqlValue::Int(row.bundle_id),
                    SqlValue::Text(row.name.clone()),
                    SqlValue::Int(row.id),
                ])
            })
            .collect::<Vec<_>>(),
    );

    let mut by_form: Vec<&AssetRow> = asset_rows.iter().collect();
    by_form.sort_by(|a, b| {
        a.form_id
            .cmp(&b.form_id)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    let idx_assets_form_root = db.build_index_btree(
        &by_form
            .into_iter()
            .map(|row| {
                encode_record(&[
                    SqlValue::Int(row.form_id),
                    SqlValue::Text(row.name.clone()),
                    SqlValue::Int(row.id),
                ])
            })
            .collect::<Vec<_>>(),
    );

    let mut by_error: Vec<&AssetRow> = asset_rows.iter().collect();
    by_error.sort_by(|a, b| {
        a.error_code
            .cmp(&b.error_code)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    let idx_assets_error_root = db.build_index_btree(
        &by_error
            .into_iter()
            .map(|row| {
                encode_record(&[
                    row.error_code
                        .as_ref()
                        .map_or(SqlValue::Null, |s| SqlValue::Text(s.clone())),
                    SqlValue::Text(row.name.clone()),
                    SqlValue::Int(row.id),
                ])
            })
            .collect::<Vec<_>>(),
    );

    // 6. `files` (WITHOUT ROWID, PK `(dir, file_name)`) + `idx_files_path` + `idx_files_asset`
    let files_root = db.build_index_btree(
        &file_rows
            .iter()
            .map(|row| {
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
            })
            .collect::<Vec<_>>(),
    );

    let mut by_path: Vec<&FileRow> = file_rows.iter().collect();
    by_path.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.dir.cmp(&b.dir))
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    let idx_files_path_root = db.build_index_btree(
        &by_path
            .into_iter()
            .map(|row| {
                encode_record(&[
                    SqlValue::Text(row.path.clone()),
                    SqlValue::Text(row.dir.clone()),
                    SqlValue::Text(row.file_name.clone()),
                ])
            })
            .collect::<Vec<_>>(),
    );

    let mut by_asset: Vec<&FileRow> = file_rows.iter().collect();
    by_asset.sort_by(|a, b| {
        a.asset_id
            .cmp(&b.asset_id)
            .then_with(|| a.role.cmp(&b.role))
            .then_with(|| a.dir.cmp(&b.dir))
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    let idx_files_asset_root = db.build_index_btree(
        &by_asset
            .into_iter()
            .map(|row| {
                encode_record(&[
                    SqlValue::Int(row.asset_id),
                    SqlValue::Text(row.role.clone()),
                    SqlValue::Text(row.dir.clone()),
                    SqlValue::Text(row.file_name.clone()),
                ])
            })
            .collect::<Vec<_>>(),
    );

    let schema = vec![
        (
            "table",
            "meta",
            "meta",
            meta_root,
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID",
        ),
        (
            "table",
            "bundles",
            "bundles",
            bundles_root,
            "CREATE TABLE bundles (id INTEGER PRIMARY KEY, name TEXT NOT NULL, asset_count INTEGER NOT NULL, file_count INTEGER NOT NULL, total_size INTEGER NOT NULL)",
        ),
        (
            "index",
            "idx_bundles_name",
            "bundles",
            idx_bundles_name_root,
            "CREATE UNIQUE INDEX idx_bundles_name ON bundles (name)",
        ),
        (
            "table",
            "forms",
            "forms",
            forms_root,
            "CREATE TABLE forms (id INTEGER PRIMARY KEY, name TEXT NOT NULL, form_version INTEGER NOT NULL, asset_count INTEGER NOT NULL, total_size INTEGER NOT NULL)",
        ),
        (
            "index",
            "idx_forms_name",
            "forms",
            idx_forms_name_root,
            "CREATE UNIQUE INDEX idx_forms_name ON forms (name)",
        ),
        (
            "table",
            "directories",
            "directories",
            directories_root,
            "CREATE TABLE directories (parent TEXT NOT NULL, name TEXT NOT NULL, path TEXT NOT NULL, file_count INTEGER NOT NULL, total_size INTEGER NOT NULL, PRIMARY KEY (parent, name)) WITHOUT ROWID",
        ),
        (
            "table",
            "assets",
            "assets",
            assets_root,
            "CREATE TABLE assets (id INTEGER PRIMARY KEY, bundle_id INTEGER NOT NULL, name TEXT NOT NULL, form_id INTEGER NOT NULL, fingerprint BLOB NOT NULL, total_size INTEGER NOT NULL, error_code TEXT, error_message TEXT, sources_json TEXT NOT NULL, resources_json TEXT NOT NULL, references_json TEXT NOT NULL)",
        ),
        (
            "index",
            "idx_assets_name",
            "assets",
            idx_assets_name_root,
            "CREATE UNIQUE INDEX idx_assets_name ON assets (name)",
        ),
        (
            "index",
            "idx_assets_bundle",
            "assets",
            idx_assets_bundle_root,
            "CREATE INDEX idx_assets_bundle ON assets (bundle_id, name)",
        ),
        (
            "index",
            "idx_assets_form",
            "assets",
            idx_assets_form_root,
            "CREATE INDEX idx_assets_form ON assets (form_id, name)",
        ),
        (
            "index",
            "idx_assets_error",
            "assets",
            idx_assets_error_root,
            "CREATE INDEX idx_assets_error ON assets (error_code, name)",
        ),
        (
            "table",
            "files",
            "files",
            files_root,
            "CREATE TABLE files (dir TEXT NOT NULL, file_name TEXT NOT NULL, path TEXT NOT NULL, bundle_id INTEGER NOT NULL, asset_id INTEGER NOT NULL, role TEXT NOT NULL, sha256 BLOB NOT NULL, extension TEXT NOT NULL, size INTEGER NOT NULL, media_type TEXT NOT NULL, PRIMARY KEY (dir, file_name)) WITHOUT ROWID",
        ),
        (
            "index",
            "idx_files_path",
            "files",
            idx_files_path_root,
            "CREATE UNIQUE INDEX idx_files_path ON files (path)",
        ),
        (
            "index",
            "idx_files_asset",
            "files",
            idx_files_asset_root,
            "CREATE INDEX idx_files_asset ON files (asset_id, role)",
        ),
    ];

    let sqlite_bytes = db.finalize(&schema);
    let key = store.put(&sqlite_bytes, "sqlite").map_err(&wrap_doc)?;
    Ok(Object {
        key,
        size: sqlite_bytes.len() as u64,
        media_type: "application/vnd.sqlite3".to_owned(),
    })
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
        let mut content = PAGE_SIZE;
        let mut ptrs = Vec::with_capacity(cells.len());
        for cell in cells {
            content -= cell.len();
            buf[content..content + cell.len()].copy_from_slice(cell);
            ptrs.push(u16::try_from(content).expect("page size is 4096"));
        }
        debug_assert!(header_offset + hdr_len + 2 * cells.len() <= content);

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

