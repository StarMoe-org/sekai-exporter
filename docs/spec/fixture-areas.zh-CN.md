# 家具区域

[English](fixture-areas.md)

MySekai 中家具的放置格。`fixture-meta` 形式是一件家具的格，`fixture-areas` 形式把它们合并成一份 JSON 文档。转换器：`fixture-meta` 与 `fixture-areas`。

## 1. 运行时

复现 MySekai 的运行时在放置一件家具时读取它的格：哪些格可以在其上放置其他家具、角色可以在哪里移动、过场覆盖什么，以及哪些格算作已使用。

## 2. 内容

每种形式都有一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

`fixture-meta` 文档含有四个格：`stackEnables`、`motionArea`、`cutsceneArea` 与 `AddUsingGrid`。一个格是按其存储顺序排列的行列表，一行是按其存储顺序排列的布尔值列表。一个格的所有行长度相同。没有行的格为空列表。

`fixture-areas` 文档含有 `packages`，后者从某个包的路径最后一段映射到该包的 `fixture-meta` 文档。没有此类对象的包不在 `packages` 中。

## 3. 来源

`fixture-meta`：脚本为 `Sekai.Mysekai.FixtureBundleMeta` 的 `MonoBehaviour`。每个格都是同名的字段，其中 `array` 保存各行，各行以 `columns` 保存各格。不为零的格为 `true`。

`fixture-areas`：`mysekai/fixture/` 之下各包的 `fixture-meta` 资产。没有此类资产的版本没有资产。

**文档以外的结果：** 对象没有的字段或类型不符的字段，或各行长度不一致的格，使资产成为错误码为 `malformed` 的错误。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。`fixture-meta` 资产取其容器路径的名称。`fixture-areas` 资产命名为 `mysekai-runtime/fixture-areas`。

## 5. 正确性

针对合成对象的测试覆盖格的各格、空格以及非矩形的格。针对合成版本的测试覆盖各包的合并以及没有任何此类资产的版本。真实版本的资产必须含有每个具有此类对象的包。

## 6. 版本

两种形式的版本均为 1。
