"""Compare the ASTC decoder with ARM's astcenc on random blocks.

Usage:
    python fuzz_astc.py <decode_stream executable> [blocks per footprint]

The blocks are random bit patterns steered towards legal encodings, so every block mode,
partition count, endpoint format and integer sequence range is reached. The script decodes each
block with both decoders and prints the counts per footprint and the first differences. A block
the decoder rejects must be one astcenc decodes to an error colour (magenta, or white for an
illegal block mode), or one with HDR endpoints, whose partitions astcenc decodes to the error
colour in the LDR profile.

Requires astc-encoder-py (MIT), a binding of astcenc (Apache-2.0).
"""

import random
import struct
import subprocess
import sys
from collections import Counter

import astc_encoder as ae

FOOTPRINTS = {48: (4, 4), 49: (5, 5), 50: (6, 6), 51: (8, 8), 52: (10, 10), 53: (12, 12)}
ERROR_TEXELS = (bytes([255, 0, 255, 255]), bytes([255, 255, 255, 255]))


def contexts(width, height):
    config = ae.ASTCConfig(ae.ASTCProfile.LDR, width, height, block_z=1, quality=0)
    return ae.ASTCContext(config)


def reference(context, block, width, height):
    image = ae.ASTCImage(ae.ASTCType.U8, width, height, 1)
    context.decompress(block, image, ae.ASTCSwizzle.from_str("RGBA"))
    return bytes(image.data)


def random_block(rng):
    kind = rng.random()
    value = rng.getrandbits(128)
    if kind < 0.02:
        # Constant blocks: LDR, with all-ones or random extents.
        low = 0x1FC | 3 << 10
        if rng.random() < 0.7:
            low |= 0x1FFFFFFFFFFFFF << 12
        else:
            low |= rng.getrandbits(52) << 12
        return (value >> 64 << 64 | low).to_bytes(16, "little")
    if kind < 0.9:
        # Keep the endpoint formats away from the HDR ones often enough to stay legal.
        ldr = [0, 1, 4, 5, 6, 8, 9, 10, 12, 13]
        partitions = value >> 11 & 3
        if partitions == 0:
            value = value & ~(0xF << 13) | rng.choice(ldr) << 13
        elif rng.random() < 0.5:
            # All partitions share one format.
            value = value & ~(0x3F << 23) | rng.choice(ldr) << 25
    return value.to_bytes(16, "little")


def main():
    exe = sys.argv[1]
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 200_000
    process = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    rng = random.Random(20261108)
    failures = 0
    for fmt, (width, height) in FOOTPRINTS.items():
        context = contexts(width, height)
        size = width * height * 4
        tally = Counter()
        for _ in range(count):
            block = random_block(rng)
            process.stdin.write(struct.pack("<iIII", fmt, width, height, 16) + block)
            process.stdin.flush()
            status = process.stdout.read(1)[0]
            if status == 0:
                rows = process.stdout.read(size)
                # The decoder returns rows from top to bottom; the reference has stored order.
                ours = b"".join(rows[(height - 1 - y) * width * 4:(height - y) * width * 4] for y in range(height))
                error = None
            else:
                length = struct.unpack("<I", process.stdout.read(4))[0]
                error = process.stdout.read(length).decode()
                ours = None
            theirs = reference(context, block, width, height)
            has_error_colour = any(theirs == texel * (width * height) for texel in ERROR_TEXELS)
            if ours is None:
                if has_error_colour:
                    tally["rejected, error colour"] += 1
                elif "HDR" in error:
                    tally["rejected, HDR endpoints in some partitions"] += 1
                else:
                    tally["DIFF rejected only by ours"] += 1
                    failures += 1
                    if failures <= 12:
                        print("rejected", width, height, block.hex(), error)
            elif ours == theirs:
                tally["same"] += 1
            else:
                tally["DIFF pixels"] += 1
                failures += 1
                if failures <= 12:
                    delta = Counter(a - b for a, b in zip(ours, theirs))
                    print("pixels", width, height, block.hex(), delta.most_common(4), "error colour" if has_error_colour else "")
        print(f"{width}x{height}", dict(tally), flush=True)
    process.stdin.close()
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
