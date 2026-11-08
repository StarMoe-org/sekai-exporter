"""Compare ACB reading with vgmstream, stream by stream.

Usage:
    python compare_acb.py <acb_streams> <decode_hca_stream> <vgmstream-cli> <directory>...

Each ACB file in the directories is read by both programs. For every stream of its memory bank
the script compares the decoded samples and the names of the cues that play the stream. It prints
the counts and exits with status 1 if anything differs.
"""

import pathlib
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict

from compare import decode_ours, wav_samples


def reference(tool, directory, path, subsong):
    target = directory / "stream.wav"
    subprocess.run(
        [tool, "-i", "-s", str(subsong), "-o", str(target), str(path)],
        check=True,
        capture_output=True,
    )
    return wav_samples(target)


def reference_names(tool, path, subsong):
    output = subprocess.run(
        [tool, "-m", "-s", str(subsong), str(path)],
        check=True,
        capture_output=True,
    ).stdout.decode("utf-8", "replace")
    for line in output.splitlines():
        if line.startswith("stream name: "):
            return set(line[len("stream name: ") :].split("; "))
    return set()


def main():
    lister, decoder, tool = sys.argv[1:4]
    process = subprocess.Popen([decoder], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    results = Counter()
    samples = []
    paths = [
        str(path)
        for root in sys.argv[4:]
        for path in sorted(pathlib.Path(root).iterdir())
        if path.suffix == ".acb"
    ]
    lines = subprocess.run([lister, *paths], check=True, capture_output=True).stdout.decode()
    streams = defaultdict(list)
    for line in lines.splitlines():
        kind, path, *rest = line.split(" ", 2)
        if kind == "stream":
            offset, length, _wave_id, *names = rest[0].split(" ", 3)
            streams[path].append((int(offset), int(length), names[0].split(";") if names and names[0] else []))
        elif kind == "sheet":
            results["sheets"] += 1
        else:
            results["sheets rejected"] += 1
            samples.append(line)

    with tempfile.TemporaryDirectory() as temporary:
        directory = pathlib.Path(temporary)
        for path, items in streams.items():
            data = pathlib.Path(path).read_bytes()
            for subsong, (offset, length, names) in enumerate(items, 1):
                rate, channels, ours, error = decode_ours(process, data[offset : offset + length])
                if error is not None:
                    results["rejected"] += 1
                    samples.append(f"{path} stream {subsong}: {error}")
                    continue
                theirs = reference(tool, directory, path, subsong)
                results["same samples" if (rate, channels, ours) == theirs else "different samples"] += 1
                expected = reference_names(tool, path, subsong)
                if expected == set(names):
                    results["same names"] += 1
                else:
                    results["different names"] += 1
                    if len(samples) < 10:
                        samples.append(f"{path} stream {subsong}: {sorted(names)} and {sorted(expected)}")
    process.stdin.close()
    print(dict(results))
    for line in samples[:10]:
        print("   ", line)
    bad = results["different samples"] + results["rejected"] + results["different names"] + results["sheets rejected"]
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
