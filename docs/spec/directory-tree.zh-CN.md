# 目录树

[English](directory-tree.md)

目录树在一份文档中保存视图某次发布的全部目录与文件：每个文件的名称、大小与媒体类型，不含存放它们的对象。模块：`sekai-format` 中的 `directory_tree`。`sekai-publish` 为视图的每次发布写出一份，`sekai-serve` 经[目录树路由](../serving.zh-CN.md#目录树)发送它。格式版本：1。

## 1. 运行时

一次浏览整个视图的读取方，例如网页的文件浏览器：一次请求得到整棵树，之后按路径在同一发布版本中读取文件，即[解析器](../serving.zh-CN.md)路由下的 `/{region}@{version}/{path}`。

## 2. 内容

文档包含键 `format`（`"directory-tree"`）、`version`、`view`、`published_version`、`revision`、`files`（文件数）、`media_types` 与 `root`。

`media_types` 把文件的每种媒体类型各列一次，按路径字节序中首次出现的顺序。文件以该列表中的位置指明其媒体类型。

**目录**包含以下键：

| 键 | 值 |
|---|---|
| `name` | 目录名，即其下路径中的一段。`root` 为空 |
| `directories` | 子目录，按名称的字节序。没有时省略 |
| `files` | 直接位于该目录下的文件。没有时省略 |

`files` 每个字段一列，均按名称的字节序：

| 键 | 值 |
|---|---|
| `names` | 文件名 |
| `sizes` | 大小（字节） |
| `media_types` | 每个文件的媒体类型在 `media_types` 中的位置 |

文件的路径是从 `root` 往下各级目录名加上文件名，以 `/` 连接。例如文件 `a.txt`、`a/c/d.json` 与 `a/z.png` 得到：

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

## 3. 来源

路径、大小与媒体类型取自发布的文件，与其[路径索引](path-index.zh-CN.md)中的相同。有文件位于其下的目录才存在。目录树只取决于发布的文件、视图、发布版本与修订号：作为对上一次发布的改动写出的发布，与整体写出的发布得到相同的目录树。

## 4. 容器

文档是不含多余空白的 UTF-8 JSON，键按上述顺序，并以 gzip 压缩。它以 `<aa>/<sha256>.json.gz` 存放，媒体类型为 `application/gzip`，发布的指针在 `tree` 下以其 SHA-256 与大小指明它（见[发布文档](publication.zh-CN.md)）。

网页用浏览器的 `DecompressionStream` 解压：

```js
const response = await fetch("/trees/jp/storage");
const tree = await new Response(response.body.pipeThrough(new DecompressionStream("gzip"))).json();
```

## 5. 正确性

读取方拒绝其他格式或版本的目录树，以及以下情况：根有名称；目录名或文件名为空或含 `/`；某目录的各列长度不同；媒体类型位置超出 `media_types`；某路径列出两次；文件数与列出的文件不符。

测试由已知文件构建目录树并检查文档内容，检查读回的文件就是构建所用的文件，并检查乱序、重复或含空段的文件无法构成目录树。发布的测试检查：无论上一次发布有目录树还是只有路径索引，改动得到的目录树都与整体发布得到的相同。

## 6. 版本

格式版本：1。上述任何一项变化都会使版本变化。
