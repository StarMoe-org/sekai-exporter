# 组件文档

[English](component-documents.md)

组件文档列出预制体中不属于几何的组件，例如弹簧、碰撞体与约束，并附带其所存的字段。对游戏对象或其他组件的引用，会变为预制体[模型文件](model.zh-CN.md)中某个节点的路径，因此程序可在模型中找到组件作用的对象。携带模型的资产形式，例如[服装模型](costume.zh-CN.md#costume-model)，把它作为成员使用，并说明它列出哪些组件类。它本身不是资产形式。

## 1. 运行时

Moesekai 的 3D 服装查看器，它在服装模型的节点上模拟服装的弹簧及其碰撞体，并施加其约束。

## 2. 内容

```json
{
  "format": "components",
  "version": 1,
  "components": [
    {
      "class": "MonoBehaviour",
      "script": "ThirdParty.SpringBone/UTJ.SpringManager",
      "node": "",
      "enabled": true,
      "fields": { "springBones": [{ "$ref": { "component": 1 } }], "gravity": { "x": 0.0, "y": -9.8, "z": 0.0 }, "…": "…" }
    },
    {
      "class": "MonoBehaviour",
      "script": "Assembly-CSharp/Sekai.SekaiSpringBone",
      "node": "Position/Hip/Skirt_F_01",
      "enabled": true,
      "fields": { "pivotNode": { "$ref": { "node": "Position/Hip" } }, "radius": 0.02, "…": "…" }
    }
  ]
}
```

| 键 | 值 |
|---|---|
| `format`、`version` | `"components"` 与本文档格式的版本 |
| `components` | 列出的组件，按其游戏对象的深度优先顺序排列，与模型文件相同；同一游戏对象上的组件按其 `m_Component` 的顺序排列 |

| 组件的键 | 值 |
|---|---|
| `class` | 类名，例如 `"AimConstraint"` 或 `"MonoBehaviour"` |
| `script` | 对于 `MonoBehaviour`，为其脚本，写作 `"<程序集>/<命名空间>.<类>"`，与[对象文档](object-documents.zh-CN.md)相同；否则为 `null` |
| `node` | 其游戏对象的路径 |
| `enabled` | `m_Enabled` 是否不为 0 |
| `fields` | 除 `m_GameObject`、`m_Enabled`、`m_Script` 与 `m_Name` 外的所存字段，按存储顺序排列 |

游戏对象的**路径**，为从根之下直到该对象的各游戏对象名称，以 `/` 连接。根的路径为空字符串。同级对象重名时，路径指深度优先顺序中的第一个，与 Unity 自身按路径查找的方式相同。

字段值按存储写出：

| 存储的值 | 写为 |
|---|---|
| 结构 | 以字段名为键、按存储顺序排列的对象 |
| `bool` | `true` 或 `false` |
| 整数 | 数 |
| `float`、`double` | 能读回同一值的最短十进制形式，若该值为 32 位浮点数则采用 32 位浮点的精度；`"NaN"`、`"Infinity"` 与 `"-Infinity"` 为字符串 |
| 字符串 | 字符串 |
| 数组 | 数组 |
| 映射 | 按存储顺序排列的 `[key, value]` 对的数组 |
| 引用 | 如下表 |


| 引用的对象 | 写为 |
|---|---|
| 无 | `null` |
| 预制体的游戏对象或其 `Transform` | `{ "$ref": { "node": "<路径>" } }` |
| 列出的组件 | `{ "$ref": { "component": <在 components 中的索引> } }` |
| 预制体游戏对象上的另一个组件 | `{ "$ref": { "node": "<路径>", "class": "<类名>" } }` |

## 3. 来源

预制体游戏对象的组件，读取方式与模型文件相同。组件的类决定资产形式是否列出它。

| 情况 | 结果 |
|---|---|
| 列出的组件所引用的对象，既非游戏对象也非预制体的组件 | 错误 `unsupported` |
| 列出的组件含有字节数据，或含有非 UTF-8 的字符串 | 错误 `unsupported` |
| 预制体的某个组件无法读取 | 该对象的错误 |

## 4. 容器

UTF-8 JSON，无多余空白，键的顺序与上表相同，字段按存储顺序排列。

## 5. 正确性

字段按存储写出，每个引用都指向与之前相同的对象，其路径经模型文件解析后指向同一节点。持续集成用合成对象构建带有弹簧、碰撞体与约束的预制体，并检查组件的顺序、每种引用以及上述各种情况。在真实资源包上，每份文档的组件都与同一预制体的独立导出结果相比较。

## 6. 版本

格式版本：1。上述任何内容的改动，都会改变使用该格式的每种资产形式的版本。
