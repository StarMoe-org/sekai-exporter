# Fixtures

The `mysekai-fixtures` form is the furniture of MySekai, as one JSON document. Converter: `mysekai-fixtures`.

## 1. Runtime

A runtime that reproduces MySekai places, stores, sorts and lists furniture on housing sites. To do that it reads, for each piece of furniture, what the piece is, what a placed piece offers a character, how it stacks and is stored, where it may be put, how large it is, what it costs, which colour cells it has, what sound it plays when placed, its name and description, and its furniture genres. The document gives these facts as one entry per row of the game data table.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document holds `version` (1) and `fixtures`.

`fixtures` is an array of furniture entries, one per row of the game data table, in ascending `id` order. An entry holds the keys below:

| Key | Value |
|---|---|
| `assetbundleName` | The name of the piece's asset bundle. The model package of the piece is this name behind the fixed prefix `mysekai__fixture__`. A model may be named by more than one entry |
| `colorIds` | The colour cells of the piece: the ascending distinct texture ids of its other colours that are above 0, then 1, which is always a cell |
| `description` | The flavour text |
| `firstPutCost` | The layout cost of the first placement |
| `fixtureType` | `system`, `custom`, `plant`, `house_plant`, `surface_appearance`, `gate`, `normal` or `canvas` |
| `fixtureTypeValue` | The number of `fixtureType`: `system` 0, `custom` 1, `plant` 2, `house_plant` 3, `surface_appearance` 4, `gate` 5, `normal` 6, `canvas` 7 |
| `gridDepth` | The depth of the piece on the furniture grid, in cells |
| `gridHeight` | The height of the piece above the floor, in cells |
| `gridWidth` | The width of the piece on the furniture grid, in cells |
| `handleType` | The handle of the piece as written, for example `none`, `block` or `light`. It decides the storing and stacking rules, and a `block_transparent` handle leaves the piece's floor cells passable |
| `id` | The id of the row |
| `layoutType` | The settable layout of the piece as written, for example `floor`, `rug`, `road`, `wall`, `wall_appearance` or `floor_appearance` |
| `mainGenreId` | The furniture genre of the piece, an id of the main genre table |
| `name` | The display name |
| `playerActionType` | `no_action`, `loop`, `timeline` or `one_shot`: what a placed piece offers a character who uses it. A `loop` or `one_shot` piece runs a gimmick, a `timeline` piece a character interaction |
| `playerActionTypeValue` | The number of `playerActionType`: `no_action` 0, `loop` 1, `timeline` 2, `one_shot` 3 |
| `putSoundId` | The id of the row of the put sound table that names the cue a placement plays |
| `putType` | The put type of the piece as written, for example `none`, `put_base` or `put_target`. It decides which pieces are placed together |
| `secondPutCost` | The layout cost of a placement after the first |
| `subGenreId` | The sub genre of the piece, an id of the sub genre table, or `null` |

A reader decides that a wall, a window pane or a clock is a wall fixture, and a road a road fixture, from `layoutType` first and `handleType` second. A wall and a floor appearance share a model, so a runtime that identifies a piece by its package reads the entry of each id, not one entry per package.

## 3. Sources

**Game data table:** `mysekaiFixtures`. Each row gives the columns `id`, `name`, `flavorText`, `gridSize` (an object with `width`, `depth` and `height`), `mysekaiFixtureType`, `mysekaiFixturePlayerActionType`, `mysekaiFixturePutType`, `mysekaiFixtureHandleType`, `mysekaiFixtureMainGenreId`, `mysekaiSettableLayoutType`, `mysekaiFixtureAnotherColors` (objects with a `textureId`), `mysekaiFixturePutSoundId`, `assetbundleName`, `firstPutCost` and `secondPutCost`. `mysekaiFixtureSubGenreId` is an integer or `null`.

The entry keys are written as follows:

| Entry key | Column |
|---|---|
| `id`, `assetbundleName`, `name`, `putSoundId`, `firstPutCost`, `secondPutCost` | The column of the same name |
| `description` | `flavorText` |
| `fixtureType` | `mysekaiFixtureType` |
| `playerActionType` | `mysekaiFixturePlayerActionType` |
| `putType` | `mysekaiFixturePutType` |
| `handleType` | `mysekaiFixtureHandleType` |
| `mainGenreId`, `subGenreId` | `mysekaiFixtureMainGenreId`, `mysekaiFixtureSubGenreId` |
| `gridWidth`, `gridDepth`, `gridHeight` | `gridSize.width`, `gridSize.depth`, `gridSize.height` |
| `layoutType` | `mysekaiSettableLayoutType` |
| `colorIds` | The `textureId` of each entry of `mysekaiFixtureAnotherColors` |

`fixtureTypeValue` and `playerActionTypeValue` are not columns: each is the number of its word, as listed in section 2, so the two keys never disagree and a word outside the two lists is an error rather than a piece that quietly offers nothing.

**Outcomes other than a document:** a version without the table fails the run with the error of the table lookup. A row that lacks a column it must give, that lacks a grid size, that holds a word outside the two lists, or that holds a value of another type makes the asset an error with the code `malformed`. So do two rows of one id.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters. The asset is named `mysekai-runtime/mysekai-fixtures`.

## 5. Correctness

Tests on synthetic game data rows cover the mapping of the columns, the word and the number of every kind and every action, the colour cells, the order by id, every key a row must give, and every outcome above that is an error.

Every key of an entry is read by a runtime that reproduces the game: `id` and `assetbundleName` identify the piece; `name`, `description`, `mainGenreId` and `subGenreId` draw and filter the furniture list; `fixtureType` and `playerActionType` with their numbers select the action of a placed piece; `putType` and `handleType` are the rules of stacking and storing; `gridWidth`, `gridDepth` and `gridHeight` size the piece on the grid; `layoutType` places walls, rugs and roads; `putSoundId` names the placement cue; `firstPutCost` and `secondPutCost` are the layout cost; `colorIds` names the colour cells the piece offers. The row's `mysekaiSettableSiteType`, `mysekaiFixtureTagGroup`, `pronunciation`, `seq` and other columns describe the game data but are not written.

## 6. Version

Form version 1.
