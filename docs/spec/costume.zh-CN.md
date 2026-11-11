# 服装

[English](costume.md)

游戏的 3D 服装界面为角色装配所用的部件，以及它在这些部件上播放的动作。转换器：`sekai-moesekai` 中的 `costume-model`、`costume-motion`、`costume-role` 与 `costume`（feature `costume`）。后两个是表转换器：它们由游戏数据表与该版本的资产构建，构建于各资源包的资产之后。

## 1. 运行时

Moesekai 的 3D 服装查看器，以游戏服装界面的方式展示角色穿着服装的样子。对一种造型，它加载身体模型、脸部模型，可选地加载另一套服装的脸部模型以取其头发（`Hair` 与 `Acc` 网格），以及饰品模型；按骨骼名称把它们合并到身体上，用所选颜色的颜色变体贴图为其重新着色，运行其弹簧与约束，并播放角色的服装界面动作：先播放一次 `motion` 与 `face`，然后反复播放 `motion_loop` 与 `face_loop`。

查看器从 `costume-role` 资产列出各角色（每个角色是一个单元中的一个游戏角色）。角色的资产列出该角色可穿戴的部件，以及哪些头部与头发可以搭配。一个 `costume` 资产给出，对一套服装的每个角色与每种颜色，该造型的部件以及展示它们的资产。

## 2. 内容

### costume-model

服装的一个部件：身体、带头发的脸部，或头部饰品，即游戏所实例化的预制体。

| 角色 | 内容 |
|---|---|
| `model` | 预制体，即[模型文件](model.zh-CN.md) |
| `components` | 其弹簧、碰撞体与约束，即[组件文档](component-documents.zh-CN.md)。预制体没有时不存在 |
| `texture/<n>` | 模型文件的第 `n` 个图像，从 0 开始计数，形式与[贴图形式](texture.zh-CN.md)的 `image` 成员相同。成员按 `n` 的顺序出现 |

组件文档列出以下类的组件：

| 类 | 脚本 |
|---|---|
| `MonoBehaviour` | `ThirdParty.SpringBone/UTJ.SpringManager` |
| `MonoBehaviour` | `Assembly-CSharp/Sekai.SekaiSpringBone` |
| `MonoBehaviour` | `ThirdParty.SpringBone/UTJ.SpringSphereCollider` |
| `MonoBehaviour` | `ThirdParty.SpringBone/UTJ.SpringCapsuleCollider` |
| `AimConstraint`、`ParentConstraint`、`PositionConstraint`、`RotationConstraint` | — |

部件的颜色变体贴图，其名称以 `_C`（颜色）、`_S`（阴影）与 `_H`（数值，按线性读取）结尾，是其自身资源包中[贴图形式](texture.zh-CN.md)的资产。它们替换部件材质中 `_MainTex`、`_ShadowTex` 与 `_ValueTex` 属性的贴图。

### costume-motion

服装界面中的一个动作。

| 角色 | 内容 |
|---|---|
| `motion` | clip，即[clip 文档](clip-documents.zh-CN.md) |

其 `Transform` 曲线按节点路径的 CRC-32 绑定到身体模型的节点，该路径的写法与[组件文档](component-documents.zh-CN.md)相同。其混合形状曲线按通道名称的 CRC-32 绑定到脸部模型的混合形状通道；它们的路径即脸部模型中带有渲染器的那个节点的路径。

### costume-role

一个角色：一个单元中的一个游戏角色。资产名为 `costume/role/<characterId>-<unit>`，只有一个成员：

| 成员 | 内容 |
|---|---|
| `costume-role` | JSON 文档，扩展名 `json` |

| 键 | 值 |
|---|---|
| `format`、`version` | `"costume-role"` 与本文档格式的版本，1 |
| `role` | 角色（见下） |
| `parts` | 角色可穿戴的每个部件（见下），按 `costume3dId` 排序 |
| `notAvailable` | 不搭配穿戴的 `[head, hair]` 对，以 `costume3dId` 表示，按序排列 |
| `defaultHairs` | 头部 `costume3dId` → 与其搭配穿戴的头发 `costume3dId`，针对 `parts` 中的头部。键为十进制字符串，按数值排序 |

角色：

| 键 | 值 |
|---|---|
| `characterId`、`unit` | 游戏角色与单元 |
| `figure`、`breastSize` | 游戏角色的体型与胸围尺寸 |
| `skin` | `default`、`shadow1` 与 `shadow2`：肤色代码 |
| `defaultBody`、`defaultHead`、`defaultHair` | 角色默认服装的 `costume3dId` |
| `motions` | clip 名称 → `costume-motion` 资产的名称，针对该角色的动作资源包所持有的 `motion`、`face`、`motion_loop` 与 `face_loop` 中的每一个。该版本中没有动作资源包时不存在 |

部件：

| 键 | 值 |
|---|---|
| `costume3dId` | 部件的 ID |
| `slot` | `body`、`hair` 或 `head` |
| `costumeNumber` | 部件所属的服装。默认服装与单元服装的部件不存在 |
| `colorId` | 颜色 |
| `colorName`、`name` | 颜色的名称与服装的名称。表中没有时不存在 |
| `headType` | 对头部：`head_only`、`head_and_hair`、`head_all`、`head_front` 或 `head_back`。其他情况不存在 |
| `publishedAt` | 部件的发布时间，以 Unix 纪元以来的毫秒计。表中未给出时不存在 |
| `thumbnail` | 部件缩略图的名称。没有时不存在 |
| `model` | 展示该部件的内容（见下）。该版本中该部件没有模型时不存在 |
| `missing` | 无法解析的内容，写作句子。没有缺失时不存在 |

`model` 含有 `asset`，即 `costume-model` 资产的名称：身体取身体，头发或类型为 `head_and_hair` 的头部取带头发的脸部，其他头部取饰品。身体或饰品的颜色有其自身的贴图时，含有 `colors`：`color`、`shadow` 与 `value` → 名称分别以 `_C`、`_S` 与 `_H` 结尾的 `texture` 资产的名称。饰品含有 `attach`，即其所挂靠的脸部 `Head` 节点的子节点名称。

### costume

一套服装。资产名为 `costume/<costumeNumber>`，只有一个成员：

| 成员 | 内容 |
|---|---|
| `costume` | JSON 文档，扩展名 `json` |

| 键 | 值 |
|---|---|
| `format`、`version` | `"costume"` 与本文档格式的版本，1 |
| `costumeNumber` | 服装 |
| `name` | 服装的名称。表中没有时不存在 |
| `variants` | 该服装的每个拥有其部件的角色，各按该服装的每种颜色给出一个造型，按角色、单元与颜色排序 |

造型：

| 键 | 值 |
|---|---|
| `role` | 该角色的 `costume-role` 资产名称 |
| `characterId`、`unit`、`colorId` | 谁的造型，以及所用颜色 |
| `body`、`head`、`hair` | 所穿部件的 `costume3dId` |
| `alternativeHeads` | 该服装在此颜色或颜色 1 下的其他头部的 `costume3dId`，按序排列 |
| `models` | `body`，身体的 `model`，写法同部件；`face`，所穿脸部的 `costume-model` 资产名称；以及 `accessory`，饰品的 `model`，头部类型为 `head_and_hair` 时不存在。模型不在该版本中时，该键不存在 |
| `missing` | 无法解析的内容，写作句子。没有缺失时不存在。有缺失部件的造型，查看器不展示 |

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

### costume-role 与 costume

表 `character3ds`、`costume3ds`、`costume3dModels`、`costume3dModelDefaultHairs`、`costume3dModelNotAvailablePatterns`、`gameCharacters` 与 `gameCharacterUnits`，以及存在时的 `costume3dGroups`。值为 `null` 的字段按不存在读取。`costume3ds` 的某一行没有 `characterId` 时，每一行从组 ID 相同的 `costume3dGroups` 行中取其所缺的 `characterId`、`name` 与 `publishedAt`；补填后仍没有 `characterId` 的行略去。

| 项目 | 规则 |
|---|---|
| 角色 | `character3ds` 中每一对 `characterId` 与 `unit`。`id` 最小的一行给出默认服装 |
| 角色的部件 | `costume3dModels` 中属于该角色的单元、且其 `costume3ds` 行具有该角色的行。`partType` 给出槽 |
| 服装 | `costume3dGroupId` 不小于 1000 的部件属于服装 `costume3dGroupId / 1000`，向下取整 |
| 一套服装的颜色 | 该角色这套服装的身体部件的颜色；其中没有身体时，取该角色这套服装全部部件的颜色 |
| 造型 | 对每种颜色，每个槽依次取该角色这套服装在该颜色下的部件，否则取颜色 1 下的部件，否则取 `costume3dId` 最小者：身体；饰品中的头部，这套服装没有饰品时则取类型为 `head_and_hair` 的头部中的头部；以及头发。没有这套服装部件的槽取该角色的默认身体或头部；头发此时取该头部的默认头发，否则取该角色的默认头发 |
| `notAvailable`、`defaultHairs` | 该角色单元在 `costume3dModelNotAvailablePatterns` 与 `costume3dModelDefaultHairs` 中的行，作为头部与头发 |
| 肤色 | 该游戏角色与该单元的 `gameCharacterUnits` 行，否则取该游戏角色的第一行 |
| 动作资源包 | `character/motion/costume_setting/<id>_00`，`<id>` 为该角色 `gameCharacterUnits` 行的两位 ID，然后是该游戏角色第一行的两位 ID；然后是体型为 `ladies` 时的 `character/motion/costume_setting/ladies`，其他体型为 `mens`。取第一个持有 `costume-motion` 资产者 |

下文的模型资源包，名称前加前缀 `live_pv/model/characterv2/`；该资源包不在该版本中时，加前缀 `live_pv/model/character/`。资源包恰有一个所需形式的资产时，才在该版本中：`costume-model`，或对颜色而言，每个存在的后缀各一个 `texture` 资产。

| 模型 | 前缀后的资源包名称 |
|---|---|
| 身体 | `body/<assetbundleName>/<figure>`，胸围尺寸不为 `none` 时追加 `_<breastSize>` |
| 身体颜色 | `color_variation/body/<assetbundleName>/<figure>/<colorAssetbundleName>` |
| 脸部 | `face/<model><suffix>`，其中 `<model>` 是头发、或类型为 `head_and_hair` 的头部的 `assetbundleName`。所穿头部类型为 `head_all`、`head_front` 或 `head_back`，且模型名称含 `0000` 或 `0001` 时，后缀为 `_<headType>`，否则为空。该资源包不在该版本中时，把模型名称中的 `0000` 换成 `0001` 再试一次 |
| 饰品 | `head_optional/<assetbundleName>`。`attach` 取模型的 `part`，没有时取 `assetbundleName` 的最后一段 |
| 饰品颜色 | `color_variation/head_optional/<assetbundleName>/<colorAssetbundleName>` |

带后缀的脸部不在该版本中时，以不带后缀的方式展示，造型把这一点记入 `missing`。不在该版本中的模型或颜色资源包、没有模型的头发，以及列于 `notAvailable` 的头部与头发对，记入 `missing`，不由其他部件顶替。

**资产以外的结果：** 无法读取的表使运行以代码 `table_unreadable` 失败，无法解析的表以 `malformed` 失败。

## 4. 容器

依照模型文件、组件文档、贴图形式与 clip 文档所述。

## 5. 正确性

在每类部件的真实资源包上，模型文件通过 Khronos glTF Validator 检查且无错误；其节点、访问器与材质，以及组件文档，都与同一预制体的独立导出结果逐值比较，每处差异均予以说明。服装界面中每个动作的每条曲线，都在查看器时钟的每一帧求值，并与对同一 clip 的独立解码逐位比较。同一批资源包在单线程或多线程下、在启用或不启用[性能](../performance.zh-CN.md)所述优化的情况下，以及在原生或 `wasm32` 上，给出的字节都相同。

针对合成表与资产的测试覆盖各角色及其默认值、造型的颜色与部件及其回退、带后缀与不带后缀的脸部、完整头部、带颜色与挂靠点的饰品、动作的回退、紧凑行的补填，以及 `missing` 记录。在每个区服的真实表上，各角色、服装与造型都与对同一批表的独立解析结果比较，每处差异均予以说明。

## 6. 版本

`costume-model`：版本 2，对应[模型文件](model.zh-CN.md)的版本 2。`costume-motion`、`costume-role` 与 `costume`：版本 1。上述任何内容的改动，或其所用格式的改动，都会改变版本。
