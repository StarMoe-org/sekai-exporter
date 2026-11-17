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
| `index` | The `paths` layout, plus small JSON shards by directory, mapping each name to its hash, size, type, owning asset and role, and an index root whose directories, listings and failures are tables cut into chunks | None | Look up the index, then fetch the object. A small TypeScript resolver with caching is provided outside the Rust workspace |
| `tree` | A path tree per region and view | Bytes that appear in several regions are stored once per tree, unless the backend supports links | Fetch by path |

The `paths` and `index` layouts reference the store directly and copy nothing. A `tree` layout uses hard links on a local file system and on backends that support them; on plain S3 it uses server-side copies.

For each asset form there is also a **listing**: logical path, asset form, fingerprint and size of every asset of that form in a region and version. Consumers use it instead of indexes built for one viewer.

## The storage view

The `storage` view reproduces the path tree served by the asset storage that the Moesekai site and other readers use, with the same paths, file names and encoding parameters. Assets that share a logical path use the names defined in the [architecture](architecture.md). It is published like the canonical view, with a path index and a pointer at `<region>/storage`, so a [resolver](serving.md) serves it at fixed paths and switches every path to a new version at once.

A `tree`, such as the one `export --tree` writes, cannot be switched atomically. A tree is therefore written in this order:

1. every new file is written;
2. the listing of the new version is written;
3. files that the new version no longer contains are deleted, one publication cycle later, so that readers of the previous version can finish.

The consistency this gives is bounded as follows. Each file is replaced atomically. While a version is being written, a reader can see files of the previous and the new version side by side. Running the publication again after an interruption converges to the new version. A file whose asset has an error keeps its previous bytes until a conversion succeeds.

## The client view

The `client` view holds the assets of a client package, which the client loads from the package instead of downloading them: from its player data, images, fonts and the prefabs of the interface it shows around the scenes of a story; from its streaming assets, cue sheets such as the sounds of its menus and the music of its title screen. `update --client` publishes it in the canonical layout with a pointer at `<region>/client`, under the version the [package](sources.md#region-configuration) names, and keeps its state under `<region>-client`, apart from that of the region's assets.

## The publication protocol

1. A version is published only when its run completed without a run failure: a failed download, a failed write or a crash. Asset errors do not stop publication; they are listed with the version.
2. Index shards are immutable and named by their SHA-256.
3. The revision of a publication is made from the chunks of its path index and failures, which depend only on what they hold. A publication may be written as a change to the previous one, which reads and writes only the shards and chunks the change falls in and gives the same documents as writing it whole.
4. Before the pointer of a version is advanced, every object the publication refers to is checked to exist in the store under its recorded hash. Objects are written under a temporary name and renamed, so an object that exists under its name is complete.
5. The pointer is written to `<region>/<view>/versions/<published version>.json`, and then made current at `<region>/<view>/current.json`, so every published version stays reachable for as long as its objects are kept. The pointer also names the [version history](spec/version-history.md) of the view and the changes of the version since the version before, so a reader lists the published versions without listing the store. A published version is named by ASCII letters, digits, `.`, `_` and `-`, and not only dots.
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
      "layout": "index",
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
| `layout` | Optional. The [layout](#layouts) of the canonical view: `"index"`, the default. The storage view is always uploaded in the `paths` layout |
| `connections` | Optional, 8 by default. Requests in flight at once, and the connections kept open between them |
| `secrets` | The names of the secrets that hold the access key ID and the secret access key. Each value is read from the environment variable of that name, or from the file of that name in the `--secrets` directory |

The bucket has the shape of the local output: each object of the store at `<prefix>store/<aa>/<sha256>.<extension>`, and the pointers of the canonical and the storage view at `<prefix>published/<region>/<view>/current.json` and `<prefix>published/<region>/<view>/versions/<published version>.json`. Immutable objects under `store/` are written with `Cache-Control: public, max-age=31536000, immutable` and pointers with `Cache-Control: no-cache`. Requests are signed with AWS Signature Version 4, and the key needs permission to read, list and write objects under the prefix.

A run knows which objects the bucket holds from the records it reads and the uploads the bucket accepts; it does not ask the bucket object by object. It uploads each object of the version that is in the local store and not known to be held. An object that is neither in the local store nor known to be held is taken to be in the bucket, since every object is put there before any record or pointer that names it. The first time a run looks for a record, it lists the record packs and reads them all. `update --verify` first lists the whole store (`ListObjectsV2`, one of its 256 directories per connection), uploads every object of the version the bucket lacks, stops with the stage `verify` when the bucket holds an object of the version with another size or lacks one that the local store lacks too, and runs even when the version is unchanged. Each upload is signed with the SHA-256 of its bytes, which is also its name, so the bucket stores no bytes that do not match. The pointer is written after every object is in the bucket, and only when it changes. A failure stops the publication with the name of its stage and leaves the previous pointer in place; running it again uploads only what is still missing and converts again only the assets whose derivations were not recorded.

## The state of a region

`update` keeps the state of each region at its site: under `<prefix>state/` in the bucket, or under `<output>/state` without one. A run reads only this state and the pointers of the views, so a machine with an empty `<output>` continues where the last run stopped.

| File | Content |
|---|---|
| `versions/<region>.json` | The identity of the last version, with the identity of the evaluation that gave its assets under `evaluation`, by part (`unit`, `form <name>`, `table <name>`). `update` and `update --check` compare with it, and a record without `evaluation` counts as evaluated differently |
| `regions/<region>.json` | The root of the state of the region, which names its chunks |
| `chunks/<aa>/<sha256>.json.gz` | The chunks of the states of every region: the bundles of the last version with their content and the files they hold, its units with their keys, the names its assets claim and which unit owns each, the assets built from tables with what each read, and the revision of each view's publication |
| `records/packs/<sha256>.json.gz` | Record packs, which records of every region go to: what a unit gave, under the key of its bundles' content and the evaluation, for any region whose unit has the same key, and the object a derivation made, with its size |

A record pack is gzipped JSON lines: the header `{"format":"record-pack","version":1}`, then one record per line, `{"unit":"<key>","record":<unit record>}` or `{"derivation":"<fingerprint>","record":<derivation record>}`, with keys and fingerprints in lowercase hexadecimal. A derivation record is `{"document":{"key":"<sha256>.<extension>","size":<bytes>}}`, or `"nothing"` for a root that holds no asset of its form. The unit records come first, then the derivation records, each in the order of their keys, with one record of a subject; the lines after the header hold at most 8 MiB unless one record is larger. The same records give the same bytes, and a pack is never changed once written. When packs hold records of one subject, a run takes the one in the pack whose name comes first.

A unit record lists each asset of the unit with the logical path of its root and `path_id`, which is the path ID of the root object, or `{"file": "<path>"}` for an asset whose root is a file of the bundle; each root left unnamed is a pair of its logical path and the same value.

The state is split into content-addressed chunks, so a run writes only the chunks its changes fall in. While it evaluates, a run uploads the output of each asset, its asset document and the members it names, and records the derivation once they are at the site. It writes the records in a pack once they fill one or five minutes after its last pack, so a run that stops leaves the derivations of its packs to the next one. Then it writes the other new objects, such as those of the views, its last pack with the unit records, the state and then the pointers. A record is written only once the objects it names are at the site. When the revisions in the state are not those of the current pointers, the run publishes both views whole and keeps only the bundles of the state.

A version with the same content and evaluation is published again when the pointer of one of its views names documents of an earlier format, an index root, shards and chunks of a version below the current one (see [Publication documents](spec/publication.md#6-version)). A pointer of an earlier format that names documents of the current one is kept until the next version, which is written as a change to it.

## Collecting garbage

A run adds objects and state files and deletes none: objects that no publication refers to any more, chunks of earlier states and records of earlier evaluations stay at the site. `sekai-exporter gc <output> [--config <file>]` finds them, in the bucket of the configuration or else in `<output>`, and with `--delete` deletes them.

| Kept | Why |
|---|---|
| Every pointer under `published/` | Each names a published version, current or not |
| Every object a pointer refers to | Its index root, table chunks and shards, the root and chunks of its path index, its directory tree, version history and changes, and every file its path index lists |
| The root of the state of each region, the chunks it names, and the documents of the assets the region built from tables, with their members | A run keeps an asset built from tables without writing it again |
| Each record whose objects are all kept | A run takes the objects a record names to be at the site |
| Each file written within the grace period (`--grace`, `1d` by default), and the objects that the records among them name | A run that is publishing writes its objects and records before its pointers |

Every other object of the store, state chunk and record is garbage. Records are never changed in their packs, so `gc` rewrites a pack written before the grace period when it holds a record that is not kept or less than 2 MiB of records: the records kept of every such pack go to new packs, which `gc` reports with their count and size. A pack that the new packs give again unchanged stays. Records kept one file each, under `records/units/<aa>/<key>.json` and `records/derivations/<aa>/<fingerprint>.json`, are moved into packs in the same way. `gc` lists the garbage of each kind (record packs, single unit records, single derivation records, state chunks and objects, the objects also by extension) with counts and sizes, and writes and deletes nothing without `--delete`. With `--delete` the new packs are written first, then the record files they replace are deleted, then the state chunks and the objects, so an interrupted collection loses no record that is kept and leaves no record that names a deleted object. A run that took such a record would publish a pointer to an object the site lacks, so `gc` runs while no update runs on the site. In a bucket, `gc` reads the times of the listing (`LastModified`), and `--delete` needs permission to delete objects under the prefix.

`gc` also collects what runs leave in `<output>` itself, wherever the site is. A bundle of the [input store](sources.md#the-input-store) is garbage when no region fetched it last, as the catalog of `<output>` records, and a file in `incoming/` of the input store or of the store is a write that an interrupted run left partial. Both are garbage once written before the grace period, and `--delete` deletes them after the files of the site. Without a catalog that records a fetch, or when the catalog cannot be read, no bundle of the input store is garbage. A bundle that a later run needs again is downloaded again.

Pointers are never deleted, so every published version stays reachable. A version published again under the same name replaces the pointer of that version, and the objects that only the earlier publication referred to become garbage.

## Evaluation and publishing together

| Deployment | Evaluation | Place | Layout |
|---|---|---|---|
| Full static mirror | Eager | Batch run | `index`, with a [resolver](serving.md) for paths |
| Self-hosted with limited storage | Bindings eager, conversions lazy | Server: a program using the crates as a library, since `serve` resolves published files only | `index`, with the store as an evictable cache |
| Serverless | Bindings eager, conversions lazy | A function running the WebAssembly core: look up the binding, fetch the bundle, convert, cache, return | `index` |
| A user with their own files | Lazy | Browser: the WebAssembly core unpacks and converts a dropped bundle locally, without a server | None |
| Another program | Any | Library | Chosen by the program |
