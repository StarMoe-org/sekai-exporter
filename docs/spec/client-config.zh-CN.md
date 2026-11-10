# 客户端配置

[English](client-config.md)

`client-config` 形式是客户端按数字 ID 读取的配置，写成一份 JSON 文档。转换器：`client-config`。

## 1. 运行时

复现游戏的运行时按设置项的 ID 及其所期望的类型请求该设置项，并把取值用于玩法常量，例如相机限制与时长。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

文档含有四个对象，每个都从字符串形式的 ID 映射到一个值，按 ID 顺序排列：

| 键 | 值 |
|---|---|
| `FloatConfigs` | 数 |
| `IntConfigs` | 整数 |
| `StringConfigs` | 字符串 |
| `BoolConfigs` | 布尔值 |

## 3. 来源

游戏数据表 `clientConfigs`，其行为 `id`、`type` 与 `value`。`type` 指明该行所属的对象：`Float`、`Int`、`String` 或 `Bool`。`value` 是字符串，按该类型读取。`Bool` 的值为 `true` 或 `false`，不区分大小写。

**文档以外的结果：** 类型不符的行、同一类型下两行共用的 ID，或无法按其类型读取的值，使资产成为错误码为 `malformed` 的错误。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。资产命名为 `mysekai-runtime/client-config`。

## 5. 正确性

针对合成表的测试覆盖每种类型以及上述每一种错误结果。真实版本的资产必须为表中的每一行保存一个值。

## 6. 版本

形式版本 1。
