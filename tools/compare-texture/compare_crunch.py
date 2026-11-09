"""Compare the decoding of crunched DXT5 textures with reference decoders.

Usage:
    python compare_crunch.py <decode_stream executable> <crunch_decode executable> <bundle directory>...

Every distinct format 29 texture in the directories is unpacked to BC3 blocks by
`reference/crunch_decode.cpp`, which is built from Unity's crunch library. The blocks are decoded
here with exact arithmetic from the formulas of the S3TC chapter of the Khronos Data Format
Specification, and the pixels must equal those of the decoder. The script prints the counts and
exits with status 1 if anything differs.

Requires UnityPy (MIT).
"""

import hashlib
import pathlib
import struct
import subprocess
import sys
from collections import Counter
from fractions import Fraction

import UnityPy

from compare import deobfuscate

UnityPy.config.FALLBACK_UNITY_VERSION = "2022.3.62f2"
CRUNCHED_DXT5 = 29


def rounded(value):
    """A fraction of 0..=1 as eight bits, rounded to the nearest."""
    return int(value * 255 + Fraction(1, 2))


def decode_bc3(block):
    color0, color1, selectors = struct.unpack("<HHI", block[8:16])
    a0, a1 = block[0], block[1]
    if a0 > a1:
        mixed = [Fraction((8 - code) * a0 + (code - 1) * a1, 7) for code in range(2, 8)]
    else:
        mixed = [Fraction((6 - code) * a0 + (code - 1) * a1, 5) for code in range(2, 6)]
        mixed += [Fraction(0), Fraction(255)]
    alphas = [int(value + Fraction(1, 2)) for value in [Fraction(a0), Fraction(a1), *mixed]]
    bits = int.from_bytes(block[2:8], "little")

    def parts(color):
        return (Fraction(color >> 11, 31), Fraction(color >> 5 & 63, 63), Fraction(color & 31, 31))

    c0, c1 = parts(color0), parts(color1)
    palette = [
        c0,
        c1,
        tuple((2 * a + b) / 3 for a, b in zip(c0, c1)),
        tuple((a + 2 * b) / 3 for a, b in zip(c0, c1)),
    ]
    palette = [bytes(map(rounded, color)) for color in palette]
    return [palette[selectors >> (2 * i) & 3] + bytes([alphas[bits >> (3 * i) & 7]]) for i in range(16)]


def decode_blocks(blocks, width, height):
    """RGBA from top to bottom; the blocks are in stored order, bottom to top."""
    columns = (width + 3) // 4
    stride = width * 4
    pixels = bytearray(stride * height)
    cache = {}
    for index in range(len(blocks) // 16):
        block = blocks[index * 16 : (index + 1) * 16]
        if block not in cache:
            cache[block] = decode_bc3(block)
        texels = cache[block]
        x0, y0 = index % columns * 4, index // columns * 4
        for i, texel in enumerate(texels):
            x, y = x0 + i % 4, y0 + i // 4
            if x < width and y < height:
                pixels[(height - 1 - y) * stride + x * 4 : (height - 1 - y) * stride + x * 4 + 4] = texel
    return bytes(pixels)


def image_data(texture, env):
    stream = texture.m_StreamData
    if stream and stream.path:
        name = stream.path.rsplit("/", 1)[-1]
        return bytes(env.file.files[name].bytes[stream.offset : stream.offset + stream.size])
    return bytes(texture.image_data)


def main():
    ours = subprocess.Popen([sys.argv[1]], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    reference = subprocess.Popen([sys.argv[2]], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    seen, tally = set(), Counter()
    failures = []
    for root in sys.argv[3:]:
        for path in sorted(pathlib.Path(root).iterdir()):
            if not path.is_file() or path.suffix:
                continue
            try:
                env = UnityPy.load(deobfuscate(path.read_bytes()))
            except Exception:
                tally["unreadable bundles"] += 1
                continue
            for obj in env.objects:
                if obj.type.name != "Texture2D":
                    continue
                texture = obj.read()
                if int(texture.m_TextureFormat) != CRUNCHED_DXT5:
                    continue
                tally["textures"] += 1
                data = image_data(texture, env)
                digest = hashlib.sha256(data).digest()
                if digest in seen:
                    continue
                seen.add(digest)
                width, height = texture.m_Width, texture.m_Height
                reference.stdin.write(struct.pack("<I", len(data)) + data)
                reference.stdin.flush()
                size = struct.unpack("<I", reference.stdout.read(4))[0]
                blocks = reference.stdout.read(size)
                ours.stdin.write(struct.pack("<iIII", CRUNCHED_DXT5, width, height, len(data)) + data)
                ours.stdin.flush()
                if ours.stdout.read(1)[0] != 0:
                    length = struct.unpack("<I", ours.stdout.read(4))[0]
                    tally["rejected"] += 1
                    failures.append(f"{path.name} {texture.m_Name}: {ours.stdout.read(length).decode()}")
                    continue
                pixels = ours.stdout.read(width * height * 4)
                if size == 0:
                    tally["reference failed"] += 1
                elif pixels == decode_blocks(blocks, width, height):
                    tally["same"] += 1
                else:
                    tally["different"] += 1
                    failures.append(f"{path.name} {texture.m_Name} {width}x{height}")
    print(dict(tally))
    for line in failures[:20]:
        print(line)
    sys.exit(1 if failures or tally["reference failed"] else 0)


if __name__ == "__main__":
    main()
