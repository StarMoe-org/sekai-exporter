# Virtual live script

[中文](virtual-live-script.zh-CN.md)

The `virtual-live-script` form is what happens in one part of a virtual live — who appears, moves and speaks, the lights, sound and audience — as a JSON document. Converter: `virtual-live-script` in `sekai-moesekai` (feature `virtual-live`). Form version: 1.

## 1. Runtime

Readers that show or search what is said and done in a virtual live, and players that replay it. The form gives the script as the game stores it; it does not resolve the names it holds into files or join it with the game data tables that list the virtual lives.

## 2. Contents

One member:

| Role | Content |
|---|---|
| `script` | A JSON document, extension `json` |

The document is one object holding every serialized field of the script object, in stored order, except the four fields every `MonoBehaviour` has: `m_GameObject`, `m_Enabled`, `m_Script` and `m_Name`. Values are written as in the [`scenario`](scenario.md#2-contents) form. The events of the script are arrays by kind, such as `characterTalkEvents` and `lightEvents`; each event gives its time in seconds.

## 3. Sources

**Root objects:** every `MonoBehaviour` (class 114) whose script is `Assembly-CSharp/Sekai.Core.VirtualLive.MasterOfCeremonyData`. The form does not read game data tables.

**Fields read:** all serialized fields of the root object.

**Outcomes other than a document:** those of the [`scenario`](scenario.md#3-sources) form.

## 4. Container

The member is finished bytes, a UTF-8 JSON file as described in section 2. There are no encoding parameters.

## 5. Correctness

The document holds the stored values of the object without interpretation, and the same object always gives the same bytes. A test builds a script object in memory and compares the bytes; on real bundles, every object of this script reads exactly to its end.

## 6. Version

| Version | Change |
|---|---|
| 1 | First version |
