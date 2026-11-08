"""Compare HCA decoding sample by sample with vgmstream.

Usage:
    python compare.py <decode_audio_stream executable> <vgmstream-cli> <directory>...

Every HCA stream found inside the files of the directories (ACB or AWB containers, or bare HCA
files) is decoded by both programs. The script prints how many distinct streams decode to the
same samples, differ, or are rejected, and exits with status 1 if any differ or are rejected.
"""

import hashlib
import pathlib
import re
import struct
import subprocess
import sys
import tempfile
from collections import Counter

SIGNATURE = re.compile(rb"HCA\x00|\xC8\xC3\xC1\x80")


def streams(data):
    """Yields each HCA stream of a container, using the sizes in the stream's own header."""
    for match in SIGNATURE.finditer(data):
        offset = match.start()
        version, size = struct.unpack(">HH", data[offset + 4 : offset + 8])
        if version not in (0x200, 0x300) or not 0x20 <= size <= 0x1000:
            continue
        header = bytes(b & 0x7F for b in data[offset : offset + size])
        fmt, comp = header.find(b"fmt\0"), header.find(b"comp")
        if fmt < 0 or comp < 0:
            continue
        frames = int.from_bytes(data[offset + fmt + 8 : offset + fmt + 12], "big")
        frame_size = int.from_bytes(data[offset + comp + 4 : offset + comp + 6], "big")
        end = offset + size + frames * frame_size
        if end <= len(data):
            yield data[offset:end]


def wav_samples(path):
    data = path.read_bytes()
    position = 12
    channels = rate = None
    while position + 8 <= len(data):
        name = data[position : position + 4]
        size = struct.unpack("<I", data[position + 4 : position + 8])[0]
        if name == b"fmt ":
            channels, rate = struct.unpack("<HI", data[position + 10 : position + 16])
        if name == b"data":
            return rate, channels, data[position + 8 : position + 8 + size]
        position += 8 + size + (size & 1)
    raise ValueError("no data chunk")


def decode_ours(process, stream):
    process.stdin.write(struct.pack("<I", len(stream)) + stream)
    process.stdin.flush()
    if process.stdout.read(1)[0] == 0:
        rate, channels, length = struct.unpack("<III", process.stdout.read(12))
        return rate, channels, process.stdout.read(length), None
    length = struct.unpack("<I", process.stdout.read(4))[0]
    return None, None, None, process.stdout.read(length).decode()


def decode_reference(tool, directory, stream):
    source = directory / ("stream.adx" if stream[:2] == b"\x80\x00" else "stream.hca")
    target = directory / "stream.wav"
    source.write_bytes(stream)
    subprocess.run([tool, "-i", "-o", str(target), str(source)], check=True, capture_output=True)
    return wav_samples(target)


def main():
    process = subprocess.Popen([sys.argv[1]], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    results = Counter()
    samples = []
    seen = set()
    with tempfile.TemporaryDirectory() as temporary:
        directory = pathlib.Path(temporary)
        for root in sys.argv[3:]:
            for path in sorted(pathlib.Path(root).iterdir()):
                if not path.is_file():
                    continue
                data = path.read_bytes()
                for stream in [data] if data[:2] == b"\x80\x00" else streams(data):
                    digest = hashlib.sha256(stream).digest()
                    if digest in seen:
                        continue
                    seen.add(digest)
                    rate, channels, ours, error = decode_ours(process, stream)
                    if error is not None:
                        results["rejected"] += 1
                        if len(samples) < 5:
                            samples.append(f"{path.name}: {error}")
                        continue
                    theirs_rate, theirs_channels, theirs = decode_reference(sys.argv[2], directory, stream)
                    if (rate, channels, ours) == (theirs_rate, theirs_channels, theirs):
                        results["same"] += 1
                        continue
                    results["different"] += 1
                    if len(samples) < 5:
                        count = min(len(ours), len(theirs)) // 2
                        pairs = zip(struct.unpack(f"<{count}h", ours[: count * 2]),
                                    struct.unpack(f"<{count}h", theirs[: count * 2]))
                        changed = [(i, a, b) for i, (a, b) in enumerate(pairs) if a != b]
                        first = changed[0] if changed else None
                        samples.append(
                            f"{path.name}: {len(ours)} and {len(theirs)} bytes, "
                            f"{len(changed)} samples differ, first {first}"
                        )
    process.stdin.close()
    print(f"{len(seen)} distinct streams: {dict(results)}")
    for line in samples:
        print("   ", line)
    sys.exit(1 if results["different"] or results["rejected"] else 0)


if __name__ == "__main__":
    main()
