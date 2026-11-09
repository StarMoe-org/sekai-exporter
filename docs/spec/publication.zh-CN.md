# 发布文档

[English](publication.md)

读取方凭这些文档，从一个小小的指针出发，就能找到某个版本的每个文件，并知道每个文件是什么。写出这些文档的协议见[发布](../publishing.zh-CN.md)。

## 1. 运行环境

读取发布产物的程序：静态站点、查看器，以及把名称解析为对象的解析器。

## 2. 内容

读取方从 `<区服>/<视图>/current.json` 开始，沿 `root` 找到索引根，再沿某目录的分片找到该目录的条目。索引根、分片与列表都是 store 中的对象，以其 SHA-256 命名。

**指针**包含键 `format`（`"pointer"`）、`version`、`published_version`、`revision` 与 `root`。

**索引根**包含键 `format`（`"index-root"`）、`version`、`published_version`、`revision`、`shards`、`listings` 与 `failures`。`shards` 按目录顺序列出 `{ "directory", "sha256" }`，`listings` 按形式顺序列出 `{ "form", "sha256" }`。`failures` 为每个无法生成的资产列出 `{ "name", "code" }`。

**分片**包含键 `format`（`"index-shard"`）、`version`、`directory` 与 `entries`，每个文件一个条目，按名称排序：

| 键 | 值 |
|---|---|
| `name` | 目录内的文件名 |
| `sha256`、`extension` | 对象在 store 中的名称 |
| `size` | 字节数 |
| `media_type` | 媒体类型 |
| `asset` | 拥有该文件的资产名称 |
| `role` | 文件在资产中的角色。资产文档的角色为 `document` |

**列表**包含键 `format`（`"listing"`）、`version`、`form` 与 `entries`，每个资产一项 `{ "name", "fingerprint", "size" }`，按名称排序。`size` 为全部成员的总大小。

**树列表**位于 `tree` 布局根目录的 `_listing.json`，包含键 `format`（`"tree-listing"`）、`version`、`published_version` 与 `entries`，每个文件一项 `{ "path", "sha256", "asset" }`，按路径排序。

资产文件的路径由其名称决定：文档为 `<名称>.asset.json`；只有一个成员的资产为 `<名称>.<扩展名>`；有多个成员的资产，每个成员为 `<名称>/<角色>.<扩展名>`。

## 3. 来源

这些文档根据某版本的资产文档及其分配的名称写出。

## 4. 容器

UTF-8 JSON，不含无意义的空白，键的顺序与上文相同。

## 5. 正确性

修订号是发布内容排序后各记录的 SHA-256：每个文件的路径、哈希、扩展名、大小、媒体类型、所属资产与角色，以及每个失败的名称与错误码。它不取决于 JSON 的格式，也不取决于资产求值的先后。测试以不同顺序发布相同的资产并比较结果。

## 6. 版本

格式版本：1。上述任何一项变化都会使版本变化。
