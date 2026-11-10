# 家具模型

[English](fixture-model.md)

`fixture-model` 形式是一件 MySekai 家具在住居站点上如运行时呈现的样子：其几何、其材质及其值，以及站点为它读取的碰撞体与导航数据。转换器：`sekai-mysekai` 中的 `fixture-model`。相关页面：[模型文件](model.zh-CN.md)说明本形式共用的部分，[贴图](texture.zh-CN.md)说明每个图像。

## 1. 运行时

复现 MySekai 住居站点的运行时，以及它的家具编辑器，从模型文件加载每件已放置的家具，按该家具的放置姿态进入其唯一的场景，并：

- 读取场景各节点的 `extras`，以了解哪些源对象未激活、哪些渲染器被禁用以及它们如何投射阴影、每个节点的源身份（机关、时间轴、门或粒子效果正是以此找到它所指的节点），以及每个节点的碰撞记录；
- 读取每个材质的 `extras`，以构建源作者所指定的材质，并按相对 URI 从文件之外加载每种贴图；
- 读取场景根上的碰撞文档，并从中解析每个碰撞体、导航修饰器与导航障碍。

它还读取角色使用该家具时所站的位置，即挂点（2.6）。它一次读取一个版本全部家具的挂点，来源是合并后的[家具挂点](fixture-attach-points.zh-CN.md)文档，这样在加载任何模型之前就能规划角色的去处。除该文档外，它通过本形式不读取文件之外的任何其他内容：碰撞文档与全部渲染器状态都在 GLB 之内，其旁仅有它的图像。哪些家具有模型、每件使用哪个文件，它从已发布集合的资产列表中获知。

## 2. 内容

成员：

| 角色 | 内容 |
|---|---|
| `model` | 该家具，作为一个[模型文件](model.zh-CN.md)，带有下文所述的差异 |
| `texture/<n>` | 模型文件的第 `n` 个图像，从 0 开始计数，形式与[贴图形式](texture.zh-CN.md)的 `image` 成员相同。角色按 `n` 的顺序出现 |
| `attach-points` | 该家具的挂点（2.6），UTF-8 JSON 文档，扩展名 `json` |

只有预制体带有 `FixtureView` 的家具才有 `fixture-model` 资产（3.1）。

### 2.1 场景

文件有一个场景，它也是默认场景（`scene` 为 0）。场景根是 `extras.fixtureViewRoot` 设为 `true` 的节点，其 `Transform` 子树即文件的每个节点。没有节点在场景之外。

游戏放置一件家具时，找到唯一带有 `FixtureView` 的 GameObject，把该 GameObject 的位置与旋转设为放置姿态，并保持其缩放。该 GameObject 就是预制体根，因此：

- 根在 GLB 中的局部 `translation` 与 `rotation` 为零与单位值，其局部 `scale` 为作者所设的缩放；
- 在预制体中没有父级的 `Transform`，是根 `Transform` 的子节点。

根的名称是该 GameObject 的 `m_Name`。同级节点可以同名（若干关节都叫 `root`，动作点叫 `loc_start013` 等等），因此名称不是身份：节点的身份是它的 `sourceObject`（2.2）。

### 2.2 节点

节点遵循[模型文件](model.zh-CN.md)：先是根，然后是其 `Transform` 子树，按深度优先顺序排列，每个节点的子节点按 `Transform` 存储它们的顺序排列。节点具有其游戏对象的名称、其 `Transform` 的局部位置、旋转与缩放，在有渲染器时还有其网格、其蒙皮与其材质，写法与模型文件相同。顶点属性为 `POSITION`、`NORMAL`、`TANGENT`、`COLOR_0`（四个归一化字节）以及 `TEXCOORD_0` 至 `TEXCOORD_3`，采用源自身的分量；材质数多于子网格数的渲染器，会用每个多出的材质再次绘制其最后一个子网格。

节点的 `extras` 是[模型文件](model.zh-CN.md)为节点书写的键值表，加上这些键。读取方会读作默认值的键不写出（`active` 仅在为 `false` 时，`enabled` 仅在为 `false` 时）。

| 键 | 值 | 写在 |
|---|---|---|
| `fixtureViewRoot` | `true` | 根：其 `FixtureView` 是游戏所放置的实例根的 GameObject |
| `sourceObject` | `{ "file": <序列化文件名>, "gameObjectId": "<带符号的 64 位 id>", "transformId": "<带符号的 64 位 id>" }`。两个 id 都是十进制字符串，因为这些值超出 JSON 数字能精确表示的范围 | 每个节点 |
| `sourceComponentIds` | 节点游戏对象上每个组件的源组件 id 的十进制字符串，按该游戏对象存储它们的顺序排列 | 每个节点 |
| `sourceChildOrder` | 每个子节点变换 id 的十进制字符串，按 `Transform` 存储它们的顺序排列 | 每个节点 |
| `shadowCastingMode` | 节点渲染器的 `Renderer.shadowCastingMode`（0 为 `Off`，1 为 `On`，2 为 `TwoSided`，3 为 `ShadowsOnly`） | 渲染器所绘制的每个节点 |
| `materialed` | 节点渲染器以材质绘制时为 `true`；其材质槽为空时为 `false`。节点没有渲染器时不出现 | 带有渲染器的每个节点 |
| `roadCellType` | 节点道路格的道路格类型，一个数 | 游戏对象为 `RoadView` 的格的节点 |
| `sourceCollision` | 节点的碰撞记录（2.3） | 游戏对象带有碰撞体、`NavMeshModifier`、`NavMeshAgent` 或 `NavMeshObstacle` 的每个节点 |
| `fixtureCollision` | 碰撞文档（2.3） | 仅根 |

`sourceObject` 存在于每个节点，包括不是放置根的节点：该家具中命名某个节点的每份其他文档都以这一身份命名它（7），读取方通过遍历场景并比较 `sourceObject` 找到该节点。

### 2.3 碰撞

碰撞文档是根节点 `extras` 中的 `fixtureCollision` 键（2.2）。它保存两类数据：碰撞体所用的碰撞网格，以及每个节点对其碰撞体与导航组件的记录。

**文档。** 一个 JSON 对象：

| 键 | 值 |
|---|---|
| `geometry` | 各碰撞网格，按 `geometryId` 升序排列。每个为 `{ "geometryId", "positions": [[x, y, z], …], "triangles": [[a, b, c], …] }`。位置处于运行时的坐标系，三角形采用第 4 节的绕向 |

`geometryId` 为 `<序列化文件名>:<源网格的十进制路径 id>`。若干碰撞体所用的同一源网格只写一次。

**节点记录。** 游戏的导航构建为该节点读取的每个节点上，其 `extras.sourceCollision`：

| 键 | 值 |
|---|---|
| `gameObjectId`、`transformId` | 与该节点 `sourceObject` 相同的十进制 id |
| `activeSelf` | 游戏对象是否激活（`m_IsActive`） |
| `activeInHierarchy` | 游戏对象及其所有祖先是否都激活 |
| `authoredLayer` | 游戏对象的层（`m_Layer`） |
| `fixtureNavLayer` | 节点为 `FixtureView` 游戏对象或其之下时为 `true` |
| `colliders` | 节点的碰撞体，按组件顺序排列（见下文） |
| `geometryIds` | 其碰撞体各不相同的 `geometryId`，已排序 |
| `modifiers` | 其 `NavMeshModifier` 组件：`{ "componentId", "enabled", "overrideArea", "area", "ignoreFromBuild", "affectedAgents" }` |
| `hasNavMeshAgent` | 节点带有 `NavMeshAgent` 时为 `true` |
| `navMeshAgents` | 其 `NavMeshAgent` 组件：`{ "componentId", "enabled", "agentTypeId", "radius", "height", "baseOffset" }` |
| `navMeshObstacles` | 其 `NavMeshObstacle` 组件：`{ "componentId", "enabled", "shape", "center", "extents", "carve", "onlyStationary", "moveThreshold", "stationaryTime" }`。`shape` 为 `0`（胶囊）或 `1`（盒）；`center` 处于运行时的坐标系，`extents` 为长度 |

碰撞体为 `{ "componentId", "kind", "enabled", "isTrigger", "includeLayers", "excludeLayers", "layerOverridePriority", "providesContacts", "convex", "cookingOptions", "mesh", "geometryId", "center", "size", "radius", "height", "direction" }`。`kind` 为 `MeshCollider`、`BoxCollider`、`SphereCollider` 或 `CapsuleCollider`。对于 `MeshCollider`，`mesh` 为源网格的 `{ "fileId", "pathId" }`，`geometryId` 命名其几何，`convex` 为 `m_Convex`，`cookingOptions` 为 `m_CookingOptions`。对于盒，设置有 `center` 与 `size`；对于球，设置有 `center` 与 `radius`；对于胶囊，设置有 `center`、`radius`、`height` 与 `direction`。某种类不使用的键为 `null`；源所用的引擎版本不存储 `includeLayers`、`excludeLayers`（层掩码的 `m_Bits`）、`layerOverridePriority` 与 `providesContacts` 时，它们也为 `null`。`center` 处于运行时的坐标系；`size`、`radius` 与 `height` 为长度。

读取方要求这些键：每个碰撞体的 `enabled` 与 `isTrigger`；每个盒、球与胶囊的 `center`；盒的 `size`；球与胶囊的 `radius`；胶囊的 `height` 与 `direction`（`0` 为 x，`1` 为 y，`2` 为 z）；网格碰撞体的 `convex` 与 `geometryId`；每条记录的 `activeInHierarchy`；每个障碍的 `enabled`、`shape`、`center`、`extents` 与 `carve`；根之下每个节点的每个修饰器的 `affectedAgents`。

**格式。** 形式版本（6）涵盖文档与文件的格式；二者都不带版本字段或约定字符串。该家具的每处碰撞都被解析：形式无法解析的碰撞会使转换失败（3.3）。

### 2.4 材质

材质具有[模型文件](model.zh-CN.md)的字段与 `material.extras.unity` 表，取下文的值，加上 2.4.2 所加的键。本形式为场景渲染器所用的每种材质写一个材质。

**2.4.1 字段。** `name` 为源材质的 `m_Name`；一个文件中的若干材质可以同名。`doubleSided` 为 `false`。`pbrMetallicRoughness` 含有 `metallicFactor` 0、`roughnessFactor` 1 与 `baseColorTexture`——`_MainTex` 的贴图，`_MainTex` 没有时为 `_BaseMap` 的贴图——写法与模型文件相同。该描述供不读取 `extras` 的程序预览；运行时仅凭 `extras` 构建材质，其混合模式为 2.4.4 的程序依据 `floats` 与 `keywords.valid` 所选定的那一种。`alphaMode` 与 `alphaCutoff` 不写出。

**2.4.2 `material.extras.unity`。** 保留模型文件的键：

| 键 | 值 |
|---|---|
| `shader` | 着色器的名称（`m_Shader` 所指 `Shader` 的 `m_ParsedForm.m_Name`），材质没有着色器时为 `null`；这样的材质没有 `shaderTextureDefaults`，也没有 `shaderPasses`，运行时让它保持场景的占位材质 |
| `renderQueue` | `m_CustomRenderQueue`。运行时不读取它；家具程序自行计算各自的队列 |
| `keywords` | `{ "valid": […], "invalid": […] }`，取自 `m_ValidKeywords` 与 `m_InvalidKeywords`，按存储顺序排列 |
| `floats`、`ints`、`colors` | `m_SavedProperties` 的每个已保存属性按名称列出，写法与模型文件相同，不单独区分任何属性 |
| `textures` | `m_SavedProperties` 的每个贴图属性按名称列出：`{ "texture": <index or null>, "scale": [x, y], "offset": [x, y] }`，缩放与偏移按存储原样。`texture` 为 glTF 贴图的索引；属性没有 `Texture2D`，或其图像不是由本形式写出的，为 `null`。与模型文件一样，没有贴图的属性也列出；运行时读取每个属性的缩放与偏移 |

本形式加入同一张表的键：

| 键 | 值 |
|---|---|
| `shaderTextureDefaults` | 对着色器所声明的每个贴图属性，其声明的默认贴图的名称。材质没有着色器时不出现 |
| `shaderPasses` | 着色器的每个通道一行，按子着色器与通道顺序排列：`{ "name", "lightMode", "subShaderIndex", "passIndex", "subShaderTags", "tags", "renderState" }`。`name` 与 `tags` 为该通道状态的 `m_Name` 与 `m_Tags`，`lightMode` 为其 `LightMode` 标签的值（标签名称比较时不区分大小写）或 `null`，`subShaderTags` 为其子着色器的 `m_Tags`，`renderState` 见 2.4.3。材质没有着色器时不出现 |

`fixtureShaderUsage` 不是键：它是 `floats` 中的浮点属性 `_FixtureShaderUsage`，与任何其他属性一样。

**2.4.3 `renderState`。** 每个通道的 `renderState` 是该通道 Unity 的序列化状态：`culling`、`zTest`、`zWrite` 以及带有 `colMask`、`srcBlend`、`destBlend`、`srcBlendAlpha`、`destBlendAlpha`、`blendOp` 与 `blendOpAlpha` 的 `blend`。一个值要么是具名值 `{ "name": "<材质属性>", "default": <着色器为它声明的值：该属性 m_DefValue 的第一个分量>, "val": <编译进通道的值> }`，着色器未声明该属性时没有 `default`；要么是固定值 `{ "name": "<noninit>", "val": <值> }`。具名值为该材质 `floats` 中同名的那一项，材质未存储它时为 `default`；固定值为 `val`。其中任何一项在通道状态中缺失，均为错误（3.3）。

**源布局。** 着色器读取其程序的多层分支（`keywords.valid` 中有 `_ENABLE_MULTI_UV_SCROLL`）的材质，从 `_LayerTex1`、`_LayerTex2` 与 `_LayerTex3` 绘制。它们与其他任何属性一样是 `textures` 中的贴图属性，各有浮点属性 `_UVScrollX` 与 `_UVScrollY` 以及浮点属性 `_ParallaxStrength`；`_LayerTex1` 总是被使用，浮点属性 `_UseLayerTex2` 与 `_UseLayerTex3` 开启其余的。

**2.4.4 程序。** 运行时为下列每个着色器名称各有一个材质对象，并把该对象代替场景的占位材质，应用于材质具有该 `shader` 的各图元：

| `shader` | 运行时材质 |
|---|---|
| `Mysekai/Fixture/Basic`、`Mysekai/Fixture/Rug`、`Mysekai/Fixture/Fence` | 家具材质。它读取 `shader`、`floats`、`colors`、`textures`、`keywords.valid`、`shaderTextureDefaults` 与 `shaderPasses`，并从 `baseColorTexture` 取得其主贴图 |
| 运行时的角色程序 | 角色材质，由同一张表与 `textures` 所保留的贴图构建 |
| `Mysekai/Fixture/Canvas`、`Mysekai/Site/Tree`，以及透明块的程序 | 家具表面材质，由同一张表与 `shaderPasses` 构建 |
| `Mysekai/Fixture/ShadowMesh` | 无材质：其网格被隐藏 |
| `Mysekai/Fixture/Road` | 道路材质，它还读取 `roadCellType`（2.2） |

状态不写出任何颜色（`floats` 中 `_ColorMask` 为 0 且 `_ZWrite` 为 0）的材质不绘制任何内容，其网格被隐藏。属于任何其他程序的材质，或着色器为 `null` 的材质，保持场景的占位材质。

程序为角色程序的材质，按角色的贴图约定绘制，包括其 v 翻转；而家具程序自行翻转 v（4）。对所有这些材质，贴图坐标都不翻转地写出（4）；不同的只是各程序所施的翻转。

### 2.5 动画

本版本的形式不写任何动画：GLB 没有 `animations` 数组，文件只含有该家具的几何、材质与碰撞。家具的机关与时间轴所播放的动画 clip 将由以后的形式携带；本页尚未定义它们。

### 2.6 挂点

挂点是角色使用该家具时所站的位置：角色走到挂点的起点；挂点有终点时，再从起点移到终点。游戏把一件家具的挂点存为其 `FixtureView` 上的一个列表，并按列表中的位置找到挂点。每个挂点有一个起点定位器和一个终点定位器，都是预制体的 GameObject，游戏读取各自的世界位置与旋转。

`attach-points` 成员是一个 JSON 对象：

| 键 | 值 |
|---|---|
| `root` | `{ "sourceObject", "scale": [x, y, z] }`：根节点的 `sourceObject`（2.2），以及模型文件所写的根的局部缩放 |
| `collider` | `FixtureView` 指定为自身碰撞体的网格碰撞体：`{ "sourceObject", "componentId" }`，即持有它的 GameObject 所对应节点的 `sourceObject`，以及其组件 id 的十进制字符串，与它在 `sourceCollision` 中的记录一致（2.3）。`FixtureView` 未指定时为 `null` |
| `points` | 各挂点，按 `FixtureView` 的存储顺序。没有挂点的家具为空列表 |

挂点为 `{ "start", "end" }`。`start` 是一个定位器；`end` 是一个定位器，挂点没有终点定位器时为 `null`。定位器为：

| 键 | 值 |
|---|---|
| `name` | 定位器 GameObject 的 `m_Name` |
| `sourceObject` | 定位器节点的 `sourceObject` |
| `position` | `[x, y, z]`，定位器节点在模型文件中的 `translation` |
| `rotation` | `[x, y, z, w]`，定位器节点在模型文件中的 `rotation` |

定位器是根的子节点（3.3），因此其位置与旋转处于根的坐标系中、尚未施加根的缩放，并采用第 4 节的基变换。放置后的家具把定位器放在：放置姿态 × `root` 的 `scale` × 定位器的 `position` 与 `rotation`。定位器自身的缩放不影响其位置，不写出。

本成员按游戏的存储如实记录挂点，不写由它们推出的任何值。游戏从起点定位器的名字取得挂点的槽位（去掉 `loc_start` 后按 `_` 切分，第二段为槽位号；没有第二段或不是数字时为 0）；这条规则属于运行时，作用于 `name`，名字不符合它不是本形式的错误。存储的挂点中 `IsUsing` 与 `CharacterId` 两个字段是游戏运行时的状态，不写出。

## 3. 来源

### 3.1 输入

根是一个 `GameObject`，其容器路径（比较时不区分大小写）形式如下：

| 资产 | 容器路径 |
|---|---|
| 一件家具的模型 | `…/resources/ondemand/mysekai/fixture/<name>/<name>.prefab` |

根是容器在该路径下列出的 GameObject；`is_root` 选定它。列出该路径的资源包是该家具的模型包，其名称为 `<prefix><name>`（上述路径把 `/` 写作 `__` 并去掉 `.prefab`）。资产的名称是容器路径。该包中的其他对象——预制体由其制作而来的导入模型文件（`.fbx`），以及其他文件夹的预制体——都不是根，本形式不为它们写资产。

`<name>` 是该家具模型包的名称，其目录与其预制体共用它。

预制体根有一个脚本类为 `FixtureView` 的 `MonoBehaviour`，并且只有该预制体根的 GameObject 是根。`FixtureView` 位于其根之下的预制体是错误（3.3），因为放置与碰撞图都定义在预制体根上。

模型文件含有根的 `Transform` 子树、其游戏对象的组件、渲染器所引用的一切（网格、材质、着色器与贴图，它们可能位于其他资源包中），以及同一子树的碰撞图（2.3）。

### 3.2 所读的 Unity 类

| 类 | 读取用于 |
|---|---|
| `GameObject` | `m_Name`、`m_IsActive`、`m_Layer`、`m_Component` |
| `Transform`、`RectTransform` | `m_LocalPosition`、`m_LocalRotation`、`m_LocalScale`、`m_Children`、`m_Father`、`m_GameObject` |
| `MeshFilter`、`MeshRenderer`、`SkinnedMeshRenderer` | 网格、材质、`m_Enabled`、`m_Bones`、`m_RootBone`、`m_ShadowCastingMode` |
| `Mesh` | 顶点数据、子网格及其拓扑、`m_BindPose` |
| `Material` | `m_Name`、`m_Shader`、`m_CustomRenderQueue`、`m_SavedProperties`、`m_ValidKeywords`、`m_InvalidKeywords` |
| `Shader` | `m_ParsedForm`（名称、贴图属性的默认值、通道及其渲染状态） |
| `Texture2D` | 图像、采样设置 |
| 脚本为 `FixtureView` 的 `MonoBehaviour` | 放置根（`fixtureViewRoot`）；挂点所用的 `_attachPoints`（其各项带 `StartLoc` 与 `EndLoc`）与 `_collider`（2.6） |
| 脚本为 `RoadView` 的 `MonoBehaviour` | 道路格类型（`roadCellType`） |
| `MeshCollider`、`BoxCollider`、`SphereCollider`、`CapsuleCollider`、`NavMeshModifier`、`NavMeshAgent`、`NavMeshObstacle` | `sourceCollision` 与碰撞几何 |

组件可以引用同一包集合中另一资源包的对象（材质、着色器、网格或贴图）。该对象从持有它的资源包读取，并写进本文件，因为 GLB 是自包含的。

`FenceView` 不被读取：栅栏家具所显示的各部分在其节点上各自带有 `active`，切换它们的连接法则属于运行时。本页未列出的每个组件都不为本形式读取。

### 3.3 结果

| 情况 | 结果 |
|---|---|
| 本页所命名的字段缺失、类型不符，或未解析为它所命名类的对象 | 错误 `malformed` |
| 本形式写出的某个所存数不是有限数（变换、顶点、变形目标偏移、绑定姿势、碰撞位置、`floats`、`ints` 或 `colors` 中的属性） | 错误 `malformed`（与模型文件相同） |
| 本形式写出的某个属性无法表示（形状不符的 `ints` 或 `colors`，重复的所存名称） | 错误 `malformed` |
| 碰撞三角形的某个索引在其位置之外，或碰撞体的网格没有三角形 | 错误 `malformed` |
| 材质的着色器无法解析 | 缺失对象的错误 |
| 碰撞体的网格无法解析，或不是 `Mesh` | 错误 `malformed` |
| 挂点的 `StartLoc` 为空，或定位器、`_collider` 未解析到预制体中所指类的对象 | 错误 `malformed` |
| 定位器的 GameObject 不是根的子节点 | 错误 `unsupported`：其姿态需经二者之间的各变换合成 |
| 根的预制体没有 `FixtureView` GameObject | 错误 `malformed`（本形式对这样的包没有资产） |
| 节点的网格数据使用了[模型文件](model.zh-CN.md)未列出的格式、分量数、拓扑或顶点通道 | 模型文件的 `unsupported` |
| 某个贴图无法解码 | [贴图形式](texture.zh-CN.md)的错误 |

上述错误遵循导出器的错误模型：转换在第一个未给出本形式所需值的对象处停止。

## 4. 容器

GLB：带有一个二进制块的 glTF 2.0，含有 2.1 的场景、其节点、其网格、材质、贴图与采样器。与[模型文件](model.zh-CN.md)的容器相比的差异为：

- **图像位于文件之外。** `images[].uri` 为 `texture/<n>.webp`，`texture/<n>` 是同一资产中具该角色的成员。每个图像都是 8 位 WebP，行序从图像顶部开始。没有图像被内嵌，也没有图像是 PNG。
- **没有扩展声明 WebP 图像。** 对于这些图像，`extensionsUsed` 与 `extensionsRequired` 中既不列出 `EXT_texture_webp`，也不列出任何其他扩展：`texture.source` 直接命名图像。
- **`KHR_texture_transform` 只用于基础颜色贴图**，且其槽在任一轴上的缩放不为 1，或在任一轴上的偏移不为 0。只有在写出时才列入 `extensionsUsed`。其缩放为该槽的缩放，其偏移为 `(ox, 1 - sy - oy)`，其中该槽的缩放为 `(sx, sy)`、偏移为 `(ox, oy)`。每个其他贴图属性只在 `textures` 中保留其缩放与偏移，由运行时自行施用。
- **贴图坐标不翻转。** `TEXCOORD_n` 为源分量对原样（u, v），其中 v 从图像底部计数，而图像中的行从顶部写出。运行时在采样时翻转 v。这是与模型文件 UV 规则的唯一差异，也是本形式不使用模型文件的翻转的原因。
- **基变换即模型文件的基变换：** 右手系、Y 向上、对 X 取反（`S = diag(-1, 1, 1)`），施用于位置、旋转、法线、切线、三角形、绑定姿势与碰撞几何，使场景与碰撞处于同一空间。
- 内存布局、访问器、索引宽度、关节与权重都与模型文件相同。`POSITION` 访问器带有 `min` 与 `max`。
- `asset.generator` 命名转换器与形式版本。`asset` 不带 `extras`。

**版本。** 格式版本：1。

## 5. 正确性

当下列各项都成立时，文件对运行时是正确的：

1. 它有一个场景，即默认场景，且恰有一个节点带有 `fixtureViewRoot`；该节点是场景唯一的根节点，并且是预制体根。
2. 每个节点带有 `sourceObject`，其值是该节点所写出源对象的 id，且没有两个节点具有相同的 `(file, gameObjectId, transformId)`。
3. 根带有 `fixtureCollision`；碰撞体所命名的每个 `geometryId` 都是该文档中的一个几何；导航构建所读取的每个节点都具有带 2.3 所述键的 `sourceCollision`。
4. 每个材质都有 `shader`、`renderQueue`、`keywords`、`floats`、`ints`、`colors` 与 `textures`，且每个贴图属性都在 `textures` 中；程序由 2.4.4 给出运行时材质的每个材质，都持有该程序的材质代码从 extras 读取的每个键；`textures` 中的每个贴图索引都命名一个贴图，每个贴图都命名一个作为该资产成员的图像；每个图像都是 WebP 且可解码。
5. 文件没有 `animations`；文件中的任何对象都不带版本字段、坐标约定，或单位或 UV 约定字符串。
6. `attach-points` 成员按存储顺序列出 `_attachPoints` 的每一项；其中每个 `sourceObject` 都是文件中某个节点的 `sourceObject`；每个定位器的 `position` 与 `rotation` 等于其节点的 `translation` 与 `rotation`；`root.scale` 等于根节点的 `scale`；`collider` 不为空时，指向该节点 `sourceCollision` 中的一个碰撞体。

**检查。**

- 文件通过 Khronos glTF Validator 检查且无错误。
- 节点身份、变换、extras 键、材质表（extras 与源属性的对照）、碰撞文档与节点记录，都与同一资源包的独立导出结果逐值比较，每处差异均予以说明。
- WebP 图像按解码后的像素与源贴图比较。
- 对抽取的家具有样本，游戏自身导航构建从源读取的每个节点与碰撞体，都通过 `sourceObject` 从本文件重新找到，其形状一致。
- 一个版本每件家具的挂点，与从同一批资源包的对象文档独立读出的其 `FixtureView` 的 `_attachPoints` 比较：数量、顺序、定位器的名字与身份，以及姿态。
- 同一资源包在单线程与多线程下，以及在原生与 Web 目标上，给出的字节都相同。

## 6. 版本

形式版本：2。版本 2 增加了 `attach-points` 成员。上述任何内容的改动，或本形式所用任何格式（模型文件、贴图、碰撞记录、材质表、挂点）的改动，都会改变本形式每种资产的版本。

## 7. 为节点命名

该家具中命名节点的每份文档，都以节点的 `sourceObject` 命名它：`file`、`gameObjectId` 与 `transformId`，即该节点所带（2.2）的同样三个值。同级名称会重复，因此节点的名称并不标识它。

哪些家具有模型、每件使用哪个文件，读取方从已发布集合的资产列表中获知：本形式只为带有 `FixtureView` 的预制体产生资产，因此凡以 `fixture-model` 资产列出的家具都有它。
