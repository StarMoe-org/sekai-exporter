# Publication documents

[中文](publication.zh-CN.md)

These documents let a reader find every file of a version from one small pointer, and know what each file is. The protocol that writes them is described in [Publishing](../publishing.md).

## 1. Runtime

Programs that read published output: static sites, viewers and the resolver that maps a name to an object.

## 2. Contents

A reader starts at `<region>/<view>/current.json`, follows `root` to the index root, finds the chunk of the directories table that holds a directory, and follows the shard of the directory to its entries. Index roots, table chunks and shards are objects in the store, named by their SHA-256.

The **pointer** has the keys `format` (`"pointer"`), `version`, `published_version` and `revision`, and then, each when the layout has it, `root` (the lowercase SHA-256 of the index root), `paths` (of the root of the [path index](path-index.md), which every layout has), `tree` (`{ "sha256", "size" }` of the [directory tree](directory-tree.md), which every layout has), `history` (`{ "sha256", "size" }` of the [version history](version-history.md) of the view up to this publication, which every publication has) and `changes` (`{ "sha256", "size" }` of the [changes](version-history.md) of its version since the version before, when they are known). The pointer of the current version is at `<region>/<view>/current.json`, and the pointer of every published version, with the same contents, at `<region>/<view>/versions/<published_version>.json`. A published version is named by ASCII letters, digits, `.`, `_` and `-`, and not only dots.

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

The revision is the lowercase hexadecimal SHA-256 of the text `publication 4\nversion <published_version>\n`, followed by `paths <sha256>\n` for each chunk of the path index and, in a layout with an index root, `failures <sha256>\n` for each chunk of the failures table, in order. The chunks of both depend only on the files and failures they hold, so the revision does not depend on the order in which assets were evaluated, nor on whether the publication was written whole or as a change to the previous one.

A publication may be written as a change to the previous publication of its view: the files, listing entries and failures of the assets that were removed or whose documents changed are taken out, and those of the new documents put in. Only the shards and chunks the change falls in are read and written, and the result is the publication the same assets give when written whole. A change that would put a file at a path another asset holds fails, as a whole publication with two files at one path does. Tests publish the same assets whole and as a change from different earlier assets, and compare the pointers without `history` and `changes`, which follow the publications before and are not part of the revision.

## 6. Version

Format versions: 5 for the pointer, and 4 for the index root, shards, chunks and tree listing, whose version the revision names. A change to any of the above changes the version of the documents it concerns, so a publication written as a change to one of an earlier pointer format keeps its other documents.

| Version | Change |
|---|---|
| 2 | Shards, listings, failures and the path index in every layout |
| 3 | The directories, listings and failures of the index root are tables of chunks; the revision is made from the chunks |
| 4 | The pointer names the directory tree |
| 5 | The pointer alone: it names the version history and the changes of the version |
