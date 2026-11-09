# Component documents

[中文](component-documents.zh-CN.md)

A component document lists components of a prefab that are not geometry, such as springs, colliders and constraints, with their stored fields. A reference to a game object or to another component becomes the path of a node of the prefab's [model file](model.md), so a program finds the objects a component works on in the model. Asset forms that carry a model, such as the [costume model](costume.md#costume-model), use it as a member and say which component classes it lists. It is not an asset form of its own.

## 1. Runtime

Moesekai's 3D costume viewer, which simulates the springs of a costume, with their colliders, and applies its constraints, on the nodes of the costume's model.

## 2. Contents

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

| Key | Value |
|---|---|
| `format`, `version` | `"components"` and the version of this document format |
| `components` | The listed components, by the depth-first order of their game objects, as in the model file, and on one game object in the order of its `m_Component` |

| Key of a component | Value |
|---|---|
| `class` | The class name, such as `"AimConstraint"` or `"MonoBehaviour"` |
| `script` | For a `MonoBehaviour`, its script as `"<assembly>/<namespace>.<class>"`, as in [object documents](object-documents.md); otherwise `null` |
| `node` | The path of its game object |
| `enabled` | Whether `m_Enabled` is not 0 |
| `fields` | The stored fields except `m_GameObject`, `m_Enabled`, `m_Script` and `m_Name`, in stored order |

The **path** of a game object is the names of the game objects from below the root down to it, joined by `/`. The root's path is the empty string. Where siblings share a name, a path names the first of them in depth-first order, as Unity's own lookup by path does.

Field values are written as stored:

| Stored value | Written as |
|---|---|
| Structure | An object whose keys are the field names in stored order |
| `bool` | `true` or `false` |
| Integer | A number |
| `float`, `double` | The shortest decimal that reads back to the same value, at the precision of a 32-bit float when the value is one; `"NaN"`, `"Infinity"` and `"-Infinity"` as strings |
| String | A string |
| Array | An array |
| Map | An array of `[key, value]` pairs in stored order |
| Reference | As in the table below |


| Reference to | Written as |
|---|---|
| Nothing | `null` |
| A game object of the prefab, or its `Transform` | `{ "$ref": { "node": "<path>" } }` |
| A listed component | `{ "$ref": { "component": <index in components> } }` |
| Another component of a game object of the prefab | `{ "$ref": { "node": "<path>", "class": "<class name>" } }` |

## 3. Sources

The components of the game objects of the prefab, read like the model file. A component's class decides whether the asset form lists it.

| Case | Result |
|---|---|
| A listed component refers to an object that is neither a game object nor a component of the prefab | Error `unsupported` |
| A listed component holds bytes, or a string that is not UTF-8 | Error `unsupported` |
| A component of the prefab could not be read | The error of its object |

## 4. Container

UTF-8 JSON without insignificant whitespace, with keys in the order of the tables above and fields in stored order.

## 5. Correctness

Fields are written as stored, and every reference names the same object as before, by a path that the model file resolves to the same node. Continuous integration builds prefabs with springs, colliders and constraints from synthetic objects, and checks the order of the components, each kind of reference, and the cases above. On real bundles, the components of each document are compared with an independent export of the same prefabs.

## 6. Version

Format version: 1. A change to any of the above changes the version of every asset form that uses the format.
