# Clip 文档

[English](clip-documents.md)

Clip 文档按引擎的存储方式保存一个 Unity `AnimationClip` 的曲线，并附有说明每条曲线驱动什么的绑定，以及 clip 调用的事件。按本文的规则求值文档，得到的值与引擎从 clip 得到的值相同。携带动画的资产形式（例如 [Live2D 形式](live2d.zh-CN.md)）把它作为成员使用，它本身不是一种资产形式。

## 1. 运行时

按游戏语义播放游戏动画 clip 的程序：Moesekai 的 Live2D 查看器中的 Live2D 运行时，它以 Unity 绑定所用的哈希把每条曲线绑定到模型参数（参见[Live2D 动作](../live2d-motion.zh-CN.md)）；以及 Moesekai 的 3D 服装查看器，它把每条曲线绑定到[服装模型](costume.zh-CN.md)的节点或混合形状。二者都以 Unity 绑定所用的哈希绑定曲线，并按 Unity 的方式求值。

## 2. 内容

```json
{
  "format": "clip",
  "version": 2,
  "name": "w-normal01-nod",
  "sample_rate": 60.0,
  "start_time": 0.0,
  "stop_time": 2.6,
  "loop": false,
  "curves": [
    {
      "path": 397467875,
      "attribute": 3702945584,
      "class_id": 114,
      "script": "Live2D.Cubism/Live2D.Cubism.Core.CubismParameter",
      "custom_type": 0,
      "component": 0,
      "kind": "streamed",
      "segments": [[-3.4028235e+38, 0.0, 0.0, 0.0, 0.0], [0.0, 1.5, -2.25, 0.0, 0.0]]
    },
    { "path": 1622659189, "attribute": 3702945584, "class_id": 114, "script": "…", "custom_type": 0, "component": 0, "kind": "constant", "value": 1.0 }
  ],
  "events": [
    { "time": 0.0, "function": "OnLive2DInvokeUserData", "data": "eyeblink,0.2,0.79,0.24", "float": 0.0, "int": 0 }
  ]
}
```

| 键 | 值 |
|---|---|
| `format`、`version` | `"clip"` 与本文档格式的版本 |
| `name` | `m_Name` |
| `sample_rate` | `m_SampleRate`，单位为帧每秒 |
| `start_time`、`stop_time` | `m_MuscleClip.m_StartTime` 与 `m_StopTime`，单位为秒 |
| `loop` | `m_MuscleClip.m_LoopTime`：clip 播到末尾后是否重新开始 |
| `curves` | 每条曲线一项，按其所属绑定在 `m_ClipBindingConstant.genericBindings` 中的顺序排列（见 §3） |
| `events` | `m_Events`，按存储顺序 |

`curves` 的每一项写明其绑定并保存其曲线：

| 键 | 值 |
|---|---|
| `path` | 绑定的 `path`：目标在动画对象之下的路径的 CRC-32，例如 `crc32("Parameters/ParamAngleX")` 或 `crc32("Position/Hip")` |
| `attribute` | 绑定的 `attribute`：对于 `Transform`，为下表中属性的编号；对于混合形状，为通道名称的 CRC-32，例如 `crc32("eye_close")`；其他情况为属性名的 CRC-32，例如 `crc32("Value")` |
| `class_id` | 绑定的 `typeID`：曲线所驱动组件的类 |
| `script` | 绑定到脚本组件时为其脚本，写作 `"<程序集>/<命名空间>.<类>"`；否则为 `null` |
| `custom_type` | 绑定的 `customType` |
| `component` | 该曲线在其所属绑定的曲线中的位置，从 0 开始。只有 `Transform` 绑定有多于一条曲线 |
| `kind` | `"streamed"`、`"dense"` 或 `"constant"`，并带有下表中的键 |

| 种类 | 键 | 时刻 `t` 的值 |
|---|---|---|
| `streamed` | `segments`：每段为 `[t₀, a, b, c, d]`，按 `t₀` 递增 | 取 `t₀ ≤ t` 的最后一段：`((a·δ + b)·δ + c)·δ + d`，其中 `δ = t − t₀` |
| `dense` | `begin_time`、`sample_rate`、`samples` | 令 `p = max(0, (t − begin_time) · sample_rate)`、`i = min(⌊p⌋, n − 1)`、`j = min(i + 1, n − 1)`：`samples[i] + (samples[j] − samples[i]) · clamp(p − i, 0, 1)` |
| `constant` | `value` | `value` |

指向 `Transform`（类 4）的绑定驱动变换的一个属性，每个分量对应一条曲线：

| `attribute` | 属性 | 曲线（`component` 为 0、1、……） |
|---|---|---|
| 1 | 局部位置 | x、y、z |
| 2 | 局部旋转，以四元数表示 | x、y、z、w |
| 3 | 局部缩放 | x、y、z |
| 4 | 局部旋转，以欧拉角（度）表示，依次绕 Z、X、Y 施加 | x、y、z |

数值以 Unity 坐标表示。clip 未绑定的属性保持其值。`class_id` 为 137（`SkinnedMeshRenderer`）且 `custom_type` 为 20 的绑定驱动混合形状通道：`attribute` 为通道名称的 CRC-32，网格把它存为该通道的 `nameHash`；数值为通道的权重，当它等于该帧的权重时，该通道的帧完全生效。

全部运算均为 32 位浮点运算，按所写顺序进行。streamed 曲线的第一段起点为 −3.4028235 × 10³⁸，即最小的 32 位数，与 Unity 的存储相同，因此每个时刻都有对应的段。时刻指 clip 时间，即播放位置加上 `start_time`。

`events` 的每一项有以下键：`time`（秒）、`function`（`functionName`）、`data`（字符串参数）、`float`（`floatParameter`）与 `int`（`intParameter`）。

## 3. 来源

clip 数据为 `m_MuscleClip.m_Clip.data`。其曲线依次编号：先是 streamed 曲线，然后是 dense 曲线，最后是 constant 曲线。绑定依此顺序取用曲线：`Transform` 绑定取其属性分量数个相连的曲线，其他绑定各取一条。

- **streamed 曲线**（`m_StreamedClip`）：`data` 是一串 32 位字，按帧读取。一帧依次为时间（32 位浮点数）、键数 *n*（32 位整数）与 *n* 个键。一个键依次为曲线序号（32 位整数）与系数 `a`、`b`、`c`、`d`（32 位浮点数）。一个键在该帧的时间开始其曲线的一段。Unity 以一个时间为 +∞、不含键的帧结束这一串，该帧不写入文档。
- **dense 曲线**（`m_DenseClip`）：`m_SampleArray` 逐帧保存 `m_FrameCount` 帧，每帧 `m_CurveCount` 个值。第 *i* 条 dense 曲线的采样是每帧的第 *i* 个值。`begin_time` 为 dense clip 的 `m_BeginTime`，`sample_rate` 为其 `m_SampleRate`。
- **constant 曲线**（`m_ConstantClip`）：`data` 的第 *i* 个值即第 *i* 条 constant 曲线。

绑定的 `script` 取自它所引用的 `MonoScript`：去掉 `.dll` 的 `m_AssemblyName`、`m_Namespace` 与 `m_ClassName`。

**文档以外的结果：**

| 情况 | 结果 |
|---|---|
| `m_Legacy` 或 `m_Compressed` 为真，或旧式表示的某个曲线列表（`m_FloatCurves`、`m_PPtrCurves`、`m_PositionCurves` 等）不为空 | 错误 `unsupported` |
| 指向 `Transform` 的绑定，其属性不在 1 至 4 之内；某个绑定为引用、整数或序列化引用曲线；`pptrCurveMapping` 不为空；或 streamed clip 含有离散曲线 | 错误 `unsupported` |
| `m_HasGenericRootTransform` 或 `m_HasMotionFloatCurves` 为真，或 `m_CycleOffset` 不为 0 | 错误 `unsupported` |
| 某个事件带有对象参数 | 错误 `unsupported` |
| 曲线数与绑定数不同、某个键指向不存在的曲线、帧时间不递增、dense 采样数组长度不对，或除最后一帧时间以外的某个值不是有限数 | 错误 `malformed` |

## 4. 容器

UTF-8 JSON，无多余空白，键的顺序与上表相同。每个数写成能读回同一个 32 位浮点数的最短十进制形式，可能带指数；`path` 与 `attribute` 是无符号 32 位整数。

## 5. 正确性

文档原样保留求值所读的每个 clip 数据值，求值规则即 Unity 的 streamed、dense 与 constant clip 的规则。持续集成用合成数值构造 clip，检查解码出的段与采样等于构造时的数值，检查 `Transform` 绑定如何取用其曲线，检查上述各种情况，并固定一份文档的字节。在真实资源包上，文档的每条曲线在查看器帧时钟的每一帧及相邻两帧的中点求值，所得值与对同一 clip 的独立解码逐位相等；[`tools/compare-live2d-motion`](../../tools/compare-live2d-motion/README.zh-CN.md) 以同样的方式求值所存的 clip。

## 6. 版本

格式版本：2。第 2 版增加了 `Transform` 绑定与 `component` 键。上述任何内容的改动都会改变版本。
