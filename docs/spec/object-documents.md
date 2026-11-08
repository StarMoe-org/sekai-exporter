# Object documents

[中文](object-documents.zh-CN.md)

An object document lists every object of one bundle with all of its fields, in the order and with the names the file stores them. It is the neutral, lossless form of the object graph: anything that no converter exports yet can still be read here.

## 1. Runtime

Programs and people that need the objects themselves: browsing and debugging tools, converters written outside this repository, and checks that compare this exporter with other Unity readers. The document is not an asset form; it records storage, not meaning.

## 2. Contents

One document is written per bundle, at `<bundle>/_objects.json`. A bundle from the client package is treated as a bundle whose name is the player data file name.

```json
{
  "format": "objects",
  "version": 1,
  "bundle": "character/member/res001_no001",
  "entries": [
    { "path": "CAB-0123456789abcdef0123456789abcdef", "kind": "serialized", "size": 52344, "sha256": "…" },
    { "path": "CAB-0123456789abcdef0123456789abcdef.resS", "kind": "resource", "size": 1048576, "sha256": "…" }
  ],
  "files": [
    {
      "path": "CAB-0123456789abcdef0123456789abcdef",
      "serialized_version": 22,
      "unity_version": "2022.3.62f2",
      "platform": 13,
      "type_trees": true,
      "externals": [
        { "index": 1, "path": "archive:/CAB-…/CAB-…", "target": { "bundle": "shader/live", "file": "CAB-…" } },
        { "index": 2, "path": "Library/unity default resources", "target": null }
      ],
      "objects": [
        {
          "id": "-4407209310344673407",
          "class": "Texture2D",
          "class_id": 28,
          "name": "res001_no001",
          "container": ["assets/sekai/assetbundle/resources/…/res001_no001.png"],
          "size": 248,
          "sha256": "…",
          "fields": { "m_Name": "res001_no001", "m_Width": 1024, "…": "…" }
        }
      ]
    }
  ]
}
```

### 2.1 Bundle level

| Key | Value |
|---|---|
| `format`, `version` | `"objects"` and the version of this specification |
| `bundle` | The bundle name |
| `entries` | Every entry of the container in directory order: its path, its kind (`serialized` for a serialized file, `resource` for any other entry), its size and the SHA-256 of its bytes |
| `files` | One record per serialized file, in directory order |

### 2.2 File level

| Key | Value |
|---|---|
| `path` | The entry path of the serialized file |
| `serialized_version` | The format version in the file header |
| `unity_version`, `platform` | The engine version string and the target platform number, as stored. A stripped version string is written as stored |
| `type_trees` | Whether the file stores type trees |
| `externals` | The file's list of external files, with `index` starting at 1 as PPtrs count them. `target` names the bundle and file the external resolves to within the same version, or is `null` when it resolves to no bundle (engine built-in resources) |
| `objects` | Every object, in the order of the file's object table |

### 2.3 Object level

| Key | Value |
|---|---|
| `id` | The path ID, as a decimal string |
| `class` | The class name: the root type name of the object's layout, or the engine's name for the class ID when no layout is known |
| `class_id` | The class ID stored in the file |
| `script` | For objects that refer to a script through `m_Script`: `"<assembly>/<namespace>.<class>"`, read from the referenced `MonoScript`; the assembly name is written without its `.dll` suffix, and a class without a namespace is written `"<assembly>/<class>"`, which may be in another file. Absent otherwise. `null` when the reference does not resolve |
| `name` | The value of a top-level `m_Name` string field, when the object has one |
| `container` | The container paths of the bundle's `AssetBundle` object that point to this object, in container order. Absent when there are none |
| `size`, `sha256` | The size and SHA-256 of the object's stored bytes |
| `fields` | The field values (§2.4), or `null` when the object could not be read |
| `error` | Present only when the object could not be read exactly (§2.5) |

### 2.4 Field values

Fields are written by the object's layout: the type tree stored in the file, or a [layout table](../glossary.md) for files without type trees.

| Stored type | Written as |
|---|---|
| Structure | An object whose keys are the field names in layout order |
| `bool` | `true` for 1, `false` for 0, and the number for any other stored byte |
| 8-, 16- and 32-bit integers, `char` | A number |
| 64-bit integers | A decimal string, so that values beyond 2^53 survive JSON readers |
| `float`, `double` | The shortest decimal that reads back to the same value; `"NaN"`, `"Infinity"` and `"-Infinity"` as strings. Every NaN is written `"NaN"`, whatever its sign and payload bits; the object's `sha256` still covers them |
| `string` | A string when the bytes are valid UTF-8, a byte payload otherwise |
| Array | An array |
| `map` | An array of `[key, value]` pairs in stored order |
| Array of 8-bit integers, `TypelessData` | A byte payload |
| `PPtr<T>` | `null` for `(0, 0)`, a reference otherwise |

A **byte payload** is `{"$bytes": {"size": n, "sha256": "…", "base64": "…"}}`. `base64` is present when the payload is at most 4096 bytes. Larger payloads are published as store objects named by their hash, so the document stays lossless without carrying them inline.

A **reference** is `{"$ref": {…}}` with:

| Target | Keys |
|---|---|
| An object in the same file | `id` |
| An object in another file of the same bundle | `file`, `id` |
| An object in another bundle | `bundle`, `file`, `id` |
| An external that resolves to no bundle | `external` (the external path), `id` |

Unity field names never begin with `$`, so `$bytes` and `$ref` cannot collide with a field.

Objects that use `[SerializeReference]` store a `references` registry. Each entry's `data` is read with the layout of the type the entry names, taken from the file's table of referenced types, and written like any other structure. Fields that point into the registry keep their stored `rid` numbers.

### 2.5 Errors

An object that cannot be read exactly is still listed. Its `fields` is `null`, `error` is `{"code": "…", "message": "…"}`, and `raw` is a byte payload of the stored bytes.

| Code | Meaning |
|---|---|
| `no_layout` | The file has no type tree for the class and no layout table covers it |
| `truncated` | The layout reads past the end of the object |
| `trailing_bytes` | The layout ends before the end of the object |
| `invalid_layout` | The layout cannot be interpreted, for example an array without a size field |
| `malformed` | A stored value contradicts the file, for example a reference to an external file the file does not list |
| `unresolved_reference_type` | A `references` entry names a type that the file's table of referenced types does not contain |

A reader never substitutes a default value and never drops a field.

## 3. Sources

The bundle bytes, after deobfuscation, and the index of every bundle in the same version, which resolves externals and `m_Script` references.

## 4. Container

UTF-8 JSON without a byte order mark, keys in the order given above, no insignificant whitespace, and a final line feed. The same input always gives the same bytes.

## 5. Correctness

- Every object is read to exactly its end; any other outcome is an error of §2.5.
- Field values are compared with an independent Unity reader on bundles built by the Unity editor (included in this repository) and on any bundles a user supplies. The comparison maps both readers' output to the value model of §2.4 and requires equality for every field of every object.
- Documents are compared byte for byte across thread counts and with and without a block cache. The comparison between native and WebAssembly builds is planned.

## 6. Version

Version 1.
