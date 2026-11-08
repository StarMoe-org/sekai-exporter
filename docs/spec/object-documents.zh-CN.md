# 对象文档

[English](object-documents.md)

对象文档列出一个资源包中的每个对象及其全部字段，字段的顺序与名称与文件中的存储一致。它是对象图中立、无损的形式：尚无转换器导出的内容，仍可在此读取。

## 1. 运行时

需要对象本身的程序与使用者：浏览与调试工具、在本仓库之外编写的转换器，以及将本导出器与其他 Unity 读取器相比对的检查。对象文档不是一种资产形式，它记录的是存储，而非含义。

## 2. 内容

每个资源包写出一份文档，位于 `<资源包>/_objects.json`。客户端安装包中的文件视为一个资源包，其名称为该 player data 文件的文件名。

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

### 2.1 资源包层

| 键 | 值 |
|---|---|
| `format`、`version` | `"objects"` 与本规范的版本 |
| `bundle` | 资源包名称 |
| `entries` | 容器中按目录顺序排列的每个条目：路径、种类（序列化文件为 `serialized`，其余条目为 `resource`）、大小及其字节的 SHA-256 |
| `files` | 每个序列化文件一条记录，按目录顺序排列 |

### 2.2 文件层

| 键 | 值 |
|---|---|
| `path` | 该序列化文件的条目路径 |
| `serialized_version` | 文件头中的格式版本 |
| `unity_version`、`platform` | 按存储原样写出的引擎版本字符串与目标平台编号。被剥离的版本字符串亦按原样写出 |
| `type_trees` | 该文件是否存储 type tree |
| `externals` | 该文件的外部文件列表，`index` 与 PPtr 的计数方式一致，从 1 开始。`target` 给出该外部文件在同一版本中解析到的资源包与文件；不解析到任何资源包（引擎内置资源）时为 `null` |
| `objects` | 全部对象，按文件对象表的顺序排列 |

### 2.3 对象层

| 键 | 值 |
|---|---|
| `id` | path ID，以十进制字符串表示 |
| `class` | 类名：对象布局的根类型名；没有已知布局时，取引擎为该类 ID 规定的名称 |
| `class_id` | 文件中存储的类 ID |
| `script` | 经由 `m_Script` 引用脚本的对象写作 `"<程序集>/<命名空间>.<类>"`，读取自被引用的 `MonoScript`，该对象可以位于其他文件。程序集名不含 `.dll` 后缀；类没有命名空间时写作 `"<程序集>/<类>"`。其余对象不含此键。引用无法解析时为 `null` |
| `name` | 对象含有顶层字符串字段 `m_Name` 时，取其值 |
| `container` | 资源包的 `AssetBundle` 对象中指向该对象的容器路径，按容器顺序排列。没有时不含此键 |
| `size`、`sha256` | 对象存储字节的大小与 SHA-256 |
| `fields` | 字段值（§2.4）；对象无法读取时为 `null` |
| `error` | 仅当对象无法恰好读完时出现（§2.5） |

### 2.4 字段值

字段按对象的布局写出：文件中存储的 type tree，或者对不带 type tree 的文件而言，[布局表](../glossary.md)。

| 存储类型 | 写法 |
|---|---|
| 结构 | 对象，其键为按布局顺序排列的字段名 |
| `bool` | 存储字节为 1 时写 `true`，为 0 时写 `false`，其他值写该数值 |
| 8、16、32 位整数，`char` | 数值 |
| 64 位整数 | 十进制字符串，以免超出 2^53 的值在 JSON 读取器中失真 |
| `float`、`double` | 能够读回同一值的最短十进制表示；`"NaN"`、`"Infinity"` 与 `"-Infinity"` 写作字符串。任何 NaN 一律写作 `"NaN"`，不区分其符号位与载荷位；对象的 `sha256` 仍覆盖这些位 |
| `string` | 字节为合法 UTF-8 时写作字符串，否则写作字节载荷 |
| 数组 | 数组 |
| `map` | 按存储顺序排列的 `[键, 值]` 对数组 |
| 8 位整数数组、`TypelessData` | 字节载荷 |
| `PPtr<T>` | `(0, 0)` 写作 `null`，其余写作引用 |

**字节载荷**写作 `{"$bytes": {"size": n, "sha256": "…", "base64": "…"}}`。载荷不超过 4096 字节时含 `base64`。更大的载荷以其哈希命名，作为 store 对象发布，因此文档在不内嵌这些载荷的前提下仍保持无损。

**引用**写作 `{"$ref": {…}}`：

| 目标 | 键 |
|---|---|
| 同一文件中的对象 | `id` |
| 同一资源包中另一文件内的对象 | `file`、`id` |
| 另一资源包中的对象 | `bundle`、`file`、`id` |
| 不解析到任何资源包的外部文件 | `external`（外部文件路径）、`id` |

Unity 的字段名从不以 `$` 开头，因此 `$bytes` 与 `$ref` 不会与字段冲突。

使用 `[SerializeReference]` 的对象存有一个 `references` 注册表。表中每一项的 `data` 按该项所指类型的布局读取，该布局取自文件的被引用类型表，并与其他结构一样写出。指向注册表的字段保留其存储的 `rid` 数值。

### 2.5 错误

无法恰好读完的对象仍会列出。其 `fields` 为 `null`，`error` 为 `{"code": "…", "message": "…"}`，`raw` 为其存储字节的字节载荷。

| 代码 | 含义 |
|---|---|
| `no_layout` | 文件中没有该类的 type tree，也没有布局表覆盖该类 |
| `truncated` | 按布局读取越过了对象末尾 |
| `trailing_bytes` | 布局在对象末尾之前结束 |
| `invalid_layout` | 布局无法解释，例如数组缺少长度字段 |
| `malformed` | 存储的值与文件自身矛盾，例如引用了文件未列出的外部文件 |
| `unresolved_reference_type` | `references` 中的某一项所指的类型不在文件的被引用类型表中 |

读取器从不以默认值替代，也从不丢弃字段。

## 3. 来源

去混淆之后的资源包字节，以及同一版本中全部资源包的索引；后者用于解析外部文件与 `m_Script` 引用。

## 4. 容器

不带字节顺序标记的 UTF-8 JSON，键按上文给出的顺序排列，不含无意义的空白，以一个换行符结尾。相同的输入总是产出相同的字节。

## 5. 正确性

- 每个对象都恰好读到其末尾；任何其他结果均属 §2.5 中的错误。
- 字段值与一个独立的 Unity 读取器相比对，比对对象为由 Unity 编辑器构建、收录于本仓库的资源包，以及使用者自行提供的任何资源包。比对将两个读取器的输出映射到 §2.4 的值模型，并要求每个对象的每个字段均相等。
- 文档在不同线程数之间、使用与不使用块缓存之间逐字节比对。原生构建与 WebAssembly 构建之间的比对已列入计划。

## 6. 版本

版本 1。
