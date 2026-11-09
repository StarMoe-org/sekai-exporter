# compare-live2d-motion

[中文](README.zh-CN.md)

Compares the two forms of Live2D motion clips described in [Live2D motions](../../docs/live2d-motion.md): the clip's curves, evaluated as Unity stores them, and the same clip written as `motion3.json` and evaluated by Live2D's Cubism Web Framework.

```bash
sekai-exporter objects <directory of Live2D bundles> objects
git clone --branch 5-r.5 https://github.com/Live2D/CubismWebFramework.git
npm install
node compare.mjs CubismWebFramework objects --report report.json
```

`compare.mjs <framework directory> <objects directory>... [--report <file>]` reads every `AnimationClip` of the object documents under the directories, with the `Live2DBuildMotionMetaData` object next to it. For each clip it:

1. decodes the streamed, dense and constant curves and evaluates them in 32-bit arithmetic on the viewer's frame clock, 1/60 s steps summed in 32 bits and held at the clip's end, and on the midpoints between frames;
2. writes the clip as `motion3.json`: a cubic segment becomes the restricted Bezier with control points at a third of its duration, a constant segment becomes a stepped segment, and dense samples become linear segments. Each curve is named by its position in the clip, since the curves are compared by position;
3. writes the file twice, with every number exactly and with every number rounded to three decimals, and loads each with `CubismMotion.create`, once as parsed and once with its points rounded to 32-bit numbers;
4. evaluates every curve with the framework's `evaluateCurve` at the same times and records the differences.

The framework is bundled from the checkout with esbuild. The only change is that `evaluateCurve`, which `CubismMotion` evaluates every curve with, is exported. Motions need no Cubism Core, so none is loaded.

The script prints the counts of clips and curves, the largest jump between the end of a cubic and the next key, the clips that loop and that have fade times, the events by function, and per way of writing and curve kind the largest difference and the number of curves over each threshold. `--report` writes the same with the ten largest differences of each. The exit status is 1 when the exact `motion3.json`, read at 32-bit precision, differs from the clip by more than 10⁻⁴.

Clips whose bindings are not Live2D parameters, and compressed clips, are listed as skipped.
