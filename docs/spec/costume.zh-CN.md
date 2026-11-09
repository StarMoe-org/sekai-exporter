# 服装

[English](costume.md)

游戏的 3D 服装界面为角色装配所用的部件，以及它在这些部件上播放的动作。转换器：`sekai-moesekai` 中的 `costume-model` 与 `costume-motion`（feature `costume`）。

## 1. 运行时

Moesekai 的 3D 服装查看器，以游戏服装界面的方式展示角色穿着服装的样子。对一种造型，它加载身体模型、脸部模型，可选地加载另一套服装的脸部模型以取其头发（`Hair` 与 `Acc` 网格），以及饰品模型；按骨骼名称把它们合并到身体上，用所选颜色的颜色变体贴图为其重新着色，运行其弹簧与约束，并播放角色的服装界面动作：先播放一次 `motion` 与 `face`，然后反复播放 `motion_loop` 与 `face_loop`。

## 2. 内容

### costume-model

服装的一个部件：身体、带头发的脸部，或头部饰品，即游戏所实例化的预制体。

| 角色 | 内容 |
|---|---|
| `model` | 预制体，即[模型文件](model.zh-CN.md) |
| `components` | 其弹簧、碰撞体与约束，即[组件文档](component-documents.zh-CN.md)。预制体没有时不存在 |
| `texture/<n>` | 模型文件的第 `n` 个图像，从 0 开始计数，形式与[贴图形式](texture.md)的 `image` 成员相同。角色按 `n` 的顺序出现 |

组件文档列出以下类的组件：

| 类 | 脚本 |
|---|---|
| `MonoBehaviour` | `ThirdParty.SpringBone/UTJ.SpringManager` |
| `MonoBehaviour` | `Assembly-CSharp/Sekai.SekaiSpringBone` |
| `MonoBehaviour` | `ThirdParty.SpringBone/UTJ.SpringSphereCollider` |
| `MonoBehaviour` | `ThirdParty.SpringBone/UTJ.SpringCapsuleCollider` |
| `AimConstraint`、`ParentConstraint`、`PositionConstraint`、`RotationConstraint` | — |

部件的颜色变体贴图，其名称以 `_C`（颜色）、`_S`（阴影）与 `_H`（数值，按线性读取）结尾，是其自身资源包中[贴图形式](texture.md)的资产。它们替换部件材质中 `_MainTex`、`_ShadowTex` 与 `_ValueTex` 属性的贴图。

### costume-motion

服装界面中的一个动作。

| 角色 | 内容 |
|---|---|
| `motion` | clip，即[clip 文档](clip-documents.zh-CN.md) |

其 `Transform` 曲线按节点路径的 CRC-32 绑定到身体模型的节点，该路径的写法与[组件文档](component-documents.zh-CN.md)相同。其混合形状曲线按通道名称的 CRC-32 绑定到脸部模型的混合形状通道；它们的路径即脸部模型中带有渲染器的那个节点的路径。

## 3. 来源

### costume-model

根是一个 `GameObject`，其容器路径（比较时不区分大小写）形式如下：

| 部件 | 容器路径 |
|---|---|
| 身体 | `…/live_pv/model/character/body/…/body.prefab` 或 `…/live_pv/model/characterv2/body/…/body.prefab` |
| 脸部与头发 | `…/live_pv/model/character/face/…/face.prefab` 或 `…/live_pv/model/characterv2/face/…/face.prefab` |
| 头部饰品 | `…/live_pv/model/character/head_optional/…/optional.prefab` 或 `…/live_pv/model/characterv2/head_optional/…/optional.prefab` |

这些资源包中的其他预制体，以及预制体由其制作而来的导入模型文件（`.fbx`），都不是根。模型文件与组件文档依照各自页面所述从根读取，二者除资产以外的结果也分别依照两页所述。

### costume-motion

根是一个 `AnimationClip`，其容器路径（比较时不区分大小写）形式为 `…/character/motion/costume_setting/…/<name>.anim`。其除资产以外的结果，依照[clip 文档](clip-documents.zh-CN.md)。

## 4. 容器

依照模型文件、组件文档、贴图形式与 clip 文档所述。

## 5. 正确性

在每类部件的真实资源包上，模型文件通过 Khronos glTF Validator 检查且无错误；其节点、访问器与材质，以及组件文档，都与同一预制体的独立导出结果逐值比较，每处差异均予以说明。服装界面中每个动作的每条曲线，都在查看器时钟的每一帧求值，并与对同一 clip 的独立解码逐位比较。同一批资源包在单线程或多线程下、在启用或不启用[性能](../performance.zh-CN.md)所述优化的情况下，以及在原生或 `wasm32` 上，给出的字节都相同。

## 6. 版本

`costume-model`：版本 1。`costume-motion`：版本 1。上述任何内容的改动，或其所用格式的改动，都会改变版本。
