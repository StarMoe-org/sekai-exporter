# 服务

[English](serving.md)

发布按内容为每个文件命名：对象位于 `store/<aa>/<sha256>.<extension>`，其含义由索引承载。按路径访问文件的读取方，例如静态网站、查看器和下载工具，需要一个**解析器**把视图中的路径映射到对象。`sekai-exporter serve` 就是这样的解析器。它的核心 `sekai-serve` crate 不做 I/O，因此同一份代码可以运行在其他宿主中，例如 S3 兼容存储桶前的边缘 worker。

解析器取代物化的路径树。路径树把每个文件复制或链接到固定路径，使存储的对象数随视图数成倍增加，并且无法原子地切换到新版本。解析出的路径树只存它的索引，所有视图共享同一批对象，新版本对所有路径同时生效。

## 视图及其路径索引

**视图**是建立在一次发布的对象之上的路径树：

| 视图 | 路径 |
|---|---|
| `canonical` | [发布](spec/publication.zh-CN.md)的路径：`<name>.asset.json`、`<name>.<extension>` 或 `<name>/<role>.<extension>` |
| `storage` | [storage 视图](spec/storage-view.zh-CN.md)的路径 |

视图的每次发布都向 store 写入一份**路径索引**，并在视图的指针 `<区服>/<视图>/current.json` 中以 `paths` 指向它。同一份指针还保存在 `<区服>/<视图>/versions/<发布版本>.json`，因此只要对象仍保留，较新的版本发布之后旧版本依然可访问。

路径索引由一个小的根文档和一系列**分块**组成，每个分块保存按字节序连续的一段路径，以及每个文件的哈希、扩展名、大小与媒体类型（[格式](spec/path-index.zh-CN.md)）。分块边界由路径的哈希而非位置决定，因此增加或改动少量路径的版本只改变包含它们的分块。未变的分块保留其哈希，发布下一个版本时只上传改变了的分块。

一次查找读取根，用二分查找找到范围包含该路径的分块，再在分块中二分查找该路径。宿主在内存中保留解码后的分块，因此在已读过的分块中查找不再读取任何内容。

## 路由

路由文件说明哪些 URL 路径指向哪些视图。路由按顺序尝试，第一条匹配的路由决定结果：

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

模式以 `/` 开头，由 `/` 分隔的段组成。段可以是字面文本、占位符或两者兼有，例如 `{region}@{version}`；两个占位符之间必须有文本。占位符取所在段中直到其后文本首次出现之前的字符，这些字符必须构成名称：ASCII 字母、数字、`.`、`_` 和 `-`，且不全是点。占位符取不到名称的路由不匹配，继续尝试下一条。第一条匹配的路由即使在其视图中没有该路径也决定结果，因此带字面文本的路由应排在更一般的路由之前。

| 占位符 | 含义 |
|---|---|
| `{region}` | 区服。由占位符或路由的 `region` 键给出，二者只能有一个 |
| `{view}` | 视图。由占位符或路由的 `view` 键给出，二者只能有一个 |
| `{version}` | 可选。已发布的版本：在该版本而非当前版本中解析路径 |
| `{path}` | 必需，且是最后一段。URL 路径的剩余部分经百分号解码后，就是视图中的路径 |

没有发布的区服、视图或版本视为不存在。

| 键 | 含义 |
|---|---|
| `delivery` | `"bytes"`：响应携带文件。`"redirect"`：响应重定向到 `store_url` 下 store 中的对象 |
| `store_url` | 使用 `"redirect"` 时 store 的 URL，对象位于 `<store_url>/<aa>/<sha256>.<extension>` |
| `max_age` | 当前版本的响应可缓存的秒数。默认 60 |
| `cors` | 存在时，作为 `Access-Control-Allow-Origin` 的值 |

路由文件若有其他键、有无效的模式，或某条路由重复给出或未给出区服或视图，解析器在启动时拒绝它。

## 响应

解析器回答 `GET` 和 `HEAD`，对 `OPTIONS` 以 `204` 和允许的方法回答。没有路由匹配或视图中没有的路径返回 `404`，附带 `Cache-Control: public, max-age=<max_age>`。其他方法返回 `405`。对象无法读取的文件返回 `500`，附带 `Cache-Control: no-store`。

使用 `"bytes"` 时，文件以 `200` 返回，附带：

| 头 | 值 |
|---|---|
| `Content-Type` | 文件的媒体类型 |
| `Content-Length` | 文件大小，或所请求范围的大小 |
| `ETag` | `"<sha256>"`，文件的哈希 |
| `Accept-Ranges` | `bytes` |
| `Content-Disposition` | `inline`，以路径的最后一段为文件名；不是 ASCII 时另以 UTF-8 的 `filename*` 给出，因此保存下来的文件保留其名称 |
| `Cache-Control` | 当前版本为 `public, max-age=<max_age>`，指定版本为 `public, max-age=31536000, immutable` |

单个字节范围 `bytes=<first>-<last>`、`bytes=<first>-` 或 `bytes=-<length>` 返回 `206`，附带 `Content-Range`。起点越过末尾的范围返回 `416`。请求多个范围，或 `If-Range` 不是文件的 `ETag` 时，返回整个文件。`If-None-Match` 列出文件的 `ETag` 或 `*` 时返回 `304`。

使用 `"redirect"` 时，文件以 `302` 重定向到其对象，附带相同的 `Cache-Control`。对象的 URL 永不改变，客户端或缓存可以任意长久地保留它。保存重定向文件的客户端以哈希为其命名；`"bytes"` 则保留路径中的名称。

设置 `cors` 时，每个响应都带有 `Access-Control-Allow-Origin`，并公开 `Accept-Ranges`、`Content-Disposition`、`Content-Range` 与 `ETag`；`OPTIONS` 允许请求头 `Range`、`If-None-Match` 与 `If-Range`。

宿主至多每 `max_age` 秒重新读取一次视图的指针，因此新发布在这段时间内到达视图的每个路径，并且一次查找绝不会混用两个版本。指定版本的指针一旦存在就不再改变，因此只读取一次。

## 运行解析器

```bash
sekai-exporter serve <output> --routes <file> [--listen <address>] [--threads <n>]
```

该命令由 `serve` feature 构建，`full` 包含它。`<output>` 是 `export` 或 `update` 的输出目录：指针从 `<output>/published` 读取，对象从 `<output>/store` 读取。`--listen` 默认为 `127.0.0.1:8080`；`--threads` 是回答请求的线程数，默认为逻辑 CPU 数。解析器提供 HTTP/1.1；TLS 与 HTTP/2 由其前面的反向代理或 CDN 负责。

## 其他宿主

核心把请求路由（`Routes::route`）为一个响应或一个待查找的文件，再把查找到的文件转为响应（`Routes::respond`）：状态码、头，以及无正文、一段短文本或某个对象的字节范围。宿主提供三种读取：视图的指针、store 中的对象、对象的字节范围，并写出响应。存储桶前的边缘 worker 从存储桶读取它们，在请求之间保留解码后的分块，并按哈希缓存对象，因此指向同一对象的所有视图的所有路径共享一份缓存。
