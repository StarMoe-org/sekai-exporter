# Version history

[中文](version-history.zh-CN.md)

A version history lists the published versions of a view, newest first, with the number of files each one has and how many it added, removed and changed. The changes of a version list those files by path. Module: `version_history` in `sekai-format`. `sekai-publish` writes both with every publication of a view, and `sekai-serve` sends them under [routes to the documents of a publication](../serving.md#version-histories). Format version: 1.

## 1. Runtime

Pages and tools that follow a view across versions, such as a changelog of the files of a region: one request gives every version with its counts, and a second gives the files a chosen version changed. A file is then read by its path in that version or the one before, `/{region}@{version}/{path}` under the routes of the [resolver](../serving.md).

## 2. Contents

The **version history** has the keys `format` (`"version-history"`), `version`, `view` and `entries`, one entry for each published version of the view, newest first:

| Key | Value |
|---|---|
| `published_version` | The name of the published version |
| `revision` | The revision of its last publication |
| `published_at` | When that publication was made, in UTC as RFC 3339, such as `2025-01-02T03:04:05Z`. Left out when it is not known |
| `files` | The number of files of the version |
| `added`, `removed`, `changed` | How many files the version added, removed and changed since the version before it. Left out, all three, when that is not known |

The **version changes** of a version have the keys `format` (`"version-changes"`), `version`, `view`, `from` (the published version before), `to` (this version), and then, each in path order:

| Key | Rows |
|---|---|
| `added` | `{ "path", "size", "media_type" }` for each file at a path that `from` had no file at |
| `removed` | `{ "path", "size" }` for each file of `from`, with its size there, at a path that `to` has no file at |
| `changed` | `{ "path", "size", "previous_size" }` for each file whose bytes differ from those at the same path in `from` |

A path whose file has the same bytes in both versions is in none of them. The counts of the entry of a version are the lengths of these lists, so a reader knows how large the changes are before it asks for them.

## 3. Sources

The history of a publication is the history of the previous publication of the view with an entry for its own version: put first when the version is new, and in place of the newest entry when the version is published again under its name. A publication with no previous publication starts a history with its own entry, which has no counts. When the previous publication has no history, the new one starts from an entry for that publication, with its number of files and without a time or counts. A version that changes none of the files, listings and failures of the previous publication of the view adds no entry: the view keeps that publication, its pointer, history and changes, under the name of the version it was published as.

The changes of a new version are those of the paths that the publication changed in the previous one, as described in [Publication documents](publication.md#5-correctness). The changes of a version published again stay the changes since the version before it: those the previous publication recorded, with each path that the new publication changed compared again with the file the version before has there. A publication of the same version with the same revision as the previous one keeps its history and changes.

The counts and the changes are not known, and are left out, when the publication is written whole while its view already has a publication, when the version is published again and the previous publication recorded no changes, and when the pointer or the path index of the version before is missing. The first publication of a view has no changes.

## 4. Container

Both documents are UTF-8 JSON without insignificant whitespace, with keys in the order above, compressed with gzip. They are stored as `<aa>/<sha256>.json.gz` with the media type `application/gzip`, and the pointer of the publication names them under `history` and `changes` with their SHA-256 and size (see [Publication documents](publication.md)). A page decompresses them as it does a [directory tree](directory-tree.md#4-container).

## 5. Correctness

A reader rejects a document of another format or version.

Tests publish versions that add, remove and change files and rewrite a file with the same bytes, and check the entries, the counts and the rows. They publish a version again with files that return to the bytes of the version before, change once more or are new, and check that its changes are still those since the version before, and that without the pointer of that version they are left out. They check that publishing the same files again keeps the documents, that a new version whose files have the same bytes, written again or not written at all, keeps the publication before it, that a view published anew goes on with its history, and that a publication without a history starts one from its previous publication.

## 6. Version

Format version: 1. A change to any of the above changes the version.
