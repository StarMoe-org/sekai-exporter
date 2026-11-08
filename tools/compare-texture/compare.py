"""Count the textures of bundles and compare their decoding with reference decoders.

Usage:
    python compare.py <decode_stream executable> <bundle directory>...

Every texture in the directories is read with UnityPy. The script prints the number of textures
per format and class, the facts that decide how a format is decoded, and for each format how many
distinct images decode to the same bytes as the reference, differ, or are rejected.

References: ASTC is compared with ARM's astcenc through astc-encoder-py, other block formats with
texture2ddecoder, and uncompressed formats with a direct expansion of the bytes.

Requires UnityPy (MIT), astc-encoder-py (MIT) and texture2ddecoder (MIT).
"""

import hashlib
import pathlib
import struct
import subprocess
import sys
from collections import Counter, defaultdict

import astc_encoder as ae
import texture2ddecoder as t2d
import UnityPy

CLASSES = {"Texture2D", "Cubemap", "Texture2DArray", "CubemapArray", "Texture3D"}
ASTC = {48: 4, 49: 5, 50: 6, 51: 8, 52: 10, 53: 12}
BLOCK_DECODERS = {
    10: t2d.decode_bc1,
    12: t2d.decode_bc3,
    26: t2d.decode_bc4,
    27: t2d.decode_bc5,
    25: t2d.decode_bc7,
    34: t2d.decode_etc1,
    41: t2d.decode_eacr,
    42: t2d.decode_eacr_signed,
    43: t2d.decode_eacrg,
    44: t2d.decode_eacrg_signed,
    45: t2d.decode_etc2,
    46: t2d.decode_etc2a1,
    47: t2d.decode_etc2a8,
}
INVERTED = b"\x10\x00\x00\x00"
PREFIXED = b"\x20\x00\x00\x00"


def deobfuscate(data):
    if data[:4] == PREFIXED:
        return data[4:]
    if data[:4] == INVERTED:
        body = bytearray(data[4:])
        for i in range(128):
            if i % 8 < 5:
                body[i] ^= 0xFF
        return bytes(body)
    return data


def swap_red_blue(pixels):
    out = bytearray(pixels)
    out[0::4], out[2::4] = pixels[2::4], pixels[0::4]
    return bytes(out)


def raw_rgba(fmt, width, height, data):
    count = width * height
    if fmt == 4:
        return data[: count * 4]
    if fmt == 3:
        out = bytearray(b"\xff" * (count * 4))
        for c in range(3):
            out[c::4] = data[c : count * 3 : 3]
        return bytes(out)
    if fmt == 7:
        out = bytearray()
        for i in range(count):
            v = data[i * 2] | data[i * 2 + 1] << 8
            r, g, b = v >> 11, v >> 5 & 63, v & 31
            out += bytes([r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2, 255])
        return bytes(out)
    if fmt == 1:
        out = bytearray(count * 4)
        out[3::4] = data[:count]
        return bytes(out)
    return None


def reference(contexts, fmt, width, height, data):
    if fmt in ASTC:
        size = ASTC[fmt]
        if size not in contexts:
            config = ae.ASTCConfig(ae.ASTCProfile.LDR, size, size, block_z=1, quality=0)
            contexts[size] = ae.ASTCContext(config)
        image = ae.ASTCImage(ae.ASTCType.U8, width, height, 1)
        columns = (width + size - 1) // size
        rows = (height + size - 1) // size
        contexts[size].decompress(data[: columns * rows * 16], image, ae.ASTCSwizzle.from_str("RGBA"))
        return bytes(image.data)
    if fmt in BLOCK_DECODERS:
        return swap_red_blue(BLOCK_DECODERS[fmt](data, width, height))
    return raw_rgba(fmt, width, height, data)


def decode_ours(process, fmt, width, height, data):
    process.stdin.write(struct.pack("<iIII", fmt, width, height, len(data)) + data)
    process.stdin.flush()
    if process.stdout.read(1)[0] == 0:
        rows = process.stdout.read(width * height * 4)
        stride = width * 4
        return b"".join(rows[(height - 1 - y) * stride : (height - y) * stride] for y in range(height)), None
    length = struct.unpack("<I", process.stdout.read(4))[0]
    return None, process.stdout.read(length).decode()


def main():
    UnityPy.config.FALLBACK_UNITY_VERSION = "2022.3.62f2"
    process = subprocess.Popen([sys.argv[1]], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    contexts = {}
    counts = Counter()
    families = defaultdict(set)
    sizes = defaultdict(list)
    facts = defaultdict(Counter)
    results = defaultdict(Counter)
    samples = defaultdict(list)
    seen = set()
    bundles = unreadable = 0
    for root in sys.argv[2:]:
        for path in sorted(pathlib.Path(root).iterdir()):
            if not path.is_file() or path.suffix in (".lock", ".part", ".json"):
                continue
            try:
                env = UnityPy.load(deobfuscate(path.read_bytes()))
                objects = [o for o in env.objects if o.type.name in CLASSES]
            except Exception:
                unreadable += 1
                continue
            bundles += 1
            family = path.name.split("__")[0]
            for obj in objects:
                try:
                    tex = obj.read()
                except Exception as error:
                    facts["read errors"][f"{obj.type.name}: {type(error).__name__}"] += 1
                    continue
                if obj.type.name != "Texture2D" and obj.type.name != "Cubemap":
                    key = f"{obj.type.name} GraphicsFormat {tex.m_Format}"
                    counts[key] += 1
                    families[key].add(family)
                    continue
                fmt = int(tex.m_TextureFormat)
                key = f"{fmt}"
                counts[key] += 1
                families[key].add(family)
                facts["class"][f"{obj.type.name} format {fmt}"] += 1
                facts["colour space"][getattr(tex, "m_ColorSpace", None)] += 1
                facts["platform blob"][bool(getattr(tex, "m_PlatformBlob", None))] += 1
                facts["image count"][getattr(tex, "m_ImageCount", None)] += 1
                facts["mip count"][getattr(tex, "m_MipCount", None)] += 1
                width, height = tex.m_Width, tex.m_Height
                sizes[key].append((width * height, width, height))
                # A dynamic font's texture is 0 x 0, and its stream data has an empty path.
                stream = getattr(tex, "m_StreamData", None)
                if not width or not height or not (tex.image_data or (stream and stream.path)):
                    results[key]["no image data"] += 1
                    continue
                data = bytes(tex.get_image_data())
                digest = hashlib.sha256(struct.pack("<iII", fmt, width, height) + data).digest()
                if digest in seen:
                    continue
                seen.add(digest)
                ours, error = decode_ours(process, fmt, width, height, data)
                if ours is None:
                    results[key]["rejected" if "unsupported texture format" not in error else "no decoder"] += 1
                    if len(samples[key]) < 3:
                        samples[key].append(f"{path.name} {tex.m_Name}: {error}")
                    continue
                theirs = reference(contexts, fmt, width, height, data)
                if theirs is None:
                    results[key]["no reference"] += 1
                elif ours == theirs:
                    results[key]["same"] += 1
                else:
                    results[key]["different"] += 1
                    if len(samples[key]) < 3:
                        changed = sum(a != b for a, b in zip(ours, theirs))
                        samples[key].append(f"{path.name} {tex.m_Name}: {changed} of {len(ours)} bytes differ")
    process.stdin.close()

    print(f"{bundles} bundles read, {unreadable} files not read")
    failed = 0
    for key, count in counts.most_common():
        shown = sorted(sizes[key])
        extent = f", {shown[0][1]}x{shown[0][2]} to {shown[-1][1]}x{shown[-1][2]}" if shown else ""
        print(f"format {key}: {count} textures in {len(families[key])} families{extent}: {dict(results[key])}")
        failed += results[key]["different"] + results[key]["rejected"]
        for line in samples[key]:
            print("   ", line)
    for name, values in facts.items():
        print(name, dict(values))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
