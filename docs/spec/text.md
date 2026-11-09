# Text

The `text` form is the file a `TextAsset` stores, byte for byte. Converter: `text` in `sekai-convert` (feature `text`). Form version: 1.

## 1. Runtime

Any program that reads the file the asset was made from: JSON readers, Lua interpreters, Spine runtimes reading `.atlas` and `.skel` files, Live2D runtimes reading `.moc3` and `.model3.json` files, and text editors. The form does not parse or re-encode the file.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `file` | The bytes of `m_Script`, unchanged |

The member's extension is the last extension of the object's first container path, in lower case: `foo.atlas.txt` gives `txt`, `foo.skel.bytes` gives `bytes`, `foo.json` gives `json`. An object with no container path, or whose path has no extension, gives `bytes`. Together with the logical path of the asset, which drops only the last extension, the file keeps the name it had in the game's project.

## 3. Sources

**Root objects:** every `TextAsset` (class 49), except those with a container path ending in `.acb.bytes` (CRI cue sheets) or `.usm.bytes` (CRI movies, including segments named `-001.usm.bytes`, `-002.usm.bytes` and so on). Those are read by the audio and video forms. The comparison of endings ignores case. The form does not read game data tables.

**Fields read:** `m_Script`.

**Outcomes other than a file:**

| Case | Result |
|---|---|
| `m_Script` is missing or is not a string | Error `malformed` |

## 4. Container

The member is finished bytes, written as they are. There are no encoding parameters.

## 5. Correctness

The member is `m_Script` as stored, so it is the file Unity imported. It is checked by comparing the SHA-256 of every member, and its extension, with the bytes and container path that UnityPy reads for the same object, on real bundles.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
