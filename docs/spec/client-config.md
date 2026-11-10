# Client config

[中文](client-config.zh-CN.md)

The `client-config` form is the configuration a client reads by numeric ID, as one JSON document. Converter: `client-config`.

## 1. Runtime

A runtime that reproduces the game asks for a setting by its ID and the type it expects, and uses the value for gameplay constants such as camera limits and timings.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `document` | A JSON document, extension `json` |

The document holds four objects, each from an ID as a string to a value, in ID order:

| Key | Value |
|---|---|
| `FloatConfigs` | Numbers |
| `IntConfigs` | Integers |
| `StringConfigs` | Strings |
| `BoolConfigs` | Booleans |

## 3. Sources

The game data table `clientConfigs`, with rows of `id`, `type` and `value`. `type` names the object of the row: `Float`, `Int`, `String` or `Bool`. `value` is a string, read as that type. A `Bool` value is `true` or `false`, in either case.

**Outcomes other than a document:** a row of another type, an ID that two rows of one type share, or a value that does not read as its type makes the asset an error with the code `malformed`.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters. The asset is named `mysekai-runtime/client-config`.

## 5. Correctness

Tests on synthetic tables cover each type and every outcome above that is an error. The asset of a real version must hold one value per row of the table.

## 6. Version

Form version 1.
