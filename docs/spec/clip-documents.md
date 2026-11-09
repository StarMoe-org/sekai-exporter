# Clip documents

[中文](clip-documents.zh-CN.md)

A clip document holds the curves of one Unity `AnimationClip` as the engine stores them, with the bindings that say what each curve drives and the events the clip calls. A program that evaluates the document as described here gets the values the engine gets from the clip. Asset forms that carry animation, such as the [Live2D forms](live2d.md), use it as a member. It is not an asset form of its own.

## 1. Runtime

Programs that play the game's animation clips with the game's semantics: the Live2D runtime of Moesekai's Live2D viewer, which binds each curve to a model parameter (see [Live2D motions](../live2d-motion.md)), and Moesekai's 3D costume viewer, which binds each curve to a node or a blend shape of a [costume model](costume.md). Both bind a curve by the hashes Unity binds it by and evaluate it as Unity does.

## 2. Contents

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

| Key | Value |
|---|---|
| `format`, `version` | `"clip"` and the version of this document format |
| `name` | `m_Name` |
| `sample_rate` | `m_SampleRate`, in frames per second |
| `start_time`, `stop_time` | `m_MuscleClip.m_StartTime` and `m_StopTime`, in seconds |
| `loop` | `m_MuscleClip.m_LoopTime`: whether the clip starts again at its end |
| `curves` | One entry per curve, in the order of the bindings of `m_ClipBindingConstant.genericBindings` that they belong to (§3) |
| `events` | `m_Events`, in stored order |

Each entry of `curves` names its binding and holds one curve of it:

| Key | Value |
|---|---|
| `path` | The binding's `path`: the CRC-32 of the path of the target below the animated object, such as `crc32("Parameters/ParamAngleX")` or `crc32("Position/Hip")` |
| `attribute` | The binding's `attribute`: for a `Transform`, the number of the property in the table below; for a blend shape, the CRC-32 of the channel name, such as `crc32("eye_close")`; otherwise the CRC-32 of the property name, such as `crc32("Value")` |
| `class_id` | The binding's `typeID`: the class of the component the curve drives |
| `script` | For a binding to a script component, its script as `"<assembly>/<namespace>.<class>"`; otherwise `null` |
| `custom_type` | The binding's `customType` |
| `component` | The position of this curve among the curves of its binding, from 0. Only `Transform` bindings have more than one curve |
| `kind` | `"streamed"`, `"dense"` or `"constant"`, with the keys below |

| Kind | Keys | Value at time `t` |
|---|---|---|
| `streamed` | `segments`: `[t₀, a, b, c, d]` per segment, by increasing `t₀` | From the last segment with `t₀ ≤ t`: `((a·δ + b)·δ + c)·δ + d`, with `δ = t − t₀` |
| `dense` | `begin_time`, `sample_rate`, `samples` | With `p = max(0, (t − begin_time) · sample_rate)`, `i = min(⌊p⌋, n − 1)` and `j = min(i + 1, n − 1)`: `samples[i] + (samples[j] − samples[i]) · clamp(p − i, 0, 1)` |
| `constant` | `value` | `value` |

A binding to a `Transform` (class 4) drives one property of the transform, and has one curve per component of it:

| `attribute` | Property | Curves (`component` 0, 1, …) |
|---|---|---|
| 1 | Local position | x, y, z |
| 2 | Local rotation, as a quaternion | x, y, z, w |
| 3 | Local scale | x, y, z |
| 4 | Local rotation, as Euler angles in degrees, applied about Z, then X, then Y | x, y, z |

Values are in Unity's coordinates. A property the clip does not bind keeps its value. A binding with `class_id` 137 (`SkinnedMeshRenderer`) and `custom_type` 20 drives a blend shape channel: `attribute` is the CRC-32 of the channel's name, which the mesh stores as the channel's `nameHash`, and the value is the channel's weight, which applies the channel's frame fully when it equals the frame's weight.

All arithmetic is in 32-bit floating point, in the order written. The first segment of a streamed curve starts at −3.4028235 × 10³⁸, the lowest 32-bit number, as Unity stores it, so every time has a segment. A time is the clip time: the playback position plus `start_time`.

Each entry of `events` has the keys `time` (seconds), `function` (`functionName`), `data` (the string parameter), `float` (`floatParameter`) and `int` (`intParameter`).

## 3. Sources

The clip data is `m_MuscleClip.m_Clip.data`. Its curves are numbered streamed curves first, then dense curves, then constant curves. The bindings take the curves in this order: a `Transform` binding takes as many consecutive curves as its property has components, and every other binding takes one.

- **Streamed curves** (`m_StreamedClip`): `data` is a sequence of 32-bit words read as frames. A frame is a time (a 32-bit float), a key count *n* (a 32-bit integer) and *n* keys. A key is a curve index (a 32-bit integer) and the coefficients `a`, `b`, `c` and `d` (32-bit floats). A key starts a segment of its curve at the frame's time. Unity ends the sequence with a frame at +∞ that holds no keys, and that frame is not written.
- **Dense curves** (`m_DenseClip`): `m_SampleArray` holds `m_FrameCount` frames of `m_CurveCount` values each, frame after frame. The samples of dense curve *i* are value *i* of each frame. `begin_time` is `m_BeginTime` and `sample_rate` is `m_SampleRate` of the dense clip.
- **Constant curves** (`m_ConstantClip`): value *i* of `data` is constant curve *i*.

A binding's `script` is read from the `MonoScript` it refers to: `m_AssemblyName` without `.dll`, `m_Namespace` and `m_ClassName`.

**Outcomes other than a document:**

| Case | Result |
|---|---|
| `m_Legacy` or `m_Compressed` is set, or a curve list of the legacy representation (`m_FloatCurves`, `m_PPtrCurves`, `m_PositionCurves` and the others) is not empty | Error `unsupported` |
| A binding to a `Transform` has an attribute other than 1 to 4, a binding is a reference, integer or serialized-reference curve, `pptrCurveMapping` is not empty, or the streamed clip has discrete curves | Error `unsupported` |
| `m_HasGenericRootTransform` or `m_HasMotionFloatCurves` is set, or `m_CycleOffset` is not 0 | Error `unsupported` |
| An event has an object parameter | Error `unsupported` |
| The number of curves differs from the number the bindings take, a key names a curve that does not exist, frame times do not increase, a dense sample array has the wrong length, or a value other than the final frame time is not finite | Error `malformed` |

## 4. Container

UTF-8 JSON without insignificant whitespace, with keys in the order of the tables above. Each number is written as the shortest decimal, possibly with an exponent, that reads back to the same 32-bit float; `path` and `attribute` are unsigned 32-bit integers.

## 5. Correctness

The document keeps every value of the clip data that evaluation reads, unchanged, and the evaluation rules are those of Unity's streamed, dense and constant clips. Continuous integration builds clips from synthetic values, checks the decoded segments and samples against the values they were built from, checks how `Transform` bindings take their curves, checks the cases above, and pins the bytes of a document. On real bundles, every curve of a document is evaluated at every frame of the viewer's clock and at the midpoints between frames, and the values are equal bit for bit to those of an independent decoding of the same clip; [`tools/compare-live2d-motion`](../../tools/compare-live2d-motion/README.md) evaluates the stored clips in the same way.

## 6. Version

Format version: 2. Version 2 adds `Transform` bindings and the `component` key. A change to any of the above changes the version.
