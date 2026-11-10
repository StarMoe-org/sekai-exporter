# Field camera

[中文](field-camera.zh-CN.md)

The `field-camera` form is the camera that follows the player in MySekai, as one JSON document. Converter: `field-camera`.

## 1. Runtime

A runtime that reproduces MySekai places its camera from the projection and the settings below, and evaluates the curves over the camera's zoom parameter.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document holds:

| Key | Value |
|---|---|
| `view` | `nearClipPlane` and `farClipPlane`, numbers, and `orthographic`, a boolean |
| `setting` | `offset`, a list of x, y and z, and the numbers `distance`, `minDistance`, `maxDistance`, `initYaw`, `initPitch`, `minPitch`, `maxPitch`, `rotSensitivity` and `fov` |
| `cameraParam` | The curves `_distance`, `_yaw`, `_pitch`, `_movePathSpeed`, `_movePathHigh`, `_moveCameraXPath` and `_moveCameraZPath` |

A curve is a list of keys in stored order, and can be empty. A key holds the numbers `time`, `value`, `inSlope`, `outSlope`, `weightedMode`, `inWeight` and `outWeight`. A slope of a stepped segment is the string `Infinity` or `-Infinity`.

## 3. Sources

The `GameObject` named by a container path ending in `/mysekai/camera/fieldcamera.prefab`. Its `MonoBehaviour` of the script `Sekai.Mysekai.FieldCamera` refers to the `Camera` as `_view`, and to the `MonoBehaviour`s of the settings and the curves as `_setting` and `_cameraParam`. The two planes and the mode are the fields `near clip plane`, `far clip plane` and `orthographic` of the `Camera`. The keys of a curve are its `m_Curve`.

**Outcomes other than a document:** a game object without that behaviour, a null reference, or a field that is absent or has another type makes the asset an error with the code `malformed`.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters. The asset takes the name of its container path.

## 5. Correctness

Tests on synthetic objects cover the joining of the three objects, an empty curve and a game object without the behaviour. The asset of a real version must equal the stored values.

## 6. Version

Form version 1.
