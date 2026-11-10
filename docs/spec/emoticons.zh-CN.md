# 表情

[English](emoticons.md)

头顶道具是显示在角色上方的小型效果：一棵通过三个 clip 绘制精灵的节点树，或一组粒子发射器。`emoticon` 形式是一个道具，写成 JSON 文档。`emoticons` 形式把一个版本的每个道具合并成一份文档。转换器：`emoticon` 与 `emoticons`。

## 1. 运行时

复现 MySekai 的运行时按名称显示一个道具。对于精灵道具，它播放 `start` clip，反复播放 `loop` clip 直到收到停止指令，设置动画器标志 `loopEndFlag`，并播放 `end` clip。对于粒子道具，它在视图所指的锚点上模拟每个发射器。它在加载时读取合并后的文档一次，并获取文档所指的图像文件。

## 2. 内容

两种形式都有一个成员：

| 角色 | 内容 |
|---|---|
| `document` | JSON 文档，扩展名 `json` |

`emoticons` 的文档含有 `items`，后者以每个道具的包名称的最后一段为键，以及 `summary`，其中 `items` 为道具数量，`unsupported` 为各道具 `unsupported` 数组的全部条目。

`emoticon` 的文档含有：

| 键 | 值 |
|---|---|
| `view` | `kind`，取 `sprite` 或 `particle`；`soundInput`，道具所播放声音的参数；对于粒子道具，`anchor`，取 `Face`、`Spine` 或 `Hips`，以及 `keepPosition`，即效果是否停留在其生成处 |
| `animator` | 仅用于带有动画器的道具。`loopEndFlag`，结束循环的标志名称 |
| `textures` | 道具绘制的每张图像，无论存放于何处，按名称顺序排列。`name`、`file`、`width`、`height`。`file` 为 `mysekai/effect/emoticon/` 目录之下 `texture` 形式图像的路径 |
| `sprites` | 以精灵名称为键的对象。`texture`，`textures` 中某一项的名称；`rect`，以像素计的 `[x, y, width, height]`；`pivot`，以矩形为分数表示的 `[x, y]`；`pixelsToUnits` |
| `nodes` | 预制体的节点，父节点在前，同级节点按 path ID 顺序排列 |
| `clips` | 只播放 `start`、`loop` 与 `end` 这三个键。`rate`，帧每秒；`duration`，秒；`channels` |
| `particles` | 每个发射的节点一项，按节点路径顺序排列。`node`、`system`、`renderer` |
| `unsupported` | 文档所略去的内容，每项一个对象，各带有 `reason` |

一个节点含有 `path`，即预制体根之下的路径，根为空字符串；`parent`，其父节点的路径，根为 `null`；`active`；`position` 与 `scale`，`[x, y, z]`；`rotation`，四元数 `[x, y, z, w]`；以及 `animationPath`，即该节点之下、动画器所用的路径，也就是通道所命名者，位于动画器之外的节点为 `null`。带有精灵渲染器的节点还含有 `sprite`，即 `sprites` 的一个键或 `null`；`sortingOrder`；`color`，`[r, g, b, a]`；`flipX`；`flipY`；`rendererEnabled`；以及 `material`。

一个材质含有 `textures`，即每个带有图像的贴图槽所用图像的 `file`；`textureScaleOffset`，即每个贴图槽的 `[scale x, scale y, offset x, offset y]`；以及 `floats`，即材质所具有的 `_BaseMapMode`、`_BaseMapRotation`、`_BaseMapRotationCoord`、`_BaseMapRotationEnabled`、`_Cull`、`_SoftParticlesEnabled`、`_ZOffset`、`_ZTest` 与 `_ZWrite` 属性。

一个通道含有 `path`，即某个节点的 `animationPath`，没有匹配的节点时为 `null`；`property`，取 `position`、`rotation`、`scale` 或 `eulerAngles`；以及 `values`，每帧一个数组，含该属性的各分量。

`system` 含有 `duration`、`looping`、`simulationSpeed`、`simulationSpace`（`Local`、`World` 或 `Custom`）、`scalingMode`、`maxParticles` 与 `start`，并为每个启用的模块一个键：`emission`、`shape`、`sizeOverLifetime`、`rotationOverLifetime`、`velocityOverLifetime`、`limitVelocity`、`textureSheet`、`colorOverLifetime` 与 `subEmitters`。模块的键即第 3 节表中各字段。

可以变化的值为一个对象，其 `mode` 选择其余各键：

| `mode` | 键 |
|---|---|
| `constant` | `value` |
| `twoConstants` | `min`、`max` |
| `curve` | `multiplier`、`keys`、`preInfinity`、`postInfinity` |
| `twoCurves` | `multiplier`、`minKeys`、`minPreInfinity`、`minPostInfinity`、`maxKeys`、`maxPreInfinity`、`maxPostInfinity` |

曲线键含有 `time`、`value`、`inSlope`、`outSlope`、`weightedMode`、`inWeight` 与 `outWeight`。为无限的斜率为字符串 `Infinity` 或 `-Infinity`。颜色为一个对象，其 `mode` 为 `color`（`color`）、`twoColors`（`min`、`max`）、`gradient` 或 `randomColor`（`gradient`），或 `twoGradients`（`minGradient`、`maxGradient`）。渐变含有 `colorKeys`，各为一个 `time` 与一个 `[r, g, b]` 的 `color`，以及 `alphaKeys`，各为一个 `time` 与一个 `alpha`，时间从 0 到 1。

子发射器含有 `emitter`，即发射节点的路径，不在包内时为 `null`；`type`，取 `birth`、`collision`、`death`、`trigger` 或 `manual`；`inherit`，标志 `color`、`size`、`rotation`、`lifetime` 与 `duration`；以及 `emitProbability`。

`renderer` 含有 `enabled`、`renderMode`、`normalDirection`、`flip`、`pivot`、`allowRoll`、`alignment`、`minParticleSize`、`maxParticleSize`、`lengthScale`、`velocityScale`、`cameraVelocityScale`、`freeformStretching` 与 `material`（一个材质或 `null`）。

## 3. 来源

**`emoticon` 的对象：** 一个 `GameObject`，列在 `mysekai/effect/emoticon/` 之下的某个包的容器中。道具即其引用所到达的内容：其下的变换，以及这些节点上的 `AvatarEmoticonSpriteView` 或 `AvatarEmoticonParticleView` 脚本、`Animator`、`SpriteRenderer`、`ParticleSystem` 与 `ParticleSystemRenderer` 组件，连同动画器的控制器中 `m_AnimationClips` 所列的 clip。任何其他组件都是带有 `type` 的 `unsupported` 条目。

clip 以其名称中以 `_` 分隔的最后一部分命名，前提是该部分是一种阶段。通道按路径与属性把 clip 的 `Transform` 曲线分组。时长为 `d`、帧率为 `r` 的 clip 有 `max(2, round(d × r) + 1)` 帧，第 `k` 帧在时刻 `k / r` 以 32 位运算求值；没有曲线的分量为 0。其他任何类的曲线都是带有 `attribute` 的 `unsupported` 条目，每个 clip 的每个属性各一条。`animationPath` 的值按路径的 CRC-32 与曲线匹配。

粒子系统的字段为：

| 键 | 所存字段 |
|---|---|
| `duration` | `lengthInSec` |
| `simulationSpace` | `moveWithTransform` |
| `maxParticles` | `InitialModule.maxNumParticles` |
| `start` | `InitialModule`：`startLifetime`、`startSpeed`、`startSize`、`startRotation`、`startColor`、`gravityModifier`、`size3D`、`rotation3D`；带 `size3D` 时还有 `startSizeY` 与 `startSizeZ`；带 `rotation3D` 时还有 `startRotationX` 与 `startRotationY` |
| `emission` | `EmissionModule`：`rateOverTime`，以及取自 `m_Bursts` 的 `bursts`，各含 `time`、取自 `countCurve` 的 `count`、`cycleCount`、`repeatInterval`、`probability` |
| `shape` | `ShapeModule`：`type`、`radius.value`、`radiusThickness`、`arc.value`、`angle`、`m_Position`、`m_Rotation`、`m_Scale` |
| `sizeOverLifetime` | `SizeModule`：`separateAxes`、`curve`；带 `separateAxes` 时还有 `y` 与 `z` |
| `rotationOverLifetime` | `RotationModule`：`separateAxes`、`curve`，即 z 轴；带 `separateAxes` 时还有 `x` 与 `y` |
| `velocityOverLifetime` | `VelocityModule`：`x`、`y`、`z`、`speedModifier` |
| `limitVelocity` | `ClampVelocityModule`：`separateAxis`、`magnitude`、`dampen`、`drag`、取自 `multiplyDragByParticleSize` 的 `multiplyDragBySize`、取自 `multiplyDragByParticleVelocity` 的 `multiplyDragByVelocity` |
| `textureSheet` | `UVModule`：`mode`、`tilesX`、`tilesY`、`animationType`、`timeMode`、`fps`、`cycles`、`rowIndex`、`rowMode`、`startFrame`、`frameOverTime`、`speedRange`、`uvChannelMask`、`flipU`、`flipV` |
| `colorOverLifetime` | `ColorModule.gradient` |

任何其他启用的模块都是带有 `module` 与 `node` 的 `unsupported` 条目。渲染器的键取自同名字段加前缀 `m_`，`alignment` 例外，取自 `m_RenderAlignment`。

**`emoticons` 的资产：** `mysekai/effect/emoticon/` 之下每个包的每个 `emoticon` 资产。没有任何此类资产的版本没有 `emoticons` 资产。

## 4. 容器

UTF-8 JSON，不带字节顺序标记，紧凑。所存的数按其存储原样写出。通道值为 32 位浮点数，写为能读回它们的最短十进制形式。

## 5. 正确性

运行时由这些键构建每个道具，除此之外不需要包中的任何内容。形式未建模的启用模块或组件按名称列出，因此缺失可见，而非悄无声息。针对合成对象的单元测试检查值的解码、模块的启用判断、子发射器、clip 采样以及每一个 `unsupported` 条目。真实版本与对同一批包的独立读取逐键比对。

## 6. 版本

`emoticon` 1，`emoticons` 1。
