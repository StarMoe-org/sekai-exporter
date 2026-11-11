# Publication documents

[中文](publication.zh-CN.md)

These documents let a reader find every file of a version from one small pointer, and know what each file is. The protocol that writes them is described in [Publishing](../publishing.md).

## 1. Runtime

Programs that read published output: static sites, viewers and the resolver that maps a name to an object.

## 2. Contents

A reader starts at `<region>/<view>/current.json`, follows `root` to the index root, and follows the shard of a directory to the entries of that directory. Index roots, shards and listings are objects in the store, named by their SHA-256.

The **pointer** has the keys `format` (`"pointer"`), `version`, `published_version`, `revision`, `root` and optionally `sqlite` (the lowercase SHA-256 of `store/<aa>/<sha256>.sqlite`).

An **`index.sqlite` database** (`store/<aa>/<sha256>.sqlite`) is an immutable, content-addressed SQLite 3 database (`page_size = 4096`, UTF-8, schema format 4, `user_version = 1`) built bottom-up with no freelist pages so static HTTP `Range` (`206 Partial Content`) clients (`sql.js-httpvfs`, `wa-sqlite`) resolve directory trees, exact paths, bundle/form filters and full asset provenance in 1–3 page reads:

- `meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID` — `format` (`"sqlite-index"`), `published_version`, `revision`, `schema_version` (`"1"`).
- `bundles (id INTEGER PRIMARY KEY, name TEXT NOT NULL, asset_count INTEGER NOT NULL, file_count INTEGER NOT NULL, total_size INTEGER NOT NULL)` (`idx_bundles_name`).
- `forms (id INTEGER PRIMARY KEY, name TEXT NOT NULL, form_version INTEGER NOT NULL, asset_count INTEGER NOT NULL, total_size INTEGER NOT NULL)` (`idx_forms_name`).
- `directories (parent TEXT NOT NULL, name TEXT NOT NULL, path TEXT NOT NULL, file_count INTEGER NOT NULL, total_size INTEGER NOT NULL, PRIMARY KEY (parent, name)) WITHOUT ROWID` — recursive file counts and byte totals per directory subtree.
- `assets (id INTEGER PRIMARY KEY, bundle_id INTEGER NOT NULL, name TEXT NOT NULL, form_id INTEGER NOT NULL, fingerprint BLOB NOT NULL, total_size INTEGER NOT NULL, error_code TEXT, error_message TEXT, sources_json TEXT NOT NULL, resources_json TEXT NOT NULL, references_json TEXT NOT NULL)` (`idx_assets_name`, `idx_assets_bundle`, `idx_assets_form`, `idx_assets_error`).
- `files (dir TEXT NOT NULL, file_name TEXT NOT NULL, path TEXT NOT NULL, bundle_id INTEGER NOT NULL, asset_id INTEGER NOT NULL, role TEXT NOT NULL, sha256 BLOB NOT NULL, extension TEXT NOT NULL, size INTEGER NOT NULL, media_type TEXT NOT NULL, archive_sha256 BLOB, archive_offset INTEGER, PRIMARY KEY (dir, file_name)) WITHOUT ROWID` (`idx_files_path`, `idx_files_asset`). Standalone browser-native files (`texture`, `sprite`, `audio`, `video`, `font`, `text`) keep `archive_sha256 IS NULL` and live directly at `store/<aa>/<sha256>.<extension>`. Multi-member composite assets in `pack_forms` (`live2d-model`, `live2d-motions`, `costume-model`, `fixture-model`, `site-model`, `emoticon`) are packed into deterministic uncompressed POSIX `ustar` archives at `store/<aa>/<archive_sha256>.tar`, with 512-byte-aligned payload byte coordinates `(archive_offset, size)` supporting both single-`GET` whole-model loads and single-`Range` member previews.

The **index root** has the keys `format` (`"index-root"`), `version`, `published_version`, `revision`, `shards`, `listings` and `failures`. `shards` lists `{ "directory", "sha256" }` in directory order, and `listings` lists `{ "form", "sha256" }` in form order. `failures` lists `{ "name", "code" }` for each asset that could not be produced.

A **shard** has the keys `format` (`"index-shard"`), `version`, `directory` and `entries`, with one entry per file in name order:

| Key | Value |
|---|---|
| `name` | The file name within the directory |
| `sha256`, `extension` | The name of the object in the store |
| `size` | The size in bytes |
| `media_type` | The media type |
| `asset` | The name of the asset that owns the file |
| `role` | The role of the file in the asset. An asset document has the role `document` |

A **listing** has the keys `format` (`"listing"`), `version`, `form` and `entries`, with `{ "name", "fingerprint", "size" }` per asset in name order. `size` is the total size of the members.

A **tree listing**, `_listing.json` at the root of a `tree` layout, has the keys `format` (`"tree-listing"`), `version`, `published_version` and `entries`, with `{ "path", "sha256", "asset" }` per file in path order.

The path of an asset's files follows from its name: `<name>.asset.json` for the document, `<name>.<extension>` for an asset of one member, and `<name>/<role>.<extension>` for each member of an asset of several.

## 3. Sources

The documents are written from the asset documents of a version and the names assigned to it.

## 4. Container

UTF-8 JSON without insignificant whitespace, with keys in the order given above.

## 5. Correctness

The revision is the SHA-256 of the sorted records of the publication: for each file its path, hash, extension, size, media type, owner and role, and for each failure its name and code. It does not depend on how the JSON is formatted or on the order in which assets were evaluated. Tests publish the same assets in different orders and compare the results.

## 6. Version

Format version: 1. A change to any of the above changes the version.
