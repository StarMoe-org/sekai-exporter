# Fixture areas

The placement grids of the furniture in MySekai. The `fixture-meta` form is the grids of one piece of furniture and the `fixture-areas` form joins them into one JSON document. Converters: `fixture-meta` and `fixture-areas`.

## 1. Runtime

A runtime that reproduces MySekai reads the grids of a piece of furniture when it is placed: which cells take other furniture on top, where a character can move, what a cutscene covers and which cells count as used.

## 2. Contents

Each form has one member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

A `fixture-meta` document holds four grids, `stackEnables`, `motionArea`, `cutsceneArea` and `AddUsingGrid`. A grid is a list of rows in stored order, and a row is a list of booleans in stored order. All rows of a grid have the same length. A grid without rows is an empty list.

A `fixture-areas` document holds `packages`, an object from the last path segment of a package to the `fixture-meta` document of that package. A package without such an object is not in `packages`.

## 3. Sources

`fixture-meta`: a `MonoBehaviour` of the script `Sekai.Mysekai.FixtureBundleMeta`. Each grid is the field of that name, whose `array` holds the rows and whose rows hold the cells as `columns`. A non-zero cell is `true`.

`fixture-areas`: the `fixture-meta` assets of the packages under `mysekai/fixture/`. A version without one has no asset.

**Outcomes other than a document:** a field that the object does not hold or that has another type, or a grid whose rows differ in length, makes the asset an error with the code `malformed`.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters. A `fixture-meta` asset takes the name of its container path. The `fixture-areas` asset is named `mysekai-runtime/fixture-areas`.

## 5. Correctness

Tests on synthetic objects cover the cells of a grid, an empty grid and a grid that is not rectangular. Tests on synthetic versions cover the joining of the packages and a version without any. The asset of a real version must hold every package that has such an object.

## 6. Version

Both forms are at version 1.
