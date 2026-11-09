# compare-live2d-motion

[English](README.md)

比较 [Live2D 动作](../../docs/live2d-motion.zh-CN.md)中所述的两种形式：按 Unity 存储方式求值的 clip 曲线，以及同一 clip 写成 `motion3.json` 后由 Live2D 的 Cubism Web Framework 求值的结果。

```bash
sekai-exporter objects <Live2D 资源包目录> objects
git clone --branch 5-r.5 https://github.com/Live2D/CubismWebFramework.git
npm install
node compare.mjs CubismWebFramework objects --report report.json
```

`compare.mjs <框架目录> <对象文档目录>... [--report <文件>]` 读取各目录下对象文档中的每个 `AnimationClip`，以及与之相邻的 `Live2DBuildMotionMetaData` 对象。对每个 clip：

1. 解码 streamed、dense 与 constant 曲线，以 32 位运算在查看器的帧时钟上求值：以 32 位累加 1/60 s 的步长，到达 clip 末尾后保持；并在相邻两帧的中点求值；
2. 把 clip 写成 `motion3.json`：三次段写成控制点位于时长三分之一处的受限 Bezier 段，常值段写成阶梯段，dense 采样写成线性段。曲线按位置比较，因此每条曲线以其在 clip 中的位置命名；
3. 写出两次，一次每个数精确写出，一次每个数舍入到三位小数；每份文件用 `CubismMotion.create` 载入两次，一次保持解析结果，一次把各点舍入为 32 位数；
4. 在相同时刻用框架的 `evaluateCurve` 求值每条曲线，记录差值。

框架由 esbuild 从检出目录打包，唯一的改动是导出 `evaluateCurve`，即 `CubismMotion` 求值每条曲线所用的函数。求值动作不需要 Cubism Core，因此不加载它。

脚本输出 clip 与曲线的数目、三次段末端与下一个键之间的最大跳变、循环的 clip 数与带淡入淡出时间的 clip 数、按函数统计的事件数，以及每种写法和每类曲线的最大差值与超过各阈值的曲线数。`--report` 写出同样的内容，并附上每项最大的十个差值。精确写出的 `motion3.json` 按 32 位精度读取时，若与 clip 的差超过 10⁻⁴，退出状态为 1。

绑定不是 Live2D 参数的 clip 与压缩的 clip 列为跳过。
