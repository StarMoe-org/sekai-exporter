# Path index

[中文](path-index.zh-CN.md)

A path index maps every path of a view to the object that holds the file. Module: `paths` in `sekai-format`. `sekai-publish` writes one for every publication of a view, and `sekai-serve` reads it (see [Serving](../serving.md)). Format version: 1.

## 1. Runtime

Resolvers that answer a request for a path with the object of the store, such as `sekai-exporter serve` and edge workers in front of a bucket.

## 2. Contents

The **root** has the keys `format` (`"path-index"`), `version`, `view`, `published_version`, `revision`, `files` (the number of paths) and `chunks`. `chunks` lists `{ "first", "sha256", "files" }` per chunk in path order: the first path of the chunk, the lowercase SHA-256 of the chunk object, and its number of paths.

A **chunk** holds a contiguous range of the paths in byte order. Each path has:

| Field | Value |
|---|---|
| path | The path in the view, UTF-8, without a leading `/` |
| SHA-256 and extension | The name of the object in the store, `<aa>/<sha256>.<extension>` |
| size | The size in bytes |
| media type | The media type |

The root is stored as `<aa>/<sha256>.json` and each chunk as `<aa>/<sha256>.paths`. The pointer of the view names the root under `paths` (see [Publication documents](publication.md)).

## 3. Sources

The paths, objects, sizes and media types are the files of the publication of the view, in byte order of their paths. A chunk ends after a path whose SHA-256, read as a big-endian 32-bit number from its first four bytes, is a multiple of 2048, and after its 8192nd path otherwise; the last chunk ends with the last path. A chunk therefore holds 2048 paths on average, and a change to a path changes only the chunk that holds it, unless the change moves a boundary.

## 4. Container

The root is UTF-8 JSON without insignificant whitespace, with keys in the order above.

A chunk is binary. Integers marked *varint* are unsigned LEB128.

| Part | Encoding |
|---|---|
| Magic | The four bytes `SKPI` |
| Version | One byte, `1` |
| Extensions | A varint count, then each extension as a varint byte length and its UTF-8 bytes, in order of first use |
| Media types | The same, for media types |
| Paths | A varint count, then for each path in order: a varint length of the prefix it shares with the previous path (0 for the first), a varint length of the rest and its bytes, the 32 bytes of the SHA-256, a varint size, a varint index into the extensions and a varint index into the media types |

## 5. Correctness

A reader rejects a chunk whose magic or version differ, whose counts or lengths run past its end, whose indexes are out of range, whose paths are not valid UTF-8 or not in strictly increasing byte order, or that has bytes after its last path. A reader also rejects a root whose chunks are not in strictly increasing order of their first path.

Tests build indexes from known plans and check that every path resolves to its file, that paths before the first, between files and after the last do not, that an unchanged range keeps its chunks when other paths change, and that a chunk decodes to the files it was encoded from.

## 6. Version

Format version: 1. A change to any of the above changes the version.
