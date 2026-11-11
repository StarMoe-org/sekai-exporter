# 资产文档

[English](asset-documents.md)

资产文档说明一个资产是什么：它的形式、由哪些对象生成、由哪些文件组成，以及每个文件的角色。持有文档与 store 的人无需目录即可使用该资产。

## 1. 运行环境

读取发布产物的程序：查看器、转换工具，以及目录。目录是这些文档的索引，可由它们重建。

## 2. 内容

每个资产写一份文档，位于 `<名称>.asset.json`，文档本身也是 store 中的对象。

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

| 键 | 值 |
|---|---|
| `format` | `"asset"` |
| `version` | 本文档格式的版本 |
| `name` | 资产名称：逻辑路径加上[命名规则](../architecture.zh-CN.md)分配的后缀 |
| `form`、`form_version` | 资产形式及其版本。每种形式在本目录下有一份规范 |
| `fingerprint` | 资产的[指纹](../pipeline.zh-CN.md#不变式)，小写十六进制 |
| `sources` | 转换实际读取的对象：根对象在前，其余对象按资源包与 path ID 排序。`path_id` 为十进制字符串，与[对象文档](object-documents.zh-CN.md)一致 |
| `resources` | 仅当转换读取了资源文件时出现：`{ "bundle": "…", "file": "…" }`，按资源包与文件名排序 |
| `members` | 资产的文件，顺序由形式定义。`role` 由形式定义，在资产内唯一。`sha256` 是文件在 store 中的名称。`size` 以字节为单位 |
| `references` | 该资产引用的其他资产的名称，按升序排列 |
| `error` | 仅在资产无法生成时出现：`{ "code": "…", "message": "…" }`，此时 `members` 为空 |

每个成员列出该成员的规范文件。视图以其他方式编码的文件列在该视图的索引中。

## 3. 来源

求值器根据转换器的结果、版本索引以及该版本分配的名称写出文档。其中没有任何内容来自配置。

## 4. 容器

UTF-8 JSON，不含无意义的空白，键的顺序与上表相同。`error` 不存在时省略。

## 5. 正确性

文档是指纹输入与版本名称的函数，因此相同的输入得到相同的字节。持续集成固定一份由固定值构建的文档的字节，并检查文档读回后值不变。

## 6. 版本

格式版本：1。上述任何一项变化都会使版本变化。
