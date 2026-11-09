"""Compare the ASTC HDR decoder with ARM's astcenc on random blocks and on images.

Usage:
    python fuzz_astc_hdr.py <decode_stream executable> [blocks per footprint] [image...]

The reference is ARM's astcenc in the HDR profile, compared as binary16 values. An image argument
is a file of 6x6 blocks named `<anything>_<width>x<height>.astc`. Random blocks are steered towards
the HDR endpoint formats. A block the decoder rejects must be one astcenc decodes to the error
colour. The colours of constant blocks are finite and not negative, because the binary16 output of
the binding replaces negative values.

Requires astc-encoder-py (MIT), a binding of astcenc (Apache-2.0).
"""

import pathlib
import random
import struct
import subprocess
import sys
from collections import Counter

import astc_encoder as ae

FOOTPRINTS = {66: 4, 67: 5, 68: 6, 69: 8, 70: 10, 71: 12}
ERROR = 0xFFFF


class Reference:
    def __init__(self):
        self.contexts = {}

    def decode(self, footprint, width, height, blocks):
        if footprint not in self.contexts:
            config = ae.ASTCConfig(ae.ASTCProfile.HDR, footprint, footprint, block_z=1, quality=0)
            self.contexts[footprint] = ae.ASTCContext(config)
        image = ae.ASTCImage(ae.ASTCType.F16, width, height, 1)
        self.contexts[footprint].decompress(blocks, image, ae.ASTCSwizzle.from_str("RGBA"))
        return bytes(image.data)


class Ours:
    def __init__(self, exe):
        self.process = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE)

    def decode(self, fmt, width, height, blocks):
        self.process.stdin.write(struct.pack("<iIII", fmt, width, height, len(blocks)) + blocks)
        self.process.stdin.flush()
        if self.process.stdout.read(1)[0] == 0:
            rows = self.process.stdout.read(width * height * 8)
            stride = width * 8
            # The decoder returns rows from top to bottom; the reference has stored order.
            return b"".join(rows[(height - 1 - y) * stride:(height - y) * stride] for y in range(height)), None
        length = struct.unpack("<I", self.process.stdout.read(4))[0]
        return None, self.process.stdout.read(length).decode()


def random_block(rng):
    kind = rng.random()
    value = rng.getrandbits(128)
    if kind < 0.1:
        # Constant blocks of both kinds, with all-ones or random extents.
        low = 0x1FC | rng.getrandbits(1) << 9 | 3 << 10
        if rng.random() < 0.7:
            low |= 0x1FFFFFFFFFFFFF << 12
        else:
            low |= rng.getrandbits(52) << 12
        color = value >> 64 & 0x7BFF_7BFF_7BFF_7BFF
        return (color << 64 | low).to_bytes(16, "little")
    if kind < 0.9:
        hdr = [2, 3, 7, 11, 14, 15]
        partitions = value >> 11 & 3
        if partitions == 0:
            value = value & ~(0xF << 13) | rng.choice(hdr) << 13
        else:
            # All partitions share one format.
            value = value & ~(0x3F << 23) | rng.choice(hdr) << 25
    return value.to_bytes(16, "little")


def main():
    ours, reference = Ours(sys.argv[1]), Reference()
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 200_000
    rng = random.Random(20261109)
    failures = 0
    for fmt, size in FOOTPRINTS.items():
        tally = Counter()
        for _ in range(count):
            block = random_block(rng)
            mine, error = ours.decode(fmt, size, size, block)
            theirs = reference.decode(size, size, size, block)
            if mine is None:
                if all(v == ERROR for v in struct.unpack(f"<{size * size * 4}H", theirs)):
                    tally["rejected, error colour"] += 1
                else:
                    tally["DIFF rejected only by ours"] += 1
                    failures += 1
                    if failures <= 12:
                        print("rejected", size, block.hex(), error)
            elif mine == theirs:
                tally["same"] += 1
            else:
                tally["DIFF pixels"] += 1
                failures += 1
                if failures <= 12:
                    print("pixels", size, block.hex())
        print(f"{size}x{size}", dict(tally), flush=True)
    for path in map(pathlib.Path, sys.argv[3:]):
        width, height = map(int, path.stem.rsplit("_", 1)[1].split("x"))
        blocks = path.read_bytes()
        mine, error = ours.decode(68, width, height, blocks)
        theirs = reference.decode(6, width, height, blocks)
        if mine != theirs:
            failures += 1
            print("image differs", path.name, error)
        else:
            print("image same", path.name)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
