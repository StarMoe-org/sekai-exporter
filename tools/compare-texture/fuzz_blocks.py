"""Compare the ETC, EAC and PVRTC decoders with the reference decoders on random images.

Usage:
    python fuzz_blocks.py <decode_stream executable> <block_decode executable> [images per format]

Random blocks reach every mode of each format; the image sizes are random too, so partial blocks
and, for PVRTC, the smallest stored sizes and non-square images are covered. Every image must
decode to the pixels of the reference. ETC1 has no meaning for a differential block whose colour
leaves the range, so an ETC1 image with such a block must be the one the decoder rejects.

Requires `block_decode`, built from `reference/block_decode.cpp`.
"""

import random
import subprocess
import sys
from collections import Counter

from block_reference import BlockReference, data_size, etc1_in_range
from decode_stream import decode_ours

FORMATS = [30, 31, 32, 33, 34, 41, 45, 47]


def random_size(rng, fmt):
    if fmt in (30, 31, 32, 33):
        return 1 << rng.randrange(8), 1 << rng.randrange(8)
    return rng.randrange(1, 41), rng.randrange(1, 41)


def main():
    process = subprocess.Popen([sys.argv[1]], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    reference = BlockReference(sys.argv[2])
    count = int(sys.argv[3]) if len(sys.argv) > 3 else 2000
    rng = random.Random(20261113)
    failures = 0
    for fmt in FORMATS:
        tally = Counter()
        for _ in range(count):
            width, height = random_size(rng, fmt)
            data = rng.randbytes(data_size(fmt, width, height))
            ours, error = decode_ours(process, fmt, width, height, data)
            in_range = fmt != 34 or all(etc1_in_range(data[i : i + 8]) for i in range(0, len(data), 8))
            if not in_range:
                outcome = "rejected out of range" if ours is None else "DIFF decoded out of range"
            elif ours is None:
                outcome = "DIFF rejected"
            else:
                outcome = "same" if ours == reference.decode(fmt, width, height, data) else "DIFF pixels"
            tally[outcome] += 1
            if outcome.startswith("DIFF"):
                failures += 1
                if failures <= 12:
                    print(outcome, fmt, width, height, error or "", data[:16].hex())
        print(f"format {fmt}", dict(tally), flush=True)
    process.stdin.close()
    reference.close()
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
