# Publishing

[中文](publishing.zh-CN.md)

Publishing gives names to the results of the [derivation graph](architecture.md) and places them where consumers read them. The core produces content-addressed results; how they are laid out and where they are stored is a separate, replaceable layer.

## One store

There is one **store**: a content-addressed place for bytes, either a local directory or an S3-compatible prefix. Results are written to it once.

- Objects in the store are named `<aa>/<sha256>.<extension>`. The name carries no meaning, in the same way as objects in a Git repository. Meaning is carried by the indexes and asset documents that refer to the object.
- A local copy of the store is a bounded cache. Incremental decisions, cache hits and publication differences need only hashes, which the catalog holds. Bytes are read again only when a view must produce different bytes, or when a new destination is published for the first time.

## Semantics travel with the output

- **Asset documents.** Each asset is published with a document, `<logical path>.asset.json`, which is itself an object in the store. It records the [asset form and its version](spec/asset-documents.md), the source objects as `(bundle, path ID, class)`, the member files and the role of each one (for example the base colour texture of a material, or the third cue of an audio object), and the other assets it refers to.
- **Index entries carry semantics.** Each entry of an index shard gives the name, hash, size, media type, owning asset and role of a file, not only a name and a hash.
- **Object documents** (`<bundle>/_objects.json`) give each object's class, fields and resolved references. The `objects` subcommand writes them, at `<output>/<bundle>/_objects.json` for each bundle file directly inside its input directory. `update` and `export` do not write them.

Anyone who has the store, the indexes and the asset documents can therefore tell what each file is, where it came from and how it relates to other files. The catalog is an index of this information and can be rebuilt from it.

The documents of a publication are specified in [Publication documents](spec/publication.md).

## Layouts

| Layout | Shape | Duplication | How a consumer reads it |
|---|---|---|---|
| `paths` | The store itself, plus a [path index](spec/path-index.md) per region and view: chunks of paths in byte order with the hash, size and type of each file | None | Through a resolver, such as `sekai-exporter serve` ([Serving](serving.md)), that maps a path to its object |
| `index` | The `paths` layout, plus small JSON shards by directory, mapping each name to its hash, size, type, owning asset and role, and the listings | None | Look up the index, then fetch the object. A small TypeScript resolver with caching is provided outside the Rust workspace |
| `sqlite` | The `index` layout, plus an `index.sqlite` database of the directories, assets and files | None | Query the database with HTTP range requests, then fetch the object |
| `tree` | A path tree per region and view | Bytes that appear in several regions are stored once per tree, unless the backend supports links | Fetch by path |

The `paths`, `index` and `sqlite` layouts reference the store directly and copy nothing. A `tree` layout uses hard links on a local file system and on backends that support them; on plain S3 it uses server-side copies.

For each asset form there is also a **listing**: logical path, asset form, fingerprint and size of every asset of that form in a region and version. Consumers use it instead of indexes built for one viewer.

## The storage view

The `storage` view reproduces the path tree served by the asset storage that the Moesekai site and other readers use, with the same paths, file names and encoding parameters. Assets that share a logical path use the names defined in the [architecture](architecture.md). It is published like the canonical view, with a path index and a pointer at `<region>/storage`, so a [resolver](serving.md) serves it at fixed paths and switches every path to a new version at once. `update --storage <dir>` also materializes it as a `tree`, which cannot be switched atomically. A tree is therefore written in this order:

1. every new file is written;
2. the listing of the new version is written;
3. files that the new version no longer contains are deleted, one publication cycle later, so that readers of the previous version can finish.

The consistency this gives is bounded as follows. Each file is replaced atomically. While a version is being written, a reader can see files of the previous and the new version side by side. Running the publication again after an interruption converges to the new version. A file whose asset has an error keeps its previous bytes until a conversion succeeds.

## The publication protocol

1. A version is published only when its run completed without a run failure: a failed download, a failed write or a crash. Asset errors do not stop publication; they are listed with the version.
2. Index shards are immutable and named by their SHA-256.
3. The revision of a publication is the hash of its sorted records, so it does not depend on how the JSON is formatted.
4. Before the pointer of a version is advanced, every object the publication refers to is checked to exist in the store under its recorded hash. Objects are written under a temporary name and renamed, so an object that exists under its name is complete.
5. The pointer is written to `<region>/<view>/versions/<published version>.json`, and then made current at `<region>/<view>/current.json`, so every published version stays reachable for as long as its objects are kept. A published version is named by ASCII letters, digits, `.`, `_` and `-`, and not only dots.
6. Failures are reported with the name of the stage that failed. A failed stage leaves the previous pointer in place.
7. When a publication goes to several destinations, the pointers of different destinations are not advanced atomically together. Each destination keeps its last complete pointer, and running the publication again completes the remaining destinations.

## S3-compatible buckets

With the `s3` feature, `sekai-exporter update` also publishes to an S3-compatible bucket when the configuration file has a `publish.s3` section:

```json
{
  "regions": { "...": {} },
  "publish": {
    "s3": {
      "endpoint": "https://s3.example.invalid/bucket",
      "region": "us-east-1",
      "prefix": "sekai/",
      "layout": "sqlite",
      "connections": 8,
      "secrets": { "access_key_id": "SEKAI_S3_ACCESS_KEY_ID", "secret_access_key": "SEKAI_S3_SECRET_ACCESS_KEY" }
    }
  }
}
```

| Key | Value |
|---|---|
| `endpoint` | The address of the bucket, path-style (`https://<host>/<bucket>`) or virtual-hosted (`https://<bucket>.<host>`) |
| `region` | The region the requests are signed for |
| `prefix` | Optional. Prepended to every object name; empty or ending in `/` |
| `layout` | Optional. The [layout](#layouts) of the canonical view: `"sqlite"`, the default, or `"index"`. The storage view is always uploaded in the `paths` layout |
| `connections` | Optional, 8 by default. Requests in flight at once |
| `secrets` | The names of the secrets that hold the access key ID and the secret access key. Each value is read from the environment variable of that name, or from the file of that name in the `--secrets` directory |

The bucket has the shape of the local output: each object of the store at `<prefix>store/<aa>/<sha256>.<extension>`, and the pointers of the canonical and the storage view at `<prefix>published/<region>/<view>/current.json` and `<prefix>published/<region>/<view>/versions/<published version>.json`. Immutable objects under `store/` are written with `Cache-Control: public, max-age=31536000, immutable` and pointers with `Cache-Control: no-cache`. Requests are signed with AWS Signature Version 4, and the key needs permission to read, list and write objects under the prefix.

A publication first reads the bucket's live `current.json` pointer. If it already names the publication, no object requests are sent. Otherwise the objects of the live publication that the local store describes are taken as present, and each other object is looked up with a `HEAD` request, or, when that would take more requests, in a listing of the store's 256 directories (`ListObjectsV2`, one directory per connection). `update --verify` trusts neither the pointer nor the local store: it lists the whole store, uploads every object the bucket lacks or holds with another size, and runs even when the version is unchanged. Each upload is signed with the SHA-256 of its bytes, which is also its name, so the bucket stores no bytes that do not match. The pointer is written after every object is in the bucket, and only when it changes. A failure stops the publication with the name of its stage and leaves the previous pointer in place; running it again uploads only what is still missing.

## Evaluation and publishing together

| Deployment | Evaluation | Place | Layout |
|---|---|---|---|
| Full static mirror | Eager | Batch run | `sqlite`, with a [resolver](serving.md) for paths |
| Self-hosted with limited storage | Bindings eager, conversions lazy | Server: a program using the crates as a library, since `serve` resolves published files only | `index`, with the store as an evictable cache |
| Serverless | Bindings eager, conversions lazy | A function running the WebAssembly core: look up the binding, fetch the bundle, convert, cache, return | `index` |
| A user with their own files | Lazy | Browser: the WebAssembly core unpacks and converts a dropped bundle locally, without a server | None |
| Another program | Any | Library | Chosen by the program |
