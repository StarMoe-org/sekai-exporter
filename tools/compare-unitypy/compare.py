"""Compare object documents with the fields UnityPy reads from the same bundles.

Usage:
    python compare.py <bundle directory> <object document directory>

<object document directory> is the output of `sekai-exporter objects` for the same bundle
directory. Every field of every object is mapped to the value model of the object document
specification and compared. The script prints one line per class and the first differences.

Requires UnityPy (MIT).
"""

import base64
import hashlib
import json
import math
import pathlib
import struct
import sys
from collections import Counter

import UnityPy

INVERTED = b"\x10\x00\x00\x00"
PREFIXED = b"\x20\x00\x00\x00"


def deobfuscate(data: bytes) -> bytes:
    if data.startswith(b"UnityFS\x00"):
        return data
    if data[:4] == PREFIXED:
        return data[4:]
    if data[:4] == INVERTED:
        body = bytearray(data[4:])
        for i in range(128):
            if i % 8 < 5:
                body[i] ^= 0xFF
        return bytes(body)
    raise ValueError("unknown bundle header")


def f32(x: float) -> bytes:
    return struct.pack("<f", x)


def is_payload(value) -> bool:
    return isinstance(value, dict) and set(value) == {"$bytes"}


def payload_matches(ours, theirs: bytes) -> bool:
    if not is_payload(ours):
        return False
    p = ours["$bytes"]
    if p["size"] != len(theirs) or p["sha256"] != hashlib.sha256(theirs).hexdigest():
        return False
    return "base64" not in p or base64.b64decode(p["base64"]) == theirs


def compare(ours, theirs, path, out):
    """Appends a description of every difference between `ours` and `theirs` to `out`."""
    if len(out) > 3:
        return
    if isinstance(theirs, bool):
        ok = ours is theirs
    elif isinstance(theirs, int):
        ok = (isinstance(ours, int) and not isinstance(ours, bool) and ours == theirs) or (
            isinstance(ours, str) and ours.lstrip("-").isdigit() and int(ours) == theirs
        )
    elif isinstance(theirs, float):
        if math.isnan(theirs):
            ok = ours == "NaN"
        elif math.isinf(theirs):
            ok = ours == ("Infinity" if theirs > 0 else "-Infinity")
        elif isinstance(ours, (int, float)) and not isinstance(ours, bool):
            ok = float(ours) == theirs or f32(float(ours)) == f32(theirs)
        else:
            ok = False
    elif isinstance(theirs, str):
        if isinstance(ours, str):
            ok = ours == theirs
        else:
            ok = payload_matches(ours, theirs.encode("utf-8", "surrogateescape"))
    elif isinstance(theirs, (bytes, bytearray)):
        ok = payload_matches(ours, bytes(theirs))
    elif isinstance(theirs, tuple) and isinstance(ours, dict) and list(ours) == ["first", "second"]:
        # UnityPy reads every `pair` as a tuple; outside a map the specification writes a pair
        # as the structure it is.
        compare(ours["first"], theirs[0], f"{path}.first", out)
        compare(ours["second"], theirs[1], f"{path}.second", out)
        return
    elif isinstance(theirs, tuple):
        if not (isinstance(ours, list) and len(ours) == len(theirs)):
            ok = False
        else:
            for i, (a, b) in enumerate(zip(ours, theirs)):
                compare(a, b, f"{path}[{i}]", out)
            return
    elif isinstance(theirs, list) and is_payload(ours):
        ok = all(isinstance(x, int) for x in theirs) and payload_matches(
            ours, bytes(x & 0xFF for x in theirs)
        )
    elif isinstance(theirs, list):
        if not (isinstance(ours, list) and len(ours) == len(theirs)):
            out.append(f"{path}: length {len(ours) if isinstance(ours, list) else ours!r} vs {len(theirs)}")
            return
        for i, (a, b) in enumerate(zip(ours, theirs)):
            compare(a, b, f"{path}[{i}]", out)
        return
    elif isinstance(theirs, dict):
        if set(theirs) == {"m_FileID", "m_PathID"}:
            fid, pid = theirs["m_FileID"], theirs["m_PathID"]
            if fid == 0 and pid == 0:
                ok = ours is None
            else:
                ref = ours.get("$ref") if isinstance(ours, dict) else None
                ok = (
                    ref is not None
                    and int(ref["id"]) == pid
                    and ((fid == 0) == (set(ref) == {"id"}))
                )
        elif not isinstance(ours, dict) or list(ours) != list(theirs):
            out.append(f"{path}: keys {list(ours) if isinstance(ours, dict) else ours!r} vs {list(theirs)}")
            return
        else:
            for k in theirs:
                compare(ours[k], theirs[k], f"{path}.{k}", out)
            return
    else:
        ok = False
    if not ok:
        out.append(f"{path}: {str(ours)[:80]} vs {str(theirs)[:80]}")


def main():
    bundles, documents = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    UnityPy.config.FALLBACK_UNITY_VERSION = "2022.3.62f2"
    totals, differing, shown = Counter(), Counter(), Counter()
    examples = []
    for path in sorted(p for p in bundles.iterdir() if p.is_file()):
        doc = json.loads((documents / path.name / "_objects.json").read_text(encoding="utf-8"))
        ours = {
            (f["path"], int(o["id"])): o for f in doc["files"] for o in f["objects"]
        }
        env = UnityPy.load(deobfuscate(path.read_bytes()))
        for bundle in env.files.values():
            for name, sf in bundle.files.items():
                if not hasattr(sf, "objects"):
                    continue
                for path_id, obj in sf.objects.items():
                    mine = ours.pop((name, path_id), None)
                    cls = mine["class"] if mine else "?"
                    totals[cls] += 1
                    out = []
                    if mine is None:
                        out.append("missing from the document")
                    else:
                        compare(mine["fields"], obj.read_typetree(), "fields", out)
                    if out:
                        differing[cls] += 1
                        if shown[cls] < 2:
                            shown[cls] += 1
                            examples.append(f"{path.name} {name} {path_id} {cls}: {'; '.join(out)}")
        for key in ours:
            differing["(only in the document)"] += 1
            examples.append(f"{path.name} {key}: only in the document")
    print(f"objects {sum(totals.values())} differing {sum(differing.values())}")
    for cls, n in totals.most_common():
        print(f"  {cls}: {n} objects, {differing[cls]} differing")
    for line in examples:
        print(line)
    return 1 if differing else 0


if __name__ == "__main__":
    sys.exit(main())
