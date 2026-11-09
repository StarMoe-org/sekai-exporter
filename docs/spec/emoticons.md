# Emoticons

An overhead item is a small effect shown above a character: a tree of nodes that draw sprites through three clips, or a set of particle emitters. The `emoticon` form is one item as a JSON document. The `emoticons` form joins every item of a version into one document. Converters: `emoticon` and `emoticons`.

## 1. Runtime

A runtime that reproduces MySekai shows an item by name. For a sprite item it plays the `start` clip, repeats the `loop` clip until told to stop, sets the animator flag `loopEndFlag`, and plays the `end` clip. For a particle item it simulates each emitter at the anchor the view names. It reads the joined document once at load and fetches the image files the document names.

## 2. Contents

Both forms have one member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document of `emoticons` holds `items`, an object keyed by the last segment of the name of the package of each item, and `summary`, with `items`, the number of items, and `unsupported`, every entry of the `unsupported` arrays of the items.

The document of `emoticon` holds:

| Key | Value |
|---|---|
| `view` | `kind`, `sprite` or `particle`; `soundInput`, the argument of the sound the item plays; for a particle item `anchor`, `Face`, `Spine` or `Hips`, and `keepPosition`, whether the effect stays where it was spawned |
| `animator` | Only for an item with an animator. `loopEndFlag`, the name of the flag that ends the loop |
| `textures` | Every image the item draws, wherever it is stored, in name order. `name`, `file`, `width`, `height`. `file` is the path of the image of the `texture` form below the directory `mysekai/effect/emoticon/` |
| `sprites` | An object keyed by sprite name. `texture`, the name of an entry of `textures`; `rect`, `[x, y, width, height]` in pixels; `pivot`, `[x, y]` as fractions of the rect; `pixelsToUnits` |
| `nodes` | The nodes of the prefab, parents first and siblings in path id order |
| `clips` | Only keys `start`, `loop` and `end` are played. `rate`, frames per second; `duration`, seconds; `channels` |
| `particles` | One entry per node that emits, in node path order. `node`, `system`, `renderer` |
| `unsupported` | What the document leaves out, one object per thing, each with a `reason` |

A node holds `path`, the path below the root of the prefab, empty for the root; `parent`, the path of its parent, or `null` for the root; `active`; `position` and `scale`, `[x, y, z]`; `rotation`, a quaternion `[x, y, z, w]`; and `animationPath`, the path below the node of the animator, which is what channels name, or `null` for a node outside it. A node with a sprite renderer also holds `sprite`, a key of `sprites` or `null`; `sortingOrder`; `color`, `[r, g, b, a]`; `flipX`; `flipY`; `rendererEnabled`; and `material`.

A material holds `textures`, the `file` of the image of each texture slot that has one; `textureScaleOffset`, `[scale x, scale y, offset x, offset y]` of every texture slot; and `floats`, the properties `_BaseMapMode`, `_BaseMapRotation`, `_BaseMapRotationCoord`, `_BaseMapRotationEnabled`, `_Cull`, `_SoftParticlesEnabled`, `_ZOffset`, `_ZTest` and `_ZWrite` that the material holds.

A channel holds `path`, an `animationPath` of a node or `null` if none matches; `property`, `position`, `rotation`, `scale` or `eulerAngles`; and `values`, one array per frame of the components of the property.

`system` holds `duration`, `looping`, `simulationSpeed`, `simulationSpace` (`Local`, `World` or `Custom`), `scalingMode`, `maxParticles` and `start`, and one key per enabled module: `emission`, `shape`, `sizeOverLifetime`, `rotationOverLifetime`, `velocityOverLifetime`, `limitVelocity`, `textureSheet`, `colorOverLifetime` and `subEmitters`. The keys of a module are the fields of the table in section 3.

A value that can vary is an object whose `mode` selects its other keys:

| `mode` | Keys |
|---|---|
| `constant` | `value` |
| `twoConstants` | `min`, `max` |
| `curve` | `multiplier`, `keys`, `preInfinity`, `postInfinity` |
| `twoCurves` | `multiplier`, `minKeys`, `minPreInfinity`, `minPostInfinity`, `maxKeys`, `maxPreInfinity`, `maxPostInfinity` |

A curve key holds `time`, `value`, `inSlope`, `outSlope`, `weightedMode`, `inWeight` and `outWeight`. A slope that is infinite is the string `Infinity` or `-Infinity`. A colour is an object whose `mode` is `color` (`color`), `twoColors` (`min`, `max`), `gradient` or `randomColor` (`gradient`) or `twoGradients` (`minGradient`, `maxGradient`). A gradient holds `colorKeys`, each a `time` and an `[r, g, b]` `color`, and `alphaKeys`, each a `time` and an `alpha`, with times from 0 to 1.

A sub-emitter holds `emitter`, the path of the node that emits, or `null` when it is not in the package; `type`, `birth`, `collision`, `death`, `trigger` or `manual`; `inherit`, flags `color`, `size`, `rotation`, `lifetime` and `duration`; and `emitProbability`.

`renderer` holds `enabled`, `renderMode`, `normalDirection`, `flip`, `pivot`, `allowRoll`, `alignment`, `minParticleSize`, `maxParticleSize`, `lengthScale`, `velocityScale`, `cameraVelocityScale`, `freeformStretching` and `material`, a material or `null`.

## 3. Sources

**Objects of `emoticon`:** a `GameObject` listed in the container of a package under `mysekai/effect/emoticon/`. The item is what its references reach: the transforms below it, and on those nodes the `AvatarEmoticonSpriteView` or `AvatarEmoticonParticleView` script, the `Animator`, `SpriteRenderer`, `ParticleSystem` and `ParticleSystemRenderer` components, with the clips listed in `m_AnimationClips` of the controller of the animator. Any other component is an `unsupported` entry with `type`.

A clip is named by the last `_`-separated part of its name when that part is a phase. Channels group the `Transform` curves of a clip by path and property. A clip of duration `d` and rate `r` has `max(2, round(d × r) + 1)` frames, and frame `k` is evaluated at time `k / r` in 32-bit arithmetic; a component without a curve is 0. A curve of any other class is an `unsupported` entry with `attribute`, once per attribute of each clip. `animationPath` values are matched to curves by the CRC-32 of the path.

The fields of a particle system are:

| Key | Stored field |
|---|---|
| `duration` | `lengthInSec` |
| `simulationSpace` | `moveWithTransform` |
| `maxParticles` | `InitialModule.maxNumParticles` |
| `start` | `InitialModule`: `startLifetime`, `startSpeed`, `startSize`, `startRotation`, `startColor`, `gravityModifier`, `size3D`, `rotation3D`; with `size3D` also `startSizeY` and `startSizeZ`; with `rotation3D` also `startRotationX` and `startRotationY` |
| `emission` | `EmissionModule`: `rateOverTime`, and `bursts` from `m_Bursts`, each with `time`, `count` from `countCurve`, `cycleCount`, `repeatInterval`, `probability` |
| `shape` | `ShapeModule`: `type`, `radius.value`, `radiusThickness`, `arc.value`, `angle`, `m_Position`, `m_Rotation`, `m_Scale` |
| `sizeOverLifetime` | `SizeModule`: `separateAxes`, `curve`; with `separateAxes` also `y` and `z` |
| `rotationOverLifetime` | `RotationModule`: `separateAxes`, `curve`, the z axis; with `separateAxes` also `x` and `y` |
| `velocityOverLifetime` | `VelocityModule`: `x`, `y`, `z`, `speedModifier` |
| `limitVelocity` | `ClampVelocityModule`: `separateAxis`, `magnitude`, `dampen`, `drag`, `multiplyDragBySize` from `multiplyDragByParticleSize`, `multiplyDragByVelocity` from `multiplyDragByParticleVelocity` |
| `textureSheet` | `UVModule`: `mode`, `tilesX`, `tilesY`, `animationType`, `timeMode`, `fps`, `cycles`, `rowIndex`, `rowMode`, `startFrame`, `frameOverTime`, `speedRange`, `uvChannelMask`, `flipU`, `flipV` |
| `colorOverLifetime` | `ColorModule.gradient` |

Any other module that is enabled is an `unsupported` entry with `module` and `node`. Renderer keys come from the field of the same name with the prefix `m_`, except `alignment` from `m_RenderAlignment`.

**Assets of `emoticons`:** every `emoticon` asset of a package under `mysekai/effect/emoticon/`. A version without any has no `emoticons` asset.

## 4. Container

UTF-8 JSON without a byte order mark, compact. Numbers that are stored are written as stored. Channel values are 32-bit floats written as the shortest decimal that reads back to them.

## 5. Correctness

A runtime builds each item from these keys and needs nothing else from the package. An enabled module or a component the form does not model is listed by name, so a loss is visible instead of silent. Unit tests on synthetic objects check value decoding, module gating, sub-emitters, clip sampling and every `unsupported` entry. A real version is compared key by key with an independent reading of the same packages.

## 6. Version

`emoticon` 1, `emoticons` 1.
