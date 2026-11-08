# compare-unitypy

Compares object documents with the fields [UnityPy](https://github.com/K0lb3/UnityPy) reads from the same bundles. Both readers' output is mapped to the value model of the [object document specification](../../docs/spec/object-documents.md), and every field of every object must be equal.

```bash
pip install UnityPy==1.25.4
sekai-exporter objects <bundles> <documents>
python compare.py <bundles> <documents>
```

The script prints the number of objects and differing objects per class, and up to two differences per class. It exits with status 1 when any object differs.

Continuous integration runs it on the fixture bundles in `crates/sekai-unity/tests/fixtures`. It can be run on any other directory of bundles in the same way.

UnityPy writes some values in a different shape from the specification. The script maps them before comparing:

| UnityPy | Specification |
|---|---|
| An array of 8-bit integers as a list of numbers | A byte payload |
| A `pair` outside a map as a tuple | A structure with `first` and `second` |
| A PPtr as `{m_FileID, m_PathID}` | `null` or a reference |
| A 64-bit integer as a number | A decimal string |
