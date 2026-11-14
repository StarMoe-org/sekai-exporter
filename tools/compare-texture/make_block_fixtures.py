"""Write the synthetic ETC, EAC and PVRTC fixtures of the decoder tests.

Usage:
    python make_block_fixtures.py <block_decode executable> <output directory>

Each fixture is seeded random data: random blocks reach every mode of a format. ETC1 keeps only
valid blocks. The images have sizes that are not whole blocks, PVRTC also non-square ones and
ones below the smallest size the format stores. The script writes the files and `blocks.txt`, one
line per file: format, width, height, file name and the SHA-256 of the pixels the reference
decoders give, top row first.
"""

import hashlib
import pathlib
import random
import sys

from block_reference import BlockReference, data_size, etc1_in_range

# Format, width, height, file name and seed.
FIXTURES = [
    (30, 64, 32, "pvrtc_rgb2.bin", 1),
    (31, 32, 64, "pvrtc_rgba2.bin", 2),
    (32, 64, 32, "pvrtc_rgb4.bin", 3),
    (33, 32, 64, "pvrtc_rgba4.bin", 4),
    (31, 12, 6, "pvrtc_rgba2_small.bin", 5),
    (33, 5, 3, "pvrtc_rgba4_small.bin", 6),
    (34, 30, 22, "etc1_random.bin", 1),
    (45, 30, 22, "etc2_rgb_random.bin", 2),
    (47, 30, 22, "etc2_rgba8_random.bin", 3),
    (41, 30, 22, "eac_r_random.bin", 4),
]


def fixture_data(fmt, width, height, seed):
    rng = random.Random(seed)
    if fmt != 34:
        return rng.randbytes(data_size(fmt, width, height))
    blocks = bytearray()
    while len(blocks) < data_size(fmt, width, height):
        block = rng.randbytes(8)
        if etc1_in_range(block):
            blocks += block
    return bytes(blocks)


def main():
    reference = BlockReference(sys.argv[1])
    output = pathlib.Path(sys.argv[2])
    lines = []
    for fmt, width, height, name, seed in FIXTURES:
        data = fixture_data(fmt, width, height, seed)
        (output / name).write_bytes(data)
        stored = reference.decode(fmt, width, height, data)
        stride = width * 4
        top_first = b"".join(stored[y * stride : (y + 1) * stride] for y in reversed(range(height)))
        lines.append(f"{fmt} {width} {height} {name} {hashlib.sha256(top_first).hexdigest()}")
    reference.close()
    listing = "\n".join(lines) + "\n"
    (output / "blocks.txt").write_text(listing, encoding="utf-8", newline="\n")
    print(listing, end="")


if __name__ == "__main__":
    main()
