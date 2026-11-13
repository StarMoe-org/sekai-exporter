"""The reference decoders of `reference/block_decode.cpp`, for ETC, EAC and PVRTC."""

import struct
import subprocess


class BlockReference:
    """A running `block_decode`, which decodes with ETCPACK and the PowerVR SDK."""

    def __init__(self, executable):
        self.process = subprocess.Popen([executable], stdin=subprocess.PIPE, stdout=subprocess.PIPE)

    def decode(self, fmt, width, height, data):
        """RGBA in stored row order, or None for a format it does not decode."""
        self.process.stdin.write(struct.pack("<iIII", fmt, width, height, len(data)) + data)
        self.process.stdin.flush()
        length = struct.unpack("<I", self.process.stdout.read(4))[0]
        return self.process.stdout.read(length) if length else None

    def close(self):
        self.process.stdin.close()
        self.process.wait()


def data_size(fmt, width, height):
    """The bytes of the first image of a block format that `block_decode` covers."""
    if fmt in (34, 41, 45):
        return ((width + 3) // 4) * ((height + 3) // 4) * 8
    if fmt == 47:
        return ((width + 3) // 4) * ((height + 3) // 4) * 16
    if fmt in (30, 31):
        return max(width, 16) * max(height, 8) // 4
    if fmt in (32, 33):
        return max(width, 8) * max(height, 8) // 2
    raise ValueError(f"format {fmt}")


def etc1_in_range(block):
    """Whether an ETC1 block is valid: a differential block keeps each colour in range."""
    if not block[3] & 2:
        return True
    for channel in range(3):
        base, delta = block[channel] >> 3, block[channel] & 7
        if not 0 <= base + (delta - 8 if delta >= 4 else delta) <= 31:
            return False
    return True
