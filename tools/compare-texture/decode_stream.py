"""The texture decoder of `sekai-codec`, through its `decode_stream` example."""

import struct


def decode_ours(process, fmt, width, height, data):
    process.stdin.write(struct.pack("<iIII", fmt, width, height, len(data)) + data)
    process.stdin.flush()
    if process.stdout.read(1)[0] == 0:
        rows = process.stdout.read(width * height * 4)
        stride = width * 4
        return b"".join(rows[(height - 1 - y) * stride : (height - y) * stride] for y in range(height)), None
    length = struct.unpack("<I", process.stdout.read(4))[0]
    return None, process.stdout.read(length).decode()
