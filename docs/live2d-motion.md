# Live2D motions

[中文](live2d-motion.zh-CN.md)

A Live2D motion can be kept in two forms: as the Unity animation clip the game ships, or as a Cubism `motion3.json` file. This document explains how the game stores its motions, compares the two forms on real clips with the runtimes that read them, and states which form is canonical and why.

## What the game stores

Live2D Cubism has two layers:

- **Cubism Core** is one native library, the same on every platform. It reads a `.moc3` model and turns parameter values into vertex positions and opacities. It does not play motions.
- **Cubism Framework** is the layer above it, one per platform (native, Web, Unity and others). The files the Cubism Editor exports besides the model, `motion3.json`, `physics3.json`, `exp3.json` and `model3.json`, are Framework formats. Each Framework interprets them itself.

The game uses the Unity Framework. The model is kept as it is: its bundle carries the `.moc3` file, and Cubism Core reads it unchanged. Motions are not kept as `motion3.json`; they are Unity `AnimationClip` objects. Cubism SDK for Unity makes such clips from `motion3.json` when a motion is imported into the editor (`CubismMotion3Json.ToAnimationClip`), turning each segment type into Unity keyframes:

| `motion3.json` segment | Unity keyframes |
|---|---|
| Linear | Two keys whose tangents equal the segment's slope |
| Bezier | Hermite tangents computed from the control point values over a third of the segment's duration; the control points' times are not read |
| Stepped | A key with an infinite in-tangent, which Unity holds constant until the next key |
| Inverse stepped | An extra key 0.01 s after the start |
| `Meta.Loop` | `AnimationClip` `loopTime` |

When the game is built, Unity compiles the keyframes of each curve into a `StreamedClip`: one cubic polynomial per interval between keys, stored as four coefficients. The clips in the game fit this mapping: their curves are continuous chains of cubics, their steps are constant pieces, and their key times and values lie on a 0.001 grid.

The game plays the clips with Unity's animation system and two motion layers of its own. A body layer and a facial layer each crossfade their clips linearly, and the facial layer overrides the body layer for the parameters it animates. Fade times come from a `Live2DBuildMotionMetaData` object next to each clip in the same bundle. The Cubism Framework's motion player, with its own fades, loop and end-point correction, is not used.

Two consequences follow:

- The behaviour the game shows is that of the clip.
- A `motion3.json` read back from a clip is equivalent to the clip, not necessarily to the file the clip was imported from: the importer reshapes a Bezier segment whose control points are not at a third of its duration, and only the reshaped curve is in the game.

## Who reads motions

By default the assets of this repository serve Moesekai (see [Objects and assets](objects-and-assets.md)). Its Live2D viewer evaluates clips as the game does:

- It reads a clip's curves as Unity stores them, binds each curve to a model parameter by the hash Unity binds it by, `crc32("Parameters/<id>")`, and plays the clip on the two layers above.
- It reads a clip's start and stop time, its loop flag and its events, and calls a function only for the events named `OnLive2DInvokeUserData`, the eye-blink settings.
- It reads a clip's fade-in time from the clip's metadata object.

It does not use a Cubism Framework motion player.

## The two forms

**The clip's curves.** A document keeps each curve of the clip as stored:

- streamed curves as their cubic segments `[t, a, b, c, d]`, with `value(t′) = ((a·δ + b)·δ + c)·δ + d` and `δ = t′ − t` from the segment that starts at or before `t′`;
- dense curves as their samples, interpolated linearly;
- constant curves as one value.

Each curve is bound by its path and attribute hashes. The document also carries the start and stop time, the loop flag, the events with their function names, and the fade times of the metadata object.

**`motion3.json`.** The same clip can be written as `motion3.json` by inverting the importer's mapping:

- a cubic segment becomes a restricted Bezier segment with control points at a third of its duration, which describes the same cubic;
- a constant segment followed by a change of value becomes a stepped segment;
- dense samples become linear segments.

Every curve needs a parameter or part identifier, and `Meta.Loop` comes from `loopTime`. Events become user data, which keeps an event's time and data string but not its function name.

## Comparison

The comparison asks whether the two forms give the same parameter values when each is evaluated by the runtime it is meant for.

- **Clips:** every `AnimationClip` of seven motion bundles and fourteen model bundles of the Japanese region. That is 1409 clips (1380 in the motion bundles, 29 in the model bundles), from main, collaboration and supporting characters, both model generations, body motions and facials.
- **Curves:** 22 822 streamed and 58 211 constant curves; the bundles hold no dense curve.
- **Time grid:** the viewer's frame clock (1/60 s steps summed in 32-bit arithmetic and held at the clip's end) plus the midpoint between every two frames, 21 822 571 values in all.
- **Clip side:** each curve is evaluated as Unity stores it, in 32-bit arithmetic, operation for operation as the viewer's evaluator does. On the 21 213 185 values of the clips the viewer binds to the sampled models, the two agree bit for bit.
- **`motion3.json` side:** each clip is written as `motion3.json` and evaluated by `evaluateCurve` of the Cubism Web Framework, the function `CubismMotion` evaluates every curve with.

The `motion3.json` is written in two ways: with every number exactly (the decimal that reads back to the stored 32-bit value), and with every number rounded to three decimals. Each way is read twice: as the Web Framework parses it, into 64-bit numbers, and with the points rounded to 32-bit numbers, the precision of the native Framework.

| `motion3.json` | Read as | Largest difference, streamed curves | Largest difference, constant curves |
|---|---|---|---|
| Exact numbers | 64-bit | 2.1 × 10⁻⁵ | 0 |
| Exact numbers | 32-bit | 1.9 × 10⁻⁵ | 0 |
| Three decimals | 64-bit | 1.909 (one frame of one clip) | 1.7 × 10⁻⁶ |
| Three decimals | 32-bit | 9.8 × 10⁻⁵ | 0 |

The differences are in parameter units; a head angle, for example, spans −30 to 30. Midpoints between frames show differences of the same size as the frames, except for the tie described below, which falls on a frame.

What the values show:

- **The curves are continuous.** At every key, the cubic of the segment before it ends within 2.2 × 10⁻⁵ of the value the next segment starts with, so a chain of segments that share their end points carries every curve.
- **With exact numbers, `motion3.json` gives the clip's values** to within 32-bit rounding of the cubic.
- **Three decimals add a rounding error below 10⁻⁴.** The keys lie on a 0.001 grid; only the control point values, which are derived from the slopes, are rounded.
- **The one large difference is a tie at a step.** It is one frame of a clip whose curves step every frame. Its key time is the 32-bit number nearest 0.7, and the viewer's clock reaches exactly that number on frame 42. The clip has reached the new step at that time. The text `0.7` read as a 64-bit number is slightly larger, so the Web Framework still returns the previous step. With exact numbers, or with 32-bit points, the tie does not arise.

Beyond the values, the sample shows:

- no clip loops;
- every clip has a metadata object with fade times;
- all 896 events call `OnLive2DInvokeUserData`.

## Decision

The canonical form of a Live2D motion is the document of the clip's curves.

`motion3.json` can carry the values of these clips, so precision does not decide the question. These properties do:

- **It is what the game plays.** The clip is the motion the game evaluates; no `motion3.json` is in the game data.
- **It needs nothing outside the bundle.** The clip binds its curves by hashes, and the viewer resolves them against the parameter table of the model it shows. `motion3.json` names every curve, and the names exist only in the `.moc3` files of model bundles, which motion bundles do not depend on. Writing `motion3.json` would make a motion bundle's asset depend on which model bundles a run has read. That breaks the rule that an asset is built from its bundle and the bundles it depends on (see [Pipeline](pipeline.md)), and a curve that no model read so far defines would be left out.
- **It keeps what the runtime reads.** The loop flag, the events' function names, the fade times next to the clip, and the curve pieces as stored are all part of the clip. `motion3.json` either lacks them or gives them a meaning of its own: its user data has no function name, and its fades and loop belong to the Framework's motion player.
- **One curve document serves every clip.** Costume motions are 3D transforms, which `motion3.json` cannot express, and they use the same kind of document. One evaluator covers both.

A runtime built on a Cubism Framework can be given a `motion3.json` derived from the canonical document by the mapping above, with the identifiers of a chosen model. Such a file is a view for that runtime; it is not part of the canonical layer.

## Reproducing the comparison

The script is [`tools/compare-live2d-motion`](../tools/compare-live2d-motion/README.md). It reads object documents, so it needs only the exporter and a checkout of the Cubism Web Framework:

```bash
sekai-exporter objects <directory of Live2D bundles> objects
git clone --branch 5-r.5 https://github.com/Live2D/CubismWebFramework.git
cd tools/compare-live2d-motion && npm install && cd -
node tools/compare-live2d-motion/compare.mjs CubismWebFramework objects --report report.json
```

It prints the counts and the largest differences above for the bundles given, and writes the largest differences of each curve kind and each way of writing to `report.json`. The exit status is 1 when the exact `motion3.json`, read at 32-bit precision, differs from the clip by more than 10⁻⁴.
