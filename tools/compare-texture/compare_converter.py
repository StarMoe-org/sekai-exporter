"""Compare the texture converter with reference decoders, texture by texture.

Usage:
    python compare_converter.py <texture_hashes executable> <bundle directory>...

The executable is the `texture_hashes` example of `sekai-pipeline`. It opens the directories as
one version, runs the texture converter on every `Texture2D` and `Cubemap`, and prints the
SHA-256 of each image. This script reads the same textures with UnityPy, decodes them with the
reference decoders of `compare.py`, and compares the two lists by bundle and path ID. The check
covers what the converter adds to the decoder: reading the fields, following `m_StreamData` into
the resource files of the bundle, and the row order of the result.

Requires UnityPy (MIT), astc-encoder-py (MIT) and texture2ddecoder (MIT).
"""

import hashlib
import pathlib
import struct
import subprocess
import sys
from collections import Counter

import UnityPy

from compare import deobfuscate, reference

CLASSES = {"Texture2D", "Cubemap"}


def top_down(pixels, width, height):
    stride = width * 4
    return b"".join(pixels[(height - 1 - y) * stride : (height - y) * stride] for y in range(height))


def references(roots):
    contexts = {}
    known = {}
    out = {}
    for root in roots:
        for path in sorted(pathlib.Path(root).iterdir()):
            if not path.is_file():
                continue
            env = UnityPy.load(deobfuscate(path.read_bytes()))
            for obj in env.objects:
                if obj.type.name not in CLASSES:
                    continue
                key = (path.name, str(obj.path_id))
                tex = obj.read()
                fmt, width, height = int(tex.m_TextureFormat), tex.m_Width, tex.m_Height
                stream = getattr(tex, "m_StreamData", None)
                if not width or not height or not (tex.image_data or (stream and stream.path)):
                    out[key] = "none"
                    continue
                data = bytes(tex.get_image_data())
                digest = hashlib.sha256(struct.pack("<iII", fmt, width, height) + data).digest()
                if digest not in known:
                    pixels = reference(contexts, fmt, width, height, data)
                    if pixels is None:
                        known[digest] = f"no reference for format {fmt}"
                    else:
                        known[digest] = hashlib.sha256(top_down(pixels, width, height)).hexdigest()
                out[key] = known[digest]
    return out


def main():
    UnityPy.config.FALLBACK_UNITY_VERSION = "2022.3.62f2"
    names = Counter(p.name for root in sys.argv[2:] for p in pathlib.Path(root).iterdir() if p.is_file())
    shared = sorted(name for name, count in names.items() if count > 1)
    if shared:
        sys.exit(f"bundle names must be unique across the directories: {', '.join(shared)}")
    listing = subprocess.run([sys.argv[1], *sys.argv[2:]], check=True, capture_output=True).stdout
    ours = {}
    for line in listing.decode().splitlines():
        bundle, path_id, result = line.split("\t", 2)
        ours[(bundle, path_id)] = result
    theirs = references(sys.argv[2:])
    results = Counter()
    samples = []
    for key in sorted(set(ours) | set(theirs)):
        a, b = ours.get(key), theirs.get(key)
        if a == b:
            results["same" if a != "none" else "same: no image"] += 1
            continue
        if a is not None and b is not None and a.startswith("error unsupported") and b.startswith("no reference"):
            results["unsupported by both"] += 1
            continue
        kind = "only ours" if b is None else "only reference" if a is None else "different"
        results[kind] += 1
        if len(samples) < 10:
            samples.append(f"{key[0]} {key[1]}: ours {a}, reference {b}")
    print(dict(results))
    for line in samples:
        print("   ", line)
    sys.exit(1 if results.keys() - {"same", "same: no image", "unsupported by both"} else 0)


if __name__ == "__main__":
    main()
