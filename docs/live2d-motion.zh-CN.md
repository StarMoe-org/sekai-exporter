# Live2D 动作

[English](live2d-motion.md)

Live2D 动作可以保存为两种形式：游戏所带的 Unity 动画 clip，或 Cubism 的 `motion3.json` 文件。本文说明游戏如何保存动作，用读取它们的运行时在真实 clip 上比较这两种形式，并说明哪种形式是规范形式及其理由。

## 游戏保存的是什么

Live2D Cubism 分为两层：

- **Cubism Core** 是一个原生库，各平台相同。它读取 `.moc3` 模型，把参数值算成顶点位置与不透明度，不播放动作。
- **Cubism Framework** 是其上的一层，每个平台一套（原生、Web、Unity 等）。Cubism Editor 在模型之外导出的文件 `motion3.json`、`physics3.json`、`exp3.json` 与 `model3.json` 都是 Framework 的格式，由各平台的 Framework 各自解释。

游戏使用 Unity 版 Framework。模型保持原样：资源包中带有 `.moc3` 文件，Cubism Core 原样读取。动作不以 `motion3.json` 保存，而是 Unity 的 `AnimationClip` 对象。Cubism SDK for Unity 在动作导入编辑器时由 `motion3.json` 生成这类 clip（`CubismMotion3Json.ToAnimationClip`），把每种分段写成 Unity 关键帧：

| `motion3.json` 分段 | Unity 关键帧 |
|---|---|
| 线性 | 两个键，切线等于该段斜率 |
| Bezier | 由控制点的值与段时长的三分之一求出 Hermite 切线；不读控制点的时间 |
| 阶梯 | 一个入切线为无穷大的键，Unity 在下一个键之前保持常值 |
| 反阶梯 | 在起点后 0.01 s 处多插一个键 |
| `Meta.Loop` | `AnimationClip` 的 `loopTime` |

构建游戏时，Unity 把每条曲线的关键帧编译成 `StreamedClip`：相邻两键之间一个三次多项式，以四个系数保存。游戏中的 clip 与这一映射相符：曲线是首尾相接的三次段，阶梯是常值段，键的时间与值都落在 0.001 的网格上。

游戏用 Unity 的动画系统和自己的两层动作播放 clip。身体层与表情层各自对其 clip 做线性交叉淡化，表情层覆盖身体层中它所驱动的参数。淡入淡出时间来自同一资源包中、每个 clip 旁边的 `Live2DBuildMotionMetaData` 对象。Cubism Framework 的动作播放器及其淡入淡出、循环与首尾校正都不参与。

由此有两点结论：

- 游戏表现出的行为就是 clip 的行为。
- 从 clip 反写出的 `motion3.json` 与 clip 等价，但不一定与导入前的文件相同：控制点不在段时长三分之一处的 Bezier 段在导入时已被改形，游戏中只有改形后的曲线。

## 谁读取动作

本仓库的资产默认服务 Moesekai（参见[对象与资产](objects-and-assets.zh-CN.md)）。它的 Live2D 查看器按游戏的方式求值 clip：

- 按 Unity 的存储读取 clip 的曲线，以 Unity 绑定所用的哈希 `crc32("Parameters/<id>")` 把每条曲线绑定到模型参数，并在上述两层上播放。
- 读取 clip 的起止时间、循环标志与事件，只对名为 `OnLive2DInvokeUserData` 的事件（眨眼设置）调用函数。
- 从 clip 的元数据对象读取淡入时间。

它不使用 Cubism Framework 的动作播放器。

## 两种形式

**clip 的曲线。** 文档按存储原样保存 clip 的每条曲线：

- streamed 曲线保存为三次段 `[t, a, b, c, d]`，`value(t′) = ((a·δ + b)·δ + c)·δ + d`，其中 `δ = t′ − t`，取起点不晚于 `t′` 的那一段；
- dense 曲线保存为采样，线性插值；
- constant 曲线保存为一个值。

每条曲线按路径与属性哈希绑定。文档还保存起止时间、循环标志、带函数名的事件，以及元数据对象中的淡入淡出时间。

**`motion3.json`。** 同一 clip 可以按导入映射的逆映射写成 `motion3.json`：

- 三次段写成控制点位于段时长三分之一处的受限 Bezier 段，它描述同一个三次多项式；
- 其后值发生变化的常值段写成阶梯段；
- dense 采样写成线性段。

每条曲线都需要参数或部件标识符，`Meta.Loop` 取自 `loopTime`。事件写成用户数据，保留事件的时间与数据字符串，不保留函数名。

## 比较

比较要回答的问题是：两种形式各由其对应的运行时求值时，是否给出相同的参数值。

- **clip：** 日服七个动作包与十四个模型包中的全部 `AnimationClip`，共 1409 个（动作包中 1380 个，模型包中 29 个），涵盖主要角色、联动角色与配角，两代模型，身体动作与表情。
- **曲线：** 22 822 条 streamed 曲线与 58 211 条 constant 曲线；这些资源包中没有 dense 曲线。
- **时间网格：** 查看器的帧时钟（以 32 位运算累加 1/60 s 的步长，到 clip 末尾后保持），再加相邻两帧的中点，共 21 822 571 个取值。
- **clip 一侧：** 每条曲线按 Unity 的存储以 32 位运算求值，每一步运算与查看器的求值器相同。在查看器能绑定到样本模型的 clip 上，两者的 21 213 185 个取值逐位相同。
- **`motion3.json` 一侧：** 每个 clip 写成 `motion3.json`，由 Cubism Web Framework 的 `evaluateCurve` 求值，即 `CubismMotion` 求值每条曲线所用的函数。

`motion3.json` 有两种写法：每个数精确写出（读回后等于所存 32 位值的十进制数），以及每个数舍入到三位小数。每种写法读取两次：按 Web Framework 的解析读成 64 位数，以及把各点舍入为 32 位数，即原生 Framework 的精度。

| `motion3.json` | 读取精度 | streamed 曲线的最大差 | constant 曲线的最大差 |
|---|---|---|---|
| 精确写出 | 64 位 | 2.1 × 10⁻⁵ | 0 |
| 精确写出 | 32 位 | 1.9 × 10⁻⁵ | 0 |
| 三位小数 | 64 位 | 1.909（一个 clip 的一帧） | 1.7 × 10⁻⁶ |
| 三位小数 | 32 位 | 9.8 × 10⁻⁵ | 0 |

差值以参数单位计；例如头部角度的取值范围是 −30 到 30。帧间中点的差值与帧上相同量级，只有下文所述的相等取舍落在帧上。

这些取值说明：

- **曲线是连续的。** 在每个键上，前一段的三次多项式的终值与下一段的起始值相差不超过 2.2 × 10⁻⁵，因此首尾共点的分段链可以承载每一条曲线。
- **精确写出时，`motion3.json` 给出 clip 的取值**，误差在三次多项式的 32 位舍入以内。
- **三位小数带来的舍入误差小于 10⁻⁴。** 键都落在 0.001 的网格上，被舍入改变的只有由斜率推出的控制点值。
- **唯一的大差值是台阶上的相等取舍。** 它出现在一个每帧都有台阶的 clip 的一帧上。该键的时间是最接近 0.7 的 32 位数，查看器的时钟在第 42 帧正好等于这个数，此时 clip 已进入新台阶。文本 `0.7` 读成 64 位数时略大，因此 Web Framework 仍返回上一个台阶。精确写出或以 32 位读取各点时，不出现这种取舍。

取值之外，样本还显示：

- 没有循环的 clip；
- 每个 clip 都有带淡入淡出时间的元数据对象；
- 全部 896 个事件调用的都是 `OnLive2DInvokeUserData`。

## 决定

Live2D 动作的规范形式是 clip 曲线的文档。

`motion3.json` 能够承载这些 clip 的取值，因此精度不是决定因素。决定因素是以下性质：

- **它就是游戏播放的内容。** 游戏求值的动作是 clip；游戏数据中没有 `motion3.json`。
- **它不需要资源包以外的任何东西。** clip 以哈希绑定曲线，查看器按所显示模型的参数表解析。`motion3.json` 要给每条曲线命名，而名字只存在于模型包的 `.moc3` 文件中，动作包并不依赖模型包。写 `motion3.json` 会使动作包的资产取决于一次运行读过哪些模型包，违背资产由其资源包及其所依赖的资源包构建这一规则（参见[管线](pipeline.zh-CN.md)），而且尚未读到的模型所定义的曲线会被遗漏。
- **它保留运行时所读的内容。** 循环标志、事件的函数名、clip 旁边的淡入淡出时间，以及按存储原样的曲线分段，都是 clip 的一部分。`motion3.json` 或者没有这些内容，或者赋予它们自己的含义：它的用户数据没有函数名，它的淡入淡出与循环属于 Framework 的动作播放器。
- **一种曲线文档服务所有 clip。** 服装动作是 3D 变换，`motion3.json` 无法表达，它们使用同一种文档。一个求值器覆盖两者。

对基于 Cubism Framework 的运行时，可以按上述映射、用所选模型的标识符，从规范文档派生出 `motion3.json`。这样的文件是为该运行时提供的视图，不属于规范层。

## 复现比较

脚本位于 [`tools/compare-live2d-motion`](../tools/compare-live2d-motion/README.zh-CN.md)。它读取对象文档，因此只需要导出器和一份 Cubism Web Framework 的检出：

```bash
sekai-exporter objects <Live2D 资源包目录> objects
git clone --branch 5-r.5 https://github.com/Live2D/CubismWebFramework.git
cd tools/compare-live2d-motion && npm install && cd -
node tools/compare-live2d-motion/compare.mjs CubismWebFramework objects --report report.json
```

脚本对给定的资源包输出上文的计数与最大差值，并把每类曲线、每种写法的最大差值写入 `report.json`。精确写出的 `motion3.json` 按 32 位精度读取时，若与 clip 的差超过 10⁻⁴，退出状态为 1。
