# Serving

[中文](serving.zh-CN.md)

A publication names every file by its content: objects live at `store/<aa>/<sha256>.<extension>`, and their meaning is carried by indexes. Readers that address files by path, such as static sites, viewers and download tools, need a **resolver** that maps a path of a view to an object. `sekai-exporter serve` is such a resolver. Its core, the `sekai-serve` crate, does no I/O, so the same code can run in other hosts, such as an edge worker in front of an S3-compatible bucket.

The resolver replaces a materialized path tree. A tree copies or links every file to a fixed path, multiplies the number of stored objects by the number of views, and cannot switch to a new version atomically. A resolved tree stores nothing but its index, every view shares the same objects, and a new version takes effect for every path at once.

## Views and their path indexes

A **view** is a path tree over the objects of a publication:

| View | Paths |
|---|---|
| `canonical` | The paths of the [publication](spec/publication.md): `<name>.asset.json`, `<name>.<extension>`, or `<name>/<role>.<extension>` |
| `storage` | The paths of the [storage view](spec/storage-view.md) |

Each publication of a view writes a **path index** to the store and names it in the pointer of the view, `<region>/<view>/current.json`, under `paths`. The same pointer is also kept at `<region>/<view>/versions/<published version>.json`, so a version stays reachable after newer ones are published, for as long as its objects are kept.

A path index is a small root document and a sequence of **chunks**, each holding a contiguous range of paths in byte order with the hash, extension, size and media type of each file ([format](spec/path-index.md)). Chunk boundaries are chosen by the hash of a path, not by position, so a version that adds or changes a few paths changes only the chunks that hold them. Unchanged chunks keep their hashes, and publishing the next version uploads only the chunks that changed.

A lookup reads the root, finds the chunk whose range holds the path by binary search, and finds the path in that chunk by binary search. A host keeps decoded chunks in memory, so a lookup in a chunk it has already read reads nothing.

## Routes

A routes file says which URL paths name which views. Routes are tried in order, and the first that matches decides:

```json
{
  "routes": [
    { "pattern": "/jp-storage/{path}", "region": "jp", "view": "storage" },
    { "pattern": "/storage/{region}/{path}", "view": "storage" },
    { "pattern": "/{region}@{version}/{path}", "view": "canonical" },
    { "pattern": "/{region}/{path}", "view": "canonical" }
  ],
  "delivery": "bytes",
  "max_age": 60,
  "cors": "*"
}
```

A pattern starts with `/` and is a sequence of `/`-separated segments. A segment is literal text, placeholders, or both, such as `{region}@{version}`; two placeholders are separated by text. A placeholder takes the characters of its segment up to the first occurrence of the text that follows it, and they must form a name: ASCII letters, digits, `.`, `_` and `-`, and not only dots. A route whose placeholders do not take names does not match, and the next route is tried. The first route that matches decides even when its view has no such path, so routes with literal text come before the more general ones.

| Placeholder | Meaning |
|---|---|
| `{region}` | The region. Either a placeholder or the route's `region` key gives it, never both |
| `{view}` | The view. Either a placeholder or the route's `view` key gives it, never both |
| `{version}` | Optional. A published version: the path is resolved in that version instead of the current one |
| `{path}` | Required, and the last segment. The rest of the URL path, percent-decoded, is the path in the view |

A region, view or version without a publication is not found.

| Key | Meaning |
|---|---|
| `delivery` | `"bytes"`: the response carries the file. `"redirect"`: the response redirects to the object in the store at `store_url` |
| `store_url` | With `"redirect"`, the URL of the store, such that an object is at `<store_url>/<aa>/<sha256>.<extension>` |
| `max_age` | How many seconds a response for the current version may be cached. Default 60 |
| `cors` | When present, the value of `Access-Control-Allow-Origin` |

A routes file with another key, an invalid pattern, or a route that gives the region or the view twice or not at all is refused when the resolver starts.

## Responses

The resolver answers `GET` and `HEAD`, and `OPTIONS` with `204` and the methods it allows. A path that no route matches, or that the view does not have, is `404` with `Cache-Control: public, max-age=<max_age>`. Other methods are `405`. A file whose object cannot be read is `500` with `Cache-Control: no-store`.

With `"bytes"`, a file is `200` with:

| Header | Value |
|---|---|
| `Content-Type` | The media type of the file |
| `Content-Length` | Its size, or the size of the requested range |
| `ETag` | `"<sha256>"`, the hash of the file |
| `Accept-Ranges` | `bytes` |
| `Content-Disposition` | `inline` with the last segment of the path as the file name, also as `filename*` in UTF-8 when it is not ASCII, so a saved file keeps its name |
| `Cache-Control` | `public, max-age=<max_age>` for the current version, `public, max-age=31536000, immutable` for a named version |

A single byte range, `bytes=<first>-<last>`, `bytes=<first>-` or `bytes=-<length>`, is `206` with `Content-Range`. A range that starts past the end is `416`. A request with several ranges, or with an `If-Range` that is not the file's `ETag`, gets the whole file. An `If-None-Match` that names the file's `ETag`, or `*`, is `304`.

With `"redirect"`, a file is `302` to its object, with the same `Cache-Control`. The object's URL never changes, so a client or cache keeps it for as long as it likes. A client that saves the redirected file names it by its hash; `"bytes"` keeps the path's name.

With `cors`, every response has `Access-Control-Allow-Origin` and exposes `Accept-Ranges`, `Content-Disposition`, `Content-Range` and `ETag`, and `OPTIONS` allows the `Range`, `If-None-Match` and `If-Range` request headers.

A host reads the pointer of a view again at most every `max_age` seconds, so a new publication reaches every path of the view within that time and never mixes two versions in one lookup. The pointer of a named version does not change once it exists, so it is read once.

## Running the resolver

```bash
sekai-exporter serve <output> --routes <file> [--listen <address>] [--threads <n>]
```

The command is built with the `serve` feature, which `full` includes. `<output>` is the output directory of `export` or `update`: pointers are read from `<output>/published`, and objects from `<output>/store`. `--listen` is `127.0.0.1:8080` by default, and `--threads`, the number of threads that answer requests, is the number of logical CPUs by default. The resolver serves HTTP/1.1; TLS and HTTP/2 belong to a reverse proxy or CDN in front of it.

## Other hosts

The core routes a request (`Routes::route`) to a response or to a file to look up, and turns the file a lookup found into a response (`Routes::respond`): a status, headers, and either no body, a short text, or a byte range of an object. A host supplies three reads, the pointer of a view, an object of the store, and a byte range of an object, and writes the response. An edge worker in front of a bucket reads them from the bucket, keeps decoded chunks between requests, and caches objects by their hash, so every path of every view that names one object shares one cached copy.
