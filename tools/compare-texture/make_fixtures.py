"""Write the texture fixtures of sekai-codec from bundles built by tools/unity-fixtures.

Usage:
    python make_fixtures.py <bundle directory> <output directory>

For every texture the script writes the bytes of its first image to `<name>.bin` and appends a
line to `expected.txt`: format, width, height, file name and the SHA-256 of the RGBA pixels, rows
from top to bottom, that the reference decoder gives. See compare.py for the references.
"""

import hashlib
import pathlib
import sys
import warnings

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import compare
import UnityPy


def main():
    warnings.simplefilter("ignore")
    UnityPy.config.FALLBACK_UNITY_VERSION = "2022.3.62f2"
    source, output = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    output.mkdir(parents=True, exist_ok=True)
    contexts = {}
    lines = []
    for path in sorted(source.iterdir()):
        if path.suffix or path.name == source.name:
            continue
        for obj in UnityPy.load(path.read_bytes()).objects:
            if obj.type.name != "Texture2D":
                continue
            texture = obj.read()
            fmt, width, height = int(texture.m_TextureFormat), texture.m_Width, texture.m_Height
            data = bytes(texture.get_image_data())
            pixels = compare.reference(contexts, fmt, width, height, data)
            stride = width * 4
            flipped = b"".join(pixels[(height - 1 - y) * stride : (height - y) * stride] for y in range(height))
            (output / f"{path.name}.bin").write_bytes(data)
            lines.append(f"{fmt} {width} {height} {path.name}.bin {hashlib.sha256(flipped).hexdigest()}")
    (output / "expected.txt").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
