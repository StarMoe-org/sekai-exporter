# Directory tree

[中文](directory-tree.zh-CN.md)

A directory tree holds every directory and file of a publication of a view in one document: the name, size and media type of each file, without the objects that hold them. Module: `directory_tree` in `sekai-format`. `sekai-publish` writes one for every publication of a view, and `sekai-serve` sends it under a [route to the tree](../serving.md#directory-trees). Format version: 1.

## 1. Runtime

Readers that browse a whole view at once, such as the file browser of a web page: one request gives the whole tree, and a file is then read by its path in the same published version, `/{region}@{version}/{path}` under the routes of the [resolver](../serving.md).

## 2. Contents

The document has the keys `format` (`"directory-tree"`), `version`, `view`, `published_version`, `revision`, `files` (the number of files), `media_types` and `root`.

`media_types` lists each media type of the files once, in order of first use, in byte order of the paths. Files name their media type by its position in this list.

A **directory** has the keys:

| Key | Value |
|---|---|
| `name` | The name of the directory, a segment of the paths below it. Empty for `root` |
| `directories` | The subdirectories, in byte order of their names. Left out when there are none |
| `files` | The files right under the directory. Left out when there are none |

`files` holds one column per field, each in byte order of the names:

| Key | Value |
|---|---|
| `names` | The file names |
| `sizes` | The sizes in bytes |
| `media_types` | The position of each media type in `media_types` |

The path of a file is the names of the directories from `root` down to it and its own name, joined by `/`. For example, the files `a.txt`, `a/c/d.json` and `a/z.png` give:

```json
{
  "name": "",
  "directories": [
    {
      "name": "a",
      "directories": [
        { "name": "c", "files": { "names": ["d.json"], "sizes": [2], "media_types": [1] } }
      ],
      "files": { "names": ["z.png"], "sizes": [3], "media_types": [2] }
    }
  ],
  "files": { "names": ["a.txt"], "sizes": [1], "media_types": [0] }
}
```

## 3. Sources

The paths, sizes and media types are those of the files of the publication, as its [path index](path-index.md) has them. A directory exists when a file is under it. The tree is a function of the files of the publication, its view, its published version and its revision: a publication written as a change to the previous one gives the same tree as one written whole.

## 4. Container

The document is UTF-8 JSON without insignificant whitespace, with keys in the order above, compressed with gzip. It is stored as `<aa>/<sha256>.json.gz` with the media type `application/gzip`, and the pointer of the publication names it under `tree` with its SHA-256 and size (see [Publication documents](publication.md)).

A page decompresses it with the `DecompressionStream` of the browser:

```js
const response = await fetch("/trees/jp/storage");
const tree = await new Response(response.body.pipeThrough(new DecompressionStream("gzip"))).json();
```

## 5. Correctness

A reader rejects a tree of another format or version, one whose root has a name, a directory or file name that is empty or contains `/`, columns of a directory that differ in length, a media type position outside `media_types`, a path listed twice, and a count of files that differs from the files it lists.

Tests build trees from known files and check the document, that the files read back are the files it was built from, and that files out of order, listed twice or with an empty segment make no tree. Tests of publishing check that the tree of a change equals the tree of the whole publication, whether the previous publication has a tree or only a path index.

## 6. Version

Format version: 1. A change to any of the above changes the version.
