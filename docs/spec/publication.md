# Publication documents

[中文](publication.zh-CN.md)

These documents let a reader find every file of a version from one small pointer, and know what each file is. The protocol that writes them is described in [Publishing](../publishing.md).

## 1. Runtime

Programs that read published output: static sites, viewers and the resolver that maps a name to an object.

## 2. Contents

A reader starts at `<region>/<view>/current.json`, follows `root` to the index root, and follows the shard of a directory to the entries of that directory. Index roots, shards and listings are objects in the store, named by their SHA-256.

The **pointer** has the keys `format` (`"pointer"`), `version`, `published_version`, `revision` and `root`.

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
