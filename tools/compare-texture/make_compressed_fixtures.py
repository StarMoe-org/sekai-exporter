"""Write the synthetic crunch and ASTC HDR fixtures of the texture decoder tests.

Usage:
    python make_compressed_fixtures.py <crunch encoder> <crunch_decode> <output directory>

The images are generated here. The crunch encoder is built from Unity's crunch library
(Unity-Technologies/crunch, branch `unity`) and `crunch_decode` from `reference/crunch_decode.cpp`;
ASTC HDR is compressed and decoded with astcenc through astc-encoder-py (MIT). `compressed.txt`
records format, width, height, file name and the SHA-256 of the reference pixels, which are RGBA
bytes for crunch and little-endian binary16 values for HDR, rows from top to bottom.

Requires UnityPy (MIT) and astc-encoder-py (MIT).
"""

import hashlib
import math
import pathlib
import struct
import subprocess
import sys
import tempfile

import astc_encoder as ae

from compare_crunch import CRUNCHED_DXT5, decode_blocks

CRUNCH_SIZE = (34, 17)
HDR = {66: (4, 20, 20), 68: (6, 26, 14), 71: (12, 30, 25)}
SWIZZLE = ae.ASTCSwizzle.from_str("RGBA")


def tga(width, height):
    """An uncompressed 32-bit TGA of gradients and hard edges, with varying alpha."""
    head = struct.pack("<BBBHHBHHHHBB", 0, 0, 2, 0, 0, 0, 0, 0, width, height, 32, 0x28)
    body = bytearray()
    for y in range(height):
        for x in range(width):
            edge = 255 if (x // 5 + y // 3) % 2 else 40
            body += bytes([x * 255 // width, y * 255 // height, edge, (x ^ y) * 7 & 255])
    return head + bytes(body)


def crunch_fixture(encoder, decoder, output):
    width, height = CRUNCH_SIZE
    with tempfile.TemporaryDirectory() as temporary:
        directory = pathlib.Path(temporary)
        source, target = directory / "in.tga", directory / "out.crn"
        source.write_bytes(tga(width, height))
        subprocess.run(
            [encoder, "-file", str(source), "-out", str(target), "-DXT5", "-fileformat", "crn", "-mipMode", "None"],
            check=True,
            capture_output=True,
        )
        data = target.read_bytes()
    input_bytes = struct.pack("<I", len(data)) + data
    reply = subprocess.run([decoder], input=input_bytes, check=True, capture_output=True).stdout
    size = struct.unpack("<I", reply[:4])[0]
    assert size, "the reference decoder rejected the file"
    pixels = decode_blocks(reply[4 : 4 + size], width, height)
    name = "crunch_dxt5.crn"
    (output / name).write_bytes(data)
    return f"{CRUNCHED_DXT5} {width} {height} {name} {hashlib.sha256(pixels).hexdigest()}"


def hdr_image(width, height):
    values = []
    for y in range(height):
        for x in range(width):
            wave = math.sin(x * 0.7) * math.cos(y * 0.5)
            values += [4.0 * x / width, 2.0 + 2.0 * wave, 0.25 + 8.0 * y / height, 0.5 + 0.5 * wave]
    return struct.pack(f"<{len(values)}f", *values)


def hdr_fixture(fmt, footprint, width, height, output):
    config = ae.ASTCConfig(ae.ASTCProfile.HDR, footprint, footprint, block_z=1, quality=60)
    context = ae.ASTCContext(config)
    image = ae.ASTCImage(ae.ASTCType.F32, width, height, 1, hdr_image(width, height))
    blocks = bytes(context.compress(image, SWIZZLE))
    decoded = ae.ASTCImage(ae.ASTCType.F16, width, height, 1)
    context.decompress(blocks, decoded, SWIZZLE)
    stored = bytes(decoded.data)
    stride = width * 8
    pixels = b"".join(stored[(height - 1 - y) * stride : (height - y) * stride] for y in range(height))
    name = f"astc_hdr_{footprint}x{footprint}.bin"
    (output / name).write_bytes(blocks)
    return f"{fmt} {width} {height} {name} {hashlib.sha256(pixels).hexdigest()}"


def main():
    encoder, decoder, output = sys.argv[1], sys.argv[2], pathlib.Path(sys.argv[3])
    lines = [crunch_fixture(encoder, decoder, output)]
    lines += [hdr_fixture(fmt, *shape, output) for fmt, shape in sorted(HDR.items())]
    (output / "compressed.txt").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
