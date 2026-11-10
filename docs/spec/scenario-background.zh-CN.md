# 剧本背景

[English](scenario-background.md)

`scenario-background` 形式是剧本背景播放的影片，写成 JSON 文档。转换器：`sekai-moesekai` 中的 `scenario-background`（feature `episode`）。形式版本：1。

## 1. 运行时

构建 `episode` 资产的数据表阶段，在它构建场景数据指明某个背景预制体的一话时：它读取该预制体的影片，使这一话指明播放这些影片的 `video` 资产。形式给出预制体所存的名称；它不把名称解析为文件。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `scenario-background` | JSON 文档，扩展名 `json` |

```json
{ "format": "scenario-background", "version": 1, "movies": ["backdrop_loop", "backdrop_b"] }
```

| 键 | 值 |
|---|---|
| `format`、`version` | `"scenario-background"` 与本文档格式的版本，1 |
| `movies` | 影片名称，各一次，按遍历预制体找到它们的顺序排列 |

影片名称是游戏从中加载该影片的资源包名称，按预制体的存储原样。空名称不产生条目。

## 3. 来源

**根对象：** 脚本为 `Assembly-CSharp/Sekai.ScenarioBackgroundBuildData` 的每个 `MonoBehaviour`（类 114）。

**读取的字段：** 根对象的 `BackgroundPrefab`；遍历所读的每个对象的 `m_IsActive`、`m_Component`、`m_Children`、`m_GameObject` 与 `movieAssetBundleName`。

**遍历。** 遍历从 `BackgroundPrefab` 所指的游戏对象开始，按深度优先访问对象，子对象按同级顺序：

1. 弹出一个对象。不是游戏对象（类 1）的对象其下没有内容，在此结束。`m_IsActive` 为 false、或既不是布尔值也不是整数的游戏对象连同其子树一起跳过。`m_IsActive` 缺失或为 `true` 时为激活，整数时不为零即激活。
2. 按顺序访问其 `m_Component` 数组的各项。每一项是对组件的引用，或是其 `component` 为对组件引用的结构；空引用，或对未持有对象的引用，予以跳过。变换（类 4）或矩形变换（类 224）给出此对象的子对象：对其 `m_Children` 的每一项，按顺序取对变换的引用、该变换的 `m_GameObject` 以及对游戏对象的引用，同上。脚本为 `Assembly-CSharp/Sekai.ScenarioBackgroundMovie` 的 `MonoBehaviour`（类 114）在其 `movieAssetBundleName` 为非空字符串且先前的影片都没有同名时，加入该名称。
3. 将此对象的子对象压栈，倒序压入，使下一个弹出的对象是其第一个子对象。

遍历已访问过的游戏对象不再访问，因此子对象的环使遍历结束。找到的影片即游戏通过收集预制体之下激活对象的影片组件所找到的影片。

**文档以外的结果：**

| 情况 | 结果 |
|---|---|
| `BackgroundPrefab` 缺失或为 null | 无资产 |
| 遍历未找到影片 | 无资产 |
| `BackgroundPrefab`、`m_Component` 或 `m_Children` 的某项、`component` 或 `m_GameObject` 不是引用，或 `m_Component`、`m_Children` 或 `movieAssetBundleName` 类型不符 | 错误 `malformed` |
| 某个引用指向该版本未持有的对象 | 解析该引用时的错误 `missing_object` |

`BackgroundPrefab` 从根对象读取，遍历中某个变换的 `m_GameObject` 从该变换读取。

## 4. 容器

成员是成品字节：UTF-8 JSON，无多余空白，键的顺序如上，名称按遍历顺序排列。遍历按固定顺序读取对象并让每个名称只保留一次，因此同一预制体总是给出相同的字节。

## 5. 正确性

文档指明游戏在预制体之下找到的影片，因此指明某个背景影片的一话，所指明的就是游戏所加载的影片。

在 `crates/sekai-format` 的测试中，文档写成一串固定的字节，再从这些字节读回。在 `crates/sekai-moesekai` 的测试（`tests/story_data.rs`）中，在内存中构建的对象检查：

- 根规则；
- 含一个影片的预制体的文档字节；
- 遍历按同级顺序深度优先，包括预制体根的影片、在其父节点的下一个同级节点之前访问的孙节点，以及矩形变换；
- `m_IsActive` 为 `false`、`0` 或 null 时，未激活的对象连同其子树被跳过，未激活的预制体根不产出资产，而 `true` 与非零整数为激活；
- 两次到达的影片只保留一次，同一对象两次列出它时也是如此，空名称不加入影片；
- 其他脚本或其他类的组件被忽略；
- 空预制体、预制体字段缺失或预制体没有影片时不产出资产；
- 子对象的环会结束，包括互为子对象的两个游戏对象；
- 对缺失对象的引用给出 `missing_object`；
- 其他类型的字段给出 `malformed`。

## 6. 版本

| 版本 | 变更 |
|---|---|
| 1 | 第一版 |
