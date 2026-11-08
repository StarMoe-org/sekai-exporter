"""Compare USM separation with WannaCRI.

Usage:
    python compare_usm.py <usm_streams> <directory>...

Every USM file in the directories is read by both programs. The script compares, per stream, the
bytes of the video frames and the audio chunks, and the count of frames marked as seek points or
key frames. It prints the counts and exits with status 1 if anything differs.

Requires WannaCRI (MIT).
"""

import hashlib
import pathlib
import subprocess
import sys
from collections import Counter

from wannacri.usm import OpMode, Usm


def reference(path):
    usm = Usm.open(str(path), encoding="shift-jis")
    found = {}
    for video in usm.videos:
        digest = hashlib.sha256()
        count = keys = size = 0
        for frame, is_key in video.stream(OpMode.NONE):
            digest.update(frame)
            size += len(frame)
            count += 1
            keys += bool(is_key)
        found["video", video.channel_number] = (count, size, digest.hexdigest(), keys)
    for audio in usm.audios:
        digest = hashlib.sha256()
        size = 0
        for chunk in audio.stream(OpMode.NONE):
            chunk = chunk[0] if isinstance(chunk, tuple) else chunk
            digest.update(chunk)
            size += len(chunk)
        found["audio", audio.channel_number] = (size, digest.hexdigest())
    return found


def main():
    results = Counter()
    samples = []
    for root in sys.argv[2:]:
        for path in sorted(pathlib.Path(root).iterdir()):
            if path.suffix != ".usm":
                continue
            output = subprocess.run([sys.argv[1], str(path)], check=True, capture_output=True).stdout.decode()
            theirs = reference(path)
            ours = {}
            for line in output.splitlines():
                fields = line.split()
                if fields[0] == "video":
                    ours["video", int(fields[2])] = (int(fields[8]), int(fields[9]), fields[10], int(fields[-1]))
                elif fields[0] == "audio":
                    ours["audio", int(fields[2])] = (int(fields[6]), fields[7])
                else:
                    results["rejected"] += 1
                    samples.append(line)
            for key, value in theirs.items():
                same = ours.get(key) == value
                results["same " + key[0] if same else "different " + key[0]] += 1
                if not same and len(samples) < 5:
                    samples.append(f"{path.name} {key}: {ours.get(key)} and {value}")
    print(dict(results))
    for line in samples:
        print("   ", line)
    sys.exit(1 if results["different video"] + results["different audio"] + results["rejected"] else 0)


if __name__ == "__main__":
    main()
