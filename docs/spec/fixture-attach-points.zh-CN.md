# 家具挂点

[English](fixture-attach-points.md)

一个版本中每件 MySekai 家具的挂点，合并为一个 JSON 文档。转换器：`fixture-attach-points`。一件家具的挂点是其[家具模型](fixture-model.zh-CN.md#26-挂点)的 `attach-points` 成员，内容由该页定义。

## 1. 运行时

复现 MySekai 的运行时在启动时加载本文档一次。它根据每件已放置家具的挂点，规划角色去哪里使用家具、角色能使用哪件家具，这些都在加载模型之前进行，也不需要模型。加载某件已放置家具的模型时，它按 `sourceObject` 绑定该家具的根节点与碰撞体（见[为节点命名](fixture-model.zh-CN.md#7-为节点命名)）。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

文档含 `packages`：从家具包路径的最后一段，映射到该包 `fixture-model` 资产的 `attach-points` 成员，内容不变。每件有 `fixture-model` 资产的家具都在 `packages` 中，包括没有挂点、`points` 为空列表的家具。`fixture-model` 资产未能产出的家具不在 `packages` 中：运行时无法放置它。

## 3. 来源

`mysekai/fixture/` 下各包的 `fixture-model` 资产。没有这类资产的版本不产出本资产。

**文档以外的结果：** `attach-points` 成员不是 JSON 文档时，资产为错误，代码为 `malformed`。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。资产名为 `mysekai-runtime/fixture-attach-points`。

## 5. 正确性

在合成版本上的测试覆盖各包的合并、没有挂点的家具，以及没有任何家具模型的版本。真实版本的资产必须含有每个具有 `fixture-model` 资产的包，且每个包的内容按 JSON 与该资产的成员逐字节相同。

## 6. 版本

形式版本：1。
