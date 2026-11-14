# 发布文档

[English](publication.md)

读取方凭这些文档，从一个小小的指针出发，就能找到某个版本的每个文件，并知道每个文件是什么。写出这些文档的协议见[发布](../publishing.zh-CN.md)。

## 1. 运行环境

读取发布产物的程序：静态站点、查看器，以及把名称解析为对象的解析器。

## 2. 内容

读取方从 `<区服>/<视图>/current.json` 开始，沿 `root` 找到索引根，在目录表中找到包含某目录的块，再沿该目录的分片找到它的条目。索引根、表块与分片都是 store 中的对象，以其 SHA-256 命名。

**指针**包含键 `format`（`"pointer"`）、`version`、`published_version` 与 `revision`，其后是布局所具有的 `root`（索引根的小写十六进制 SHA-256）、`sqlite`（`store/<aa>/<sha256>.sqlite` 的 SHA-256）和 `paths`（[路径索引](path-index.zh-CN.md)根的 SHA-256，每种布局都有）。当前版本的指针位于 `<区服>/<视图>/current.json`，每个已发布版本的指针以相同内容位于 `<区服>/<视图>/versions/<published_version>.json`。发布版本的名称由 ASCII 字母、数字、`.`、`_` 和 `-` 组成，且不全是点。

**`index.sqlite` 数据库**（`store/<aa>/<sha256>.sqlite`）是一份不可变、内容寻址的 SQLite 3 数据库（`page_size = 4096`、UTF-8、schema format 4、`user_version = 2`），自底向上紧凑构建、无空闲列表页，使静态 HTTP `Range`（`206 Partial Content`）客户端（`sql.js-httpvfs`、`wa-sqlite`）在 1–3 次页读取内即可完成目录展开、精确路径查找、按资源包/形式筛选及完整资产溯源查询：

- `meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID`：`format`（`"sqlite-index"`）、`published_version`、`revision`、`schema_version`（`"2"`）。
- `bundles (id INTEGER PRIMARY KEY, name TEXT NOT NULL, asset_count INTEGER NOT NULL, file_count INTEGER NOT NULL, total_size INTEGER NOT NULL)`（`idx_bundles_name`）。
- `forms (id INTEGER PRIMARY KEY, name TEXT NOT NULL, form_version INTEGER NOT NULL, asset_count INTEGER NOT NULL, total_size INTEGER NOT NULL)`（`idx_forms_name`）。
- `directories (parent TEXT NOT NULL, name TEXT NOT NULL, path TEXT NOT NULL, file_count INTEGER NOT NULL, total_size INTEGER NOT NULL, PRIMARY KEY (parent, name)) WITHOUT ROWID`：各目录子树的递归文件数与总字节数。
- `assets (id INTEGER PRIMARY KEY, bundle_id INTEGER NOT NULL, name TEXT NOT NULL, form_id INTEGER NOT NULL, fingerprint BLOB NOT NULL, total_size INTEGER NOT NULL, error_code TEXT, error_message TEXT, sources_json TEXT NOT NULL, resources_json TEXT NOT NULL, references_json TEXT NOT NULL)`（`idx_assets_name`、`idx_assets_bundle`、`idx_assets_form`、`idx_assets_error`）。
- `files (dir TEXT NOT NULL, file_name TEXT NOT NULL, path TEXT NOT NULL, bundle_id INTEGER NOT NULL, asset_id INTEGER NOT NULL, role TEXT NOT NULL, sha256 BLOB NOT NULL, extension TEXT NOT NULL, size INTEGER NOT NULL, media_type TEXT NOT NULL, PRIMARY KEY (dir, file_name)) WITHOUT ROWID`（`idx_files_path`、`idx_files_asset`）。每个文件都是对象 `store/<aa>/<sha256>.<extension>`。

**索引根**包含键 `format`（`"index-root"`）、`version`、`published_version`、`revision`、`directories`、`listings` 与 `failures`。除 `listings` 外每一项都是一张表；`listings` 按形式顺序列出 `{ "form", "chunks" }`，其中 `chunks` 是该形式列表的表：

| 表 | 行（按键排序） | 键 |
|---|---|---|
| `directories` | `{ "directory", "sha256" }`：每个目录的分片 | `directory` |
| 某形式的列表 | 该形式每个资产一项 `{ "name", "fingerprint", "size" }`；`size` 为全部成员的总大小 | `name` |
| `failures` | 每个无法生成的资产一项 `{ "name", "code" }` | `name` |

**表**按键的顺序以 `{ "first", "sha256", "rows" }` 列出它的块：首行的键、块对象的 SHA-256 与行数。**块**包含键 `format`（`"index-chunk"`）、`version` 与 `rows`。一行的键经 SHA-256 后，前四个字节按大端 32 位数读取为 512 的倍数时，块在该行之后结束；否则在第 2048 行之后结束；最后一块以最后一行结束。因此改动一行只改变包含它的块，除非改动移动了边界。没有行的表没有块，没有资产的形式不列出。

**分片**包含键 `format`（`"index-shard"`）、`version`、`directory` 与 `entries`，每个文件一个条目，按名称排序：

| 键 | 值 |
|---|---|
| `name` | 目录内的文件名 |
| `sha256`、`extension` | 对象在 store 中的名称 |
| `size` | 字节数 |
| `media_type` | 媒体类型 |
| `asset` | 拥有该文件的资产名称 |
| `role` | 文件在资产中的角色。资产文档的角色为 `document` |

**树列表**位于 `tree` 布局根目录的 `_listing.json`，包含键 `format`（`"tree-listing"`）、`version`、`published_version` 与 `entries`，每个文件一项 `{ "path", "sha256", "asset" }`，按路径排序。

资产文件的路径由其名称决定：文档为 `<名称>.asset.json`；只有一个成员的资产为 `<名称>.<扩展名>`；有多个成员的资产，每个成员为 `<名称>/<角色>.<扩展名>`。

## 3. 来源

这些文档根据某版本的资产文档及其分配的名称写出。

## 4. 容器

UTF-8 JSON，不含无意义的空白，键的顺序与上文相同。

## 5. 正确性

修订号是以下文本的小写十六进制 SHA-256：`publication 3\nversion <published_version>\n`，其后按顺序为路径索引的每个块写 `paths <sha256>\n`，在有索引根的布局中再为失败表的每个块写 `failures <sha256>\n`。两者的块只取决于其中的文件与失败，因此修订号既不取决于资产求值的先后，也不取决于发布是整体写出还是作为对上一次发布的改动写出。

发布可以作为对该视图上一次发布的改动写出：移除已删除或文档有变化的资产的文件、列表项与失败，再放入新文档的。只读写改动涉及的分片与块，结果与整体写出相同资产得到的发布一致。若改动要把文件放到另一资产占用的路径，则失败，与整体发布中两个文件同路径一样。测试把相同资产分别整体发布、以及作为从不同旧资产出发的改动发布，并比较指针。

## 6. 版本

格式版本：3。上述任何一项变化都会使版本变化。

| 版本 | 变更 |
|---|---|
| 2 | 每种布局都有分片、列表、失败与路径索引 |
| 3 | 索引根的目录、列表与失败改为分块的表；修订号由块得出 |
