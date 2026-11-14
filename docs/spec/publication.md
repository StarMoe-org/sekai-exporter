# Publication documents

[中文](publication.zh-CN.md)

These documents let a reader find every file of a version from one small pointer, and know what each file is. The protocol that writes them is described in [Publishing](../publishing.md).

## 1. Runtime

Programs that read published output: static sites, viewers and the resolver that maps a name to an object.

## 2. Contents

A reader starts at `<region>/<view>/current.json`, follows `root` to the index root, finds the chunk of the directories table that holds a directory, and follows the shard of the directory to its entries. Index roots, table chunks and shards are objects in the store, named by their SHA-256.

The **pointer** has the keys `format` (`"pointer"`), `version`, `published_version` and `revision`, and then, each when the layout has it, `root` (the lowercase SHA-256 of the index root), `sqlite` (of `store/<aa>/<sha256>.sqlite`) and `paths` (of the root of the [path index](path-index.md), which every layout has). The pointer of the current version is at `<region>/<view>/current.json`, and the pointer of every published version, with the same contents, at `<region>/<view>/versions/<published_version>.json`. A published version is named by ASCII letters, digits, `.`, `_` and `-`, and not only dots.

An **`index.sqlite` database** (`store/<aa>/<sha256>.sqlite`) is an immutable, content-addressed SQLite 3 database (`page_size = 4096`, UTF-8, schema format 4, `user_version = 2`) built bottom-up with no freelist pages so static HTTP `Range` (`206 Partial Content`) clients (`sql.js-httpvfs`, `wa-sqlite`) resolve directory trees, exact paths, bundle/form filters and full asset provenance in 1–3 page reads:

- `meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID` — `format` (`"sqlite-index"`), `published_version`, `revision`, `schema_version` (`"2"`).
- `bundles (id INTEGER PRIMARY KEY, name TEXT NOT NULL, asset_count INTEGER NOT NULL, file_count INTEGER NOT NULL, total_size INTEGER NOT NULL)` (`idx_bundles_name`).
- `forms (id INTEGER PRIMARY KEY, name TEXT NOT NULL, form_version INTEGER NOT NULL, asset_count INTEGER NOT NULL, total_size INTEGER NOT NULL)` (`idx_forms_name`).
- `directories (parent TEXT NOT NULL, name TEXT NOT NULL, path TEXT NOT NULL, file_count INTEGER NOT NULL, total_size INTEGER NOT NULL, PRIMARY KEY (parent, name)) WITHOUT ROWID` — recursive file counts and byte totals per directory subtree.
- `assets (id INTEGER PRIMARY KEY, bundle_id INTEGER NOT NULL, name TEXT NOT NULL, form_id INTEGER NOT NULL, fingerprint BLOB NOT NULL, total_size INTEGER NOT NULL, error_code TEXT, error_message TEXT, sources_json TEXT NOT NULL, resources_json TEXT NOT NULL, references_json TEXT NOT NULL)` (`idx_assets_name`, `idx_assets_bundle`, `idx_assets_form`, `idx_assets_error`).
- `files (dir TEXT NOT NULL, file_name TEXT NOT NULL, path TEXT NOT NULL, bundle_id INTEGER NOT NULL, asset_id INTEGER NOT NULL, role TEXT NOT NULL, sha256 BLOB NOT NULL, extension TEXT NOT NULL, size INTEGER NOT NULL, media_type TEXT NOT NULL, PRIMARY KEY (dir, file_name)) WITHOUT ROWID` (`idx_files_path`, `idx_files_asset`). Every file is the object `store/<aa>/<sha256>.<extension>`.

The **index root** has the keys `format` (`"index-root"`), `version`, `published_version`, `revision`, `directories`, `listings` and `failures`. Each of them but `listings` is a table, and `listings` lists `{ "form", "chunks" }` in form order, where `chunks` is the table of the listing of that form:

| Table | Rows, in key order | Key |
|---|---|---|
| `directories` | `{ "directory", "sha256" }`: the shard of each directory | `directory` |
| listing of a form | `{ "name", "fingerprint", "size" }` for each asset of the form; `size` is the total size of the members | `name` |
| `failures` | `{ "name", "code" }` for each asset that could not be produced | `name` |

A **table** lists its chunks in key order as `{ "first", "sha256", "rows" }`: the key of the first row, the SHA-256 of the chunk object and its number of rows. A **chunk** has the keys `format` (`"index-chunk"`), `version` and `rows`. A chunk ends after a row whose key, hashed with SHA-256 and read as a big-endian 32-bit number from its first four bytes, is a multiple of 512, and after its 2048th row otherwise; the last chunk ends with the last row. A change to a row therefore changes only the chunk that holds it, unless the change moves a boundary. A table without rows has no chunks, and a form without assets is not listed.

A **shard** has the keys `format` (`"index-shard"`), `version`, `directory` and `entries`, with one entry per file in name order:

| Key | Value |
|---|---|
| `name` | The file name within the directory |
| `sha256`, `extension` | The name of the object in the store |
| `size` | The size in bytes |
| `media_type` | The media type |
| `asset` | The name of the asset that owns the file |
| `role` | The role of the file in the asset. An asset document has the role `document` |

A **tree listing**, `_listing.json` at the root of a `tree` layout, has the keys `format` (`"tree-listing"`), `version`, `published_version` and `entries`, with `{ "path", "sha256", "asset" }` per file in path order.

The path of an asset's files follows from its name: `<name>.asset.json` for the document, `<name>.<extension>` for an asset of one member, and `<name>/<role>.<extension>` for each member of an asset of several.

## 3. Sources

The documents are written from the asset documents of a version and the names assigned to it.

## 4. Container

UTF-8 JSON without insignificant whitespace, with keys in the order given above.

## 5. Correctness

The revision is the lowercase hexadecimal SHA-256 of the text `publication 3\nversion <published_version>\n`, followed by `paths <sha256>\n` for each chunk of the path index and, in a layout with an index root, `failures <sha256>\n` for each chunk of the failures table, in order. The chunks of both depend only on the files and failures they hold, so the revision does not depend on the order in which assets were evaluated, nor on whether the publication was written whole or as a change to the previous one.

A publication may be written as a change to the previous publication of its view: the files, listing entries and failures of the assets that were removed or whose documents changed are taken out, and those of the new documents put in. Only the shards and chunks the change falls in are read and written, and the result is the publication the same assets give when written whole. A change that would put a file at a path another asset holds fails, as a whole publication with two files at one path does. Tests publish the same assets whole and as a change from different earlier assets, and compare the pointers.

## 6. Version

Format version: 3. A change to any of the above changes the version.

| Version | Change |
|---|---|
| 2 | Shards, listings, failures and the path index in every layout |
| 3 | The directories, listings and failures of the index root are tables of chunks; the revision is made from the chunks |
