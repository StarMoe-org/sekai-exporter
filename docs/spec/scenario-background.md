# Scenario background

[中文](scenario-background.zh-CN.md)

The `scenario-background` form is the movies a scenario background plays, as a JSON document. Converter: `scenario-background` in `sekai-moesekai` (feature `episode`). Form version: 1.

## 1. Runtime

The table stage that builds the `episode` assets, when it builds an episode whose scene data names a background prefab: it reads the movies of that prefab, so that the episode names the `video` assets the movies are played from. The form gives the names the prefab stores; it does not resolve them into files.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `scenario-background` | A JSON document, extension `json` |

```json
{ "format": "scenario-background", "version": 1, "movies": ["backdrop_loop", "backdrop_b"] }
```

| Key | Value |
|---|---|
| `format`, `version` | `"scenario-background"` and the version of this document format, 1 |
| `movies` | The movie names, each once, in the order the walk of the prefab finds them |

A movie name is the name of the asset bundle the game loads the movie from, as the prefab stores it. An empty name gives no entry.

## 3. Sources

**Root objects:** every `MonoBehaviour` (class 114) whose script is `Assembly-CSharp/Sekai.ScenarioBackgroundBuildData`.

**Fields read:** of the root, `BackgroundPrefab`; of each object the walk reads, `m_IsActive`, `m_Component`, `m_Children`, `m_GameObject` and `movieAssetBundleName`.

**Walk.** The walk starts at the game object that `BackgroundPrefab` refers to and visits objects depth first, children in sibling order:

1. Pop an object. An object that is not a game object (class 1) has nothing below it and ends here. A game object whose `m_IsActive` is false, or is a value that is neither a boolean nor an integer, is skipped with its subtree. `m_IsActive` that is absent or `true` is active, and an integer is active when it is nonzero.
2. Visit the entries of its `m_Component` array in order. Each entry is a reference to a component, or a structure whose `component` is a reference to one; a null reference, or a reference to an object that is not held, is skipped. A transform (class 4) or rect transform (class 224) gives the children of this object: for each entry of its `m_Children`, in order, the reference to a transform, the `m_GameObject` of that transform and the reference to a game object, as above. A `MonoBehaviour` (class 114) whose script is `Assembly-CSharp/Sekai.ScenarioBackgroundMovie` adds its `movieAssetBundleName` when that is a nonempty string and no earlier movie has the same name.
3. Push the children of the object, the last first, so that the next object popped is its first child.

A game object that the walk has visited is not visited again, so a cycle of children ends the walk. The movies found are those the game finds by collecting the movie components of the active objects below the prefab.

**Outcomes other than a document:**

| Case | Result |
|---|---|
| `BackgroundPrefab` is absent or null | No asset |
| The walk finds no movie | No asset |
| `BackgroundPrefab`, an entry of `m_Component` or of `m_Children`, `component` or `m_GameObject` is not a reference, or `m_Component`, `m_Children` or `movieAssetBundleName` is not of its type | Error `malformed` |
| A reference names an object that the version does not hold | The error of resolving it, `missing_object` |

`BackgroundPrefab` is read from the root object, and the `m_GameObject` of a transform of the walk from that transform.

## 4. Container

The member is finished bytes: UTF-8 JSON without insignificant whitespace, with the keys in the order above and the names in walk order. The walk reads objects in a fixed order and keeps each name once, so the same prefab always gives the same bytes.

## 5. Correctness

The document names the movies the game finds under the prefab, so an episode that names the movies of a background names those the game loads.

In the tests of `crates/sekai-format`, the document is written to a fixed sequence of bytes and read back from them. In the tests of `crates/sekai-moesekai` (`tests/story_data.rs`), objects built in memory check:

- the root rule;
- the document's bytes for a prefab with one movie;
- that the walk is depth first in sibling order, including a movie of the prefab root, a grandchild visited before the next sibling of its parent, and a rect transform;
- that an inactive object is skipped with its subtree for `m_IsActive` that is `false`, `0` or null, and that an inactive prefab root gives no asset, while `true` and nonzero integers are active;
- that a movie reached twice is kept once, also when one object lists it twice, and that an empty name adds no movie;
- that components of another script or of another class are ignored;
- that a null prefab, an absent prefab field or a prefab without a movie gives no asset;
- that a cycle of children ends, including two game objects that are each other's children;
- that a reference to a missing object gives `missing_object`;
- that a field of another type gives `malformed`.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
