# 场地相机

[English](field-camera.md)

`field-camera` 形式是 MySekai 中跟随玩家的相机，写成一份 JSON 文档。转换器：`field-camera`。

## 1. 运行时

复现 MySekai 的运行时依据投影与下述设置放置其相机，并随相机的缩放参数求值各条曲线。

## 2. 内容

一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

文档含有：

| 键 | 值 |
|---|---|
| `view` | `nearClipPlane` 与 `farClipPlane`，为数值，以及 `orthographic`，为布尔值 |
| `setting` | `offset`，为 x、y、z 组成的列表，以及数值 `distance`、`minDistance`、`maxDistance`、`initYaw`、`initPitch`、`minPitch`、`maxPitch`、`rotSensitivity` 与 `fov` |
| `cameraParam` | 曲线 `_distance`、`_yaw`、`_pitch`、`_movePathSpeed`、`_movePathHigh`、`_moveCameraXPath` 与 `_moveCameraZPath` |

曲线为按键的存储顺序排列的键列表，可以为空。一个键含有数值 `time`、`value`、`inSlope`、`outSlope`、`weightedMode`、`inWeight` 与 `outWeight`。阶跃段的斜率为字符串 `Infinity` 或 `-Infinity`。

## 3. 来源

容器路径以 `/mysekai/camera/fieldcamera.prefab` 结尾的 `GameObject`。其脚本为 `Sekai.Mysekai.FieldCamera` 的 `MonoBehaviour` 以 `_view` 引用该 `Camera`，以 `_setting` 与 `_cameraParam` 引用设置与曲线的 `MonoBehaviour`。两个裁剪面与模式为 `Camera` 的字段 `near clip plane`、`far clip plane` 与 `orthographic`。曲线的键为它的 `m_Curve`。

**文档以外的结果：** 没有该行为的游戏对象、空引用，或不存在或类型不符的字段，使资产成为错误码为 `malformed` 的错误。

## 4. 容器

成员是成品字节，即第 2 节所述的 UTF-8 JSON 文件。没有编码参数。资产取其容器路径的名称。

## 5. 正确性

针对合成对象的测试覆盖三个对象的连接、空曲线以及没有该行为的游戏对象。真实版本的资产必须等于所存的值。

## 6. 版本

形式版本 1。
