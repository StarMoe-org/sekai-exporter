# Fixture attach points

[中文](fixture-attach-points.zh-CN.md)

The attach points of every piece of MySekai furniture of a version, joined into one JSON document. Converter: `fixture-attach-points`. The attach points of one piece are the `attach-points` member of its [furniture model](fixture-model.md#26-attach-points), which defines their content.

## 1. Runtime

A runtime that reproduces MySekai loads this document once, when it starts. It plans where characters go to use a piece of furniture, and which piece a character can use, from the attach points of every placed piece, before and without loading their models. When it loads the model of a placed piece, it binds the piece's root node and its collider by `sourceObject` (see [naming nodes](fixture-model.md#7-naming-nodes)).

## 2. Contents

One member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document holds `packages`, an object from the last path segment of a furniture package to the `attach-points` member of that package's `fixture-model` asset, unchanged. Every piece with a `fixture-model` asset is in `packages`, including a piece without attach points, whose `points` list is empty. A piece whose `fixture-model` asset could not be produced is not in `packages`: the runtime cannot place it.

## 3. Sources

The `fixture-model` assets of the packages under `mysekai/fixture/`. A version without one has no asset.

**Outcomes other than a document:** an `attach-points` member that is not a JSON document makes the asset an error with the code `malformed`.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters. The asset is named `mysekai-runtime/fixture-attach-points`.

## 5. Correctness

Tests on synthetic versions cover the joining of the packages, a piece without attach points, and a version without any furniture model. The asset of a real version must hold every package that has a `fixture-model` asset, each with the member of that asset byte for byte as JSON.

## 6. Version

Form version: 1.
