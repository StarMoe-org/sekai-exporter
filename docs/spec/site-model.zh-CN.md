# 场地模型

[English](site-model.md)

`site-model` 形式是 MySekai 场地的一个包在运行时呈现的样子：家园、草原这样的场地，房子的一层，房间的一扇门，一个采集物，以及 `mysekai/site/` 下的其他包。它包含该包的每个预制体及其几何、材质及其值，以及运行时为它读取的碰撞体与导航数据。转换器：`sekai-mysekai` 中的 `site-model`。相关页面：[模型文件](model.zh-CN.md)说明本形式共用的部分，[家具模型](fixture-model.zh-CN.md)说明本形式沿用的节点键、碰撞记录与材质表，[贴图](texture.zh-CN.md)说明每个图像。

## 1. 运行时

复现 MySekai 场地的运行时加载一个包的模型文件，并：

- 按名字找到游戏所加载预制体的场景并显示：场地本身（`home`、`grasslands`……）、家园每个等级的障碍物（`rank<等级>`）、房间的地板与墙（`mdl_static_floor`、`mdl_static_wall`）。先显示的场景是该包的主预制体（3.1）；
- 按网格名找到场地的地面，按节点的碰撞记录和文件旁的碰撞网格找到行走面、相机阻挡面与脚步声面；
- 读取节点的 `extras`，以了解哪些源对象未激活、哪些渲染器被禁用以及它们如何投射阴影，以及每个节点的源身份——其他文档正是以此指称节点；
- 读取每个材质的 `extras`，以构建源作者所指定的材质，并按相对 URI 从文件之外加载每种贴图。

场地在世界中的位置、一个版本有哪些场地，来自游戏数据表，不来自本形式。场地的位置由运行时自行施加：模型文件处于其自身预制体的坐标系中。

## 2. 内容

成员：

| 角色 | 内容 |
|---|---|
| `model` | 该包的所有预制体，作为一个多场景的[模型文件](model.zh-CN.md)，差异见下文 |
| `texture/<n>` | 模型文件的第 `n` 个图像（从 0 计），采用[贴图形式](texture.zh-CN.md)中 `image` 成员的形式。各角色按 `n` 的顺序出现 |
| `collision` | 该包的碰撞文档（2.3），扩展名为 `json` 的 UTF-8 JSON 文档 |

### 2.1 场景

该包的每个预制体（3.1）在文件中各有一个场景，按其容器路径（不区分大小写）的顺序排列，以预制体根游戏对象的名称命名。默认场景（`scene`）是主预制体的场景。每个场景唯一的根节点是其预制体的根，每个预制体的节点排在之前各预制体的节点之后。网格、材质、贴图与图像在各场景之间共用。

同一包中的两个预制体可能根名相同；这时两个场景同名，按路径顺序排列。

### 2.2 节点

节点、其顺序、名称、变换与网格，以及顶点属性，均与[家具模型](fixture-model.zh-CN.md#22-节点)相同。节点的 `extras` 是[模型文件](model.zh-CN.md)为节点写出的键值表（节点的渲染器绘制引擎网格时含 `engineMesh`，见 2.5），再加上家具模型的这些键：

| 键 | 写在 |
|---|---|
| `sourceObject` | 每个节点 |
| `sourceComponentIds` | 每个节点 |
| `sourceChildOrder` | 每个节点 |
| `shadowCastingMode` | 每个有渲染器绘制的节点 |
| `materialed` | 每个带渲染器的节点 |
| `sourceCollision` | 游戏对象带有碰撞体、`NavMeshModifier`、`NavMeshAgent` 或 `NavMeshObstacle` 的每个节点（2.3） |

只有家具才有的键不写：`fixtureViewRoot`、`fixtureCollision`、`roadCellType`，以及碰撞记录中的 `fixtureNavLayer`。

### 2.3 碰撞

节点的 `sourceCollision` 是[家具模型](fixture-model.zh-CN.md#23-碰撞)的节点记录，差异如下：

- 没有 `fixtureNavLayer`；
- `activeInHierarchy` 从节点所在预制体的根算起；
- 每个 `MeshCollider` 都有 `engineMesh` 键，除非碰撞体的网格是引擎网格（2.5），否则为 `null`；
- 网格为空或为引擎网格的 `MeshCollider` 按源中所存记录：其 `mesh` 与 `geometryId` 为 `null`。引擎把这样的碰撞体视为没有表面，运行时也一样。

`collision` 成员即家具模型的碰撞文档：`{ "geometry": [ { "geometryId", "positions", "triangles" }, … ] }`，包含该包所有预制体的碰撞网格，按 `geometryId` 升序，每个只写一次。位置处于运行时坐标系，三角形的绕序见第 4 节。

哪个碰撞体是行走面、相机阻挡面或脚步声面，不是一个键：游戏按其游戏对象的名称（`…_nav_ground`、`…_nav_cam`、`…_nav_wall`、`…_footse`）区分，运行时也一样。

### 2.4 材质

材质按[家具模型](fixture-model.zh-CN.md#24-材质)的写法写出：2.4.1 的字段、2.4.2 的 `material.extras.unity` 表与 2.4.3 的渲染状态。运行时按 `shader` 选用的程序（`Mysekai/Site/Ground`、`Mysekai/Site/FieldObject`、`Mysekai/Site/Tree`、`Mysekai/Water` 等）属于运行时；本形式对每个材质写出相同的表。

### 2.5 引擎网格

少数游戏对象绘制的是引擎自带的网格，例如引擎的四边形或平面，任何资源包都不含它。网格为此类对象的渲染器在文件中没有网格，其节点带 `extras.engineMesh`：`{ "file": "<引擎资源文件>", "pathId": "<十进制 path id>" }`，即所存的引用。要绘制它的运行时自行生成该引擎网格。引擎的文件为 `Library/unity default resources` 与 `Resources/unity_builtin_extra`。

## 3. 来源

### 3.1 输入

包是 `mysekai/site/` 下的资源包，场地地图（`mysekai/site/sitemap/`）除外。其预制体是容器以如下形式的路径（不区分大小写）所列的游戏对象：`…/resources/ondemand/mysekai/site/<路径>.prefab`；以两个路径列出的预制体取其中顺序在前的那个。预制体所据以制作的导入模型文件（`.fbx`）不是预制体，不写出：游戏只加载预制体。

主预制体是 `<目录>/<名称>.prefab`，其中 `<目录>` 是容纳该包所有预制体的最深目录，`<名称>` 是该目录的最后一段，例如 `…/site/field/home/home.prefab`；包中没有这样的预制体时，取路径顺序中的第一个预制体，例如房子某一层的 `mdl_static_floor`。每个预制体都是本形式的根，只有主预制体产出资产，其余不产出。资产以主预制体的容器路径命名。

模型文件包含每个预制体的 `Transform` 子树、其游戏对象的组件、渲染器所引用的一切（网格、材质、着色器、贴图，可能位于其他资源包中），以及这些子树的碰撞（2.3）。

### 3.2 所读的 Unity 类

即[家具模型](fixture-model.zh-CN.md#32-所读的-unity-类)所读的类，去掉 `FixtureView` 与 `RoadView`。`NavMeshData`、相机、灯光、体积以及场地的各种行为（`SiteObjectView`、`SiteEnvironmentConfig` 等）不为本形式读取。碰撞体的网格可以是引擎网格（2.5）。

### 3.3 结果

| 情况 | 结果 |
|---|---|
| 本页所列字段缺失、类型不符，或未解析到其所指类的对象 | 错误 `malformed` |
| 本形式写出的某个存储数值不是有限数 | 错误 `malformed`（同模型文件） |
| 本形式写出的某个属性无法表达 | 错误 `malformed` |
| 碰撞三角形的索引超出其位置，或碰撞体的网格没有三角形 | 错误 `malformed` |
| 碰撞体的网格解析到的对象不是 `Mesh` | 错误 `malformed` |
| 预制体所引用的对象不在任何资源包中，且不是引擎对象（2.5） | 错误 `missing_object` |
| 节点的网格数据所用格式、分量数、拓扑或顶点通道不在[模型文件](model.zh-CN.md)所列之内 | 模型文件的 `unsupported` |
| 贴图无法解码 | [贴图形式](texture.zh-CN.md)的错误 |

## 4. 容器

即[家具模型](fixture-model.zh-CN.md#4-容器)的容器：带一个二进制块的 GLB；图像位于文件之外，为 `texture/<n>.webp`，不由任何扩展声明；`KHR_texture_transform` 只用于带平铺或偏移的基础色贴图；纹理坐标不翻转；坐标基变换同模型文件，对各场景与碰撞一并施加。文件含 2.1 的各场景。

**版本。** 格式版本：1。

## 5. 正确性

文件对运行时正确，当且仅当以下各项成立：

1. 该包的每个预制体各有一个场景，以其根命名，按路径顺序排列；默认场景是主预制体的场景；每个节点恰在一个场景中。
2. 每个节点带 `sourceObject`，含其所据以写出的源对象的 id，且没有两个节点的 `(file, gameObjectId, transformId)` 相同。
3. 碰撞体所指的每个 `geometryId` 都是 `collision` 成员中的一个几何，且每个几何都被某个碰撞体所指。
4. 每个材质都有家具模型的各键；`textures` 中的每个贴图索引都指向一个贴图，每个贴图都指向一个属于该资产成员的图像。
5. 文件没有 `animations`；文件中的任何对象都不带版本字段、坐标约定，或单位、UV 约定字符串。

**检查。**

- 持续集成用合成对象构建包，检查各场景、其顺序、名称与默认场景、节点键、碰撞文档、没有网格的碰撞体以及引擎网格。
- 在一个完整版本上：每个包都能转换；每个文件通过 Khronos glTF Validator，除未声明 WebP 图像（第 4 节）所致的错误外没有错误；运行时按名字查找的场景与网格都存在；同一批包在单线程与多线程下产出相同的字节。
- 同一版本的家具模型，在节点键、碰撞记录与材质表与本形式共用前后逐字节相同。

## 6. 版本

形式版本：1。上述任何一项，或其所用格式（模型文件、贴图、碰撞记录、材质表）的改变，都会改变本形式每个资产的版本。
