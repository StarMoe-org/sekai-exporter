# Asset documents

[中文](asset-documents.zh-CN.md)

An asset document says what one asset is: its form, the objects it was made from, the files it consists of and the role of each file. Anyone who has the document and the store can use the asset without the catalog.

## 1. Runtime

Programs that read the published output: viewers, conversion tools and the catalog, which is an index of these documents and can be rebuilt from them.

## 2. Contents

One document is written per asset, at `<name>.asset.json`, and is itself an object in the store.

```json
{
  "format": "asset",
  "version": 1,
  "name": "card/res001_no001/card_normal",
  "form": "texture",
  "form_version": 1,
  "fingerprint": "…",
  "sources": [
    { "bundle": "character/member/res001_no001", "path_id": "-8500090008082076930", "class": "Texture2D" }
  ],
  "members": [
    { "role": "image", "sha256": "…", "size": 482113, "extension": "png", "media_type": "image/png" }
  ],
  "references": []
}
```

| Key | Value |
|---|---|
| `format` | `"asset"` |
| `version` | The version of this document format |
| `name` | The name of the asset: its logical path and any suffix assigned by the [naming rules](../architecture.md#identity) |
| `form`, `form_version` | The asset form and its version. Each form has a specification in this directory |
| `fingerprint` | The [fingerprint](../pipeline.md#invariants) of the asset, as lowercase hexadecimal |
| `sources` | The objects the conversion read: the root first, then the others ordered by bundle and path ID. `path_id` is a decimal string, as in [object documents](object-documents.md) |
| `resources` | Present only when the conversion read resource files: `{ "bundle": "…", "file": "…" }`, ordered by bundle and file |
| `members` | The files of the asset, in the order the form defines. `role` is defined by the form and is unique within the asset. `sha256` names the file in the store. `size` is in bytes |
| `references` | The names of other assets this asset refers to, in ascending order |
| `error` | Present only when the asset could not be produced: `{ "code": "…", "message": "…" }`. `members` is then empty |

Each member lists the canonical file of the member. Files that a view encodes differently are listed in the index of that view.

## 3. Sources

The evaluator writes the document from the result of the converter, the index of the version and the names assigned to the version. Nothing in it comes from configuration.

## 4. Container

UTF-8 JSON without insignificant whitespace, with keys in the order of the table above. `error` is omitted when absent.

## 5. Correctness

The document is a function of the fingerprint inputs and the names of the version, so equal inputs give equal bytes. Continuous integration pins the bytes of a document built from fixed values, and checks that a document reads back to the same values.

## 6. Version

Format version: 1. A change to any of the above changes the version.
