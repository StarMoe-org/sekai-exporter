"""Compare two directory trees file by file.

Usage:
    python compare.py A B [--known known.tsv] [--ffprobe PATH] [--report out.json]

A is the tree under test and B is the reference tree. Each relative path is classified as
missing (only in B), extra (only in A), same, or different. Files are judged by type: JSON by
value, images by decoded pixels, lossy media by their format contract, and every other file by
SHA-256. Paths that match a pattern in the known-differences file are reported under its reason.

Exit status: 0 if every missing, extra or different path is explained by the known-differences
file, 1 if any is not, 2 for usage errors.
"""

import argparse
import fnmatch
import hashlib
import io
import json
import math
import os
import re
import shutil
import struct
import subprocess
import sys

CATEGORIES = ("missing", "extra", "different", "same")
JSON_SUFFIXES = (".json",)
IMAGE_SUFFIXES = (".png", ".webp", ".jpg")
MEDIA_SUFFIXES = (".mp3", ".m4a", ".aac", ".mp4", ".ogg")
DEFAULT_DURATION_TOLERANCE = 0.05
BIT_RATE_STEP = 32000
MEDIA_STREAM_FIELDS = ("codec_type", "codec_name", "sample_rate", "channels", "width", "height")
JPEG_SOF_MARKERS = frozenset(
    [0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF]
)
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


class ProbeError(Exception):
    """ffprobe could not be run, or its output could not be used."""


def list_files(root):
    """Returns the sorted relative paths of all files under root, with forward slashes."""
    found = []
    for directory, subdirectories, names in os.walk(root):
        subdirectories.sort()
        for name in names:
            full = os.path.join(directory, name)
            found.append(os.path.relpath(full, root).replace(os.sep, "/"))
    return sorted(found)


def join_relative(root, rel):
    return os.path.join(root, *rel.split("/"))


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def same_bytes(path_a, path_b):
    if os.path.getsize(path_a) != os.path.getsize(path_b):
        return False
    return sha256_file(path_a) == sha256_file(path_b)


def load_pillow():
    """Returns the PIL.Image module, or None when Pillow is not installed."""
    try:
        from PIL import Image
    except ImportError:
        return None
    return Image


# JSON

def compare_json(data_a, data_b):
    try:
        value_a = json.loads(data_a)
    except ValueError as error:
        return "different", f"A is not valid JSON: {error}"
    try:
        value_b = json.loads(data_b)
    except ValueError as error:
        return "different", f"B is not valid JSON: {error}"
    difference = first_json_difference(value_a, value_b)
    if difference is None:
        return "same", "bytes differ, JSON values equal"
    return "different", difference


def first_json_difference(value_a, value_b, path="$"):
    """Returns a description of the first differing JSON path, or None when the values are equal.

    Object key order is ignored. Numbers compare as parsed values, and booleans never equal
    numbers.
    """
    if isinstance(value_a, bool) or isinstance(value_b, bool):
        if isinstance(value_a, bool) and isinstance(value_b, bool) and value_a == value_b:
            return None
        return _describe(path, value_a, value_b)
    if isinstance(value_a, dict) and isinstance(value_b, dict):
        for key in sorted(set(value_a) | set(value_b)):
            child = path + _key_step(key)
            if key not in value_b:
                return f"{child}: only in A"
            if key not in value_a:
                return f"{child}: only in B"
            found = first_json_difference(value_a[key], value_b[key], child)
            if found is not None:
                return found
        return None
    if isinstance(value_a, list) and isinstance(value_b, list):
        if len(value_a) != len(value_b):
            return f"{path}: length {len(value_a)} vs {len(value_b)}"
        for index, (item_a, item_b) in enumerate(zip(value_a, value_b)):
            found = first_json_difference(item_a, item_b, f"{path}[{index}]")
            if found is not None:
                return found
        return None
    if isinstance(value_a, (dict, list)) or isinstance(value_b, (dict, list)):
        return _describe(path, value_a, value_b)
    if value_a == value_b:
        return None
    return _describe(path, value_a, value_b)


def _key_step(key):
    if IDENTIFIER.fullmatch(key):
        return "." + key
    return "[" + json.dumps(key, ensure_ascii=False) + "]"


def _describe(path, value_a, value_b):
    return f"{path}: {_show(value_a)} vs {_show(value_b)}"


def _show(value):
    text = json.dumps(value, ensure_ascii=False, sort_keys=True)
    return text if len(text) <= 80 else text[:77] + "..."


# Images

def image_header(data):
    """Returns (format, width, height) read from the header of PNG, JPEG or WebP data, or None."""
    if data[:8] == PNG_SIGNATURE and len(data) >= 24:
        width, height = struct.unpack(">II", data[16:24])
        return "png", width, height
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP" and len(data) >= 30:
        chunk = data[12:16]
        if chunk == b"VP8 ":
            width, height = struct.unpack("<HH", data[26:30])
            return "webp", width & 0x3FFF, height & 0x3FFF
        if chunk == b"VP8L" and data[20] == 0x2F:
            bits = int.from_bytes(data[21:25], "little")
            return "webp", (bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1
        if chunk == b"VP8X":
            return "webp", 1 + int.from_bytes(data[24:27], "little"), 1 + int.from_bytes(data[27:30], "little")
        return None
    if data[:2] == b"\xff\xd8":
        position = 2
        while position + 4 <= len(data):
            if data[position] != 0xFF:
                return None
            marker = data[position + 1]
            if marker == 0xFF:
                position += 1
                continue
            if marker in (0x01, 0xD8) or 0xD0 <= marker <= 0xD7:
                position += 2
                continue
            if marker in JPEG_SOF_MARKERS:
                if position + 9 > len(data):
                    return None
                height, width = struct.unpack(">HH", data[position + 5 : position + 9])
                return "jpeg", width, height
            (length,) = struct.unpack(">H", data[position + 2 : position + 4])
            position += 2 + length
    return None


def _rgba(pillow, data):
    with pillow.open(io.BytesIO(data)) as image:
        return image.convert("RGBA")


def compare_image(data_a, data_b, pillow):
    """Compares decoded RGBA pixels when Pillow is given, otherwise format and size from headers."""
    if pillow is not None:
        try:
            image_a = _rgba(pillow, data_a)
            image_b = _rgba(pillow, data_b)
        except Exception as error:  # Pillow raises several exception types for undecodable input
            return "different", f"cannot decode: {error}"
        if image_a.size != image_b.size:
            return "different", f"size {image_a.size[0]}x{image_a.size[1]} vs {image_b.size[0]}x{image_b.size[1]}"
        pixels_a = image_a.tobytes()
        pixels_b = image_b.tobytes()
        if pixels_a == pixels_b:
            return "same", "bytes differ, pixels equal"
        changed = sum(
            1
            for offset in range(0, len(pixels_a), 4)
            if pixels_a[offset : offset + 4] != pixels_b[offset : offset + 4]
        )
        return "different", f"{changed} pixel(s) differ"
    header_a = image_header(data_a)
    header_b = image_header(data_b)
    if header_a is None or header_b is None:
        return "different", "bytes differ; image header not recognised"
    if header_a != header_b:
        return "different", f"header {_describe_header(header_a)} vs {_describe_header(header_b)}"
    return "different", "bytes differ; format and size equal, pixels not compared (Pillow is not installed)"


def _describe_header(header):
    kind, width, height = header
    return f"{kind} {width}x{height}"


# Media

def run_ffprobe(command, path):
    """Runs ffprobe on path. A command ending in .py runs with the current interpreter."""
    prefix = [sys.executable, command] if command.lower().endswith(".py") else [command]
    argv = prefix + ["-v", "error", "-show_streams", "-show_format", "-of", "json", str(path)]
    try:
        result = subprocess.run(argv, capture_output=True, check=False)
    except OSError as error:
        raise ProbeError(str(error)) from error
    if result.returncode != 0:
        message = result.stderr.decode("utf-8", "replace").strip()
        raise ProbeError(message or f"exit status {result.returncode}")
    try:
        probe = json.loads(result.stdout)
    except ValueError as error:
        raise ProbeError(f"invalid JSON output: {error}") from error
    if not isinstance(probe, dict):
        raise ProbeError("output is not a JSON object")
    return probe


def _number(value):
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _frame_rate(text):
    numerator, _, denominator = str(text or "").partition("/")
    try:
        top = float(numerator)
        bottom = float(denominator or 1)
    except ValueError:
        return None
    return top / bottom if top > 0 and bottom > 0 else None


def duration_tolerance(streams):
    """Largest frame duration among the streams: the duration may differ by this much."""
    values = []
    for stream in streams:
        kind = stream.get("codec_type")
        if kind == "video":
            rate = _frame_rate(stream.get("avg_frame_rate")) or _frame_rate(stream.get("r_frame_rate"))
            values.append(1 / rate if rate else DEFAULT_DURATION_TOLERANCE)
        elif kind == "audio":
            rate = _number(stream.get("sample_rate"))
            codec = stream.get("codec_name")
            if rate and codec == "aac":
                values.append(1024 / rate)
            elif rate and codec == "mp3":
                values.append(1152 / rate)
            else:
                values.append(DEFAULT_DURATION_TOLERANCE)
    return max(values, default=DEFAULT_DURATION_TOLERANCE)


def _bit_rate_tier(value):
    rate = _number(value)
    if rate is None:
        return None
    return math.floor(rate / BIT_RATE_STEP + 0.5)


def first_media_difference(probe_a, probe_b):
    """Returns (field, description) for the first differing field of the format contract, or None.

    The duration tolerance is taken from the streams of the reference tree (probe_b).
    """
    format_a = probe_a.get("format") or {}
    format_b = probe_b.get("format") or {}
    streams_a = probe_a.get("streams") or []
    streams_b = probe_b.get("streams") or []
    if format_a.get("format_name") != format_b.get("format_name"):
        return "format_name", f"{_show(format_a.get('format_name'))} vs {_show(format_b.get('format_name'))}"
    if len(streams_a) != len(streams_b):
        return "streams", f"count {len(streams_a)} vs {len(streams_b)}"
    for index, (stream_a, stream_b) in enumerate(zip(streams_a, streams_b)):
        for field in MEDIA_STREAM_FIELDS:
            if stream_a.get(field) != stream_b.get(field):
                return f"streams[{index}].{field}", f"{_show(stream_a.get(field))} vs {_show(stream_b.get(field))}"
    duration_a = _number(format_a.get("duration"))
    duration_b = _number(format_b.get("duration"))
    if duration_a is None or duration_b is None:
        if duration_a != duration_b:
            return "duration", f"{_show(format_a.get('duration'))} vs {_show(format_b.get('duration'))}"
    else:
        tolerance = duration_tolerance(streams_b)
        if abs(duration_a - duration_b) > tolerance + 1e-9:
            return "duration", f"{duration_a} vs {duration_b} (tolerance {tolerance:.6f} s)"
    tier_a = _bit_rate_tier(format_a.get("bit_rate"))
    tier_b = _bit_rate_tier(format_b.get("bit_rate"))
    if tier_a != tier_b:
        return "bit_rate", f"32 kb/s tiers {tier_a} vs {tier_b}"
    return None


def compare_media(path_a, path_b, ffprobe):
    if ffprobe is None:
        return "different", "bytes differ; ffprobe not available"
    try:
        probe_a = run_ffprobe(ffprobe, path_a)
        probe_b = run_ffprobe(ffprobe, path_b)
    except ProbeError as error:
        return "different", f"ffprobe failed: {error}"
    difference = first_media_difference(probe_a, probe_b)
    if difference is None:
        return "same", "bytes differ, format contract equal"
    field, description = difference
    return "different", f"{field}: {description}"


# Classification and trees

def classify(path_a, path_b, rel, ffprobe=None, pillow=None):
    """Returns (category, detail) for one path present in both trees; category is same or different."""
    if same_bytes(path_a, path_b):
        return "same", None
    suffix = os.path.splitext(rel)[1].lower()
    if suffix in MEDIA_SUFFIXES:
        return compare_media(path_a, path_b, ffprobe)
    with open(path_a, "rb") as handle:
        data_a = handle.read()
    with open(path_b, "rb") as handle:
        data_b = handle.read()
    if suffix in JSON_SUFFIXES:
        return compare_json(data_a, data_b)
    if suffix in IMAGE_SUFFIXES:
        return compare_image(data_a, data_b, pillow)
    return "different", f"bytes differ ({len(data_a)} vs {len(data_b)} bytes)"


def load_known(path):
    """Reads pattern<TAB>reason lines; blank lines and lines starting with # are skipped."""
    rules = []
    with open(path, encoding="utf-8") as handle:
        for number, line in enumerate(handle, 1):
            text = line.rstrip("\r\n")
            if not text.strip() or text.startswith("#"):
                continue
            pattern, separator, reason = text.partition("\t")
            if not separator or not pattern or not reason.strip():
                raise ValueError(f"{path}:{number}: expected pattern<TAB>reason")
            rules.append((pattern, reason.strip()))
    return rules


def known_reason(rel, rules):
    for pattern, reason in rules:
        if fnmatch.fnmatchcase(rel, pattern):
            return reason
    return None


def compare_trees(root_a, root_b, known_rules=(), ffprobe=None, pillow=None):
    """Classifies every relative path found in either tree. Returns entries sorted by path."""
    files_a = set(list_files(root_a))
    files_b = set(list_files(root_b))
    entries = []
    for rel in sorted(files_a | files_b):
        if rel not in files_b:
            category, detail = "extra", "only in A"
        elif rel not in files_a:
            category, detail = "missing", "only in B"
        else:
            category, detail = classify(
                join_relative(root_a, rel), join_relative(root_b, rel), rel, ffprobe, pillow
            )
        known = known_reason(rel, known_rules) if category != "same" else None
        entries.append({"path": rel, "category": category, "detail": detail, "known": known})
    return entries


def count_categories(entries):
    counts = {category: 0 for category in CATEGORIES}
    for entry in entries:
        counts[entry["category"]] += 1
    return counts


def unexplained(entries):
    return [entry for entry in entries if entry["category"] != "same" and entry["known"] is None]


def group_by_category(entries):
    """Returns {category: {top-level directory: [entries]}}; files at the root use the key '.'."""
    groups = {category: {} for category in CATEGORIES}
    for entry in entries:
        top = entry["path"].split("/", 1)[0] if "/" in entry["path"] else "."
        groups[entry["category"]].setdefault(top, []).append(entry)
    return {category: dict(sorted(groups[category].items())) for category in CATEGORIES}


def describe(entry):
    line = f"{entry['path']}"
    if entry["detail"]:
        line += f" -- {entry['detail']}"
    if entry["known"] is not None:
        line += f" [known: {entry['known']}]"
    elif entry["category"] != "same":
        line += " [UNEXPLAINED]"
    return line


def print_report(entries):
    counts = count_categories(entries)
    print(f"{len(entries)} paths: " + ", ".join(f"{category} {counts[category]}" for category in CATEGORIES))
    for category, groups in group_by_category(entries).items():
        sections = []
        for top, items in groups.items():
            shown = [entry for entry in items if category != "same" or entry["detail"]]
            if shown:
                sections.append((top, shown))
        if not sections:
            continue
        print(f"{category}:")
        for top, items in sections:
            print(f"  {top}/" if top != "." else "  (top level)")
            for entry in items:
                print(f"    {describe(entry)}")
    print(f"unexplained: {len(unexplained(entries))}")


def write_report(path, entries):
    report = {
        "counts": count_categories(entries),
        "unexplained": len(unexplained(entries)),
        "groups": group_by_category(entries),
    }
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        json.dump(report, handle, indent=2, sort_keys=True, ensure_ascii=False)
        handle.write("\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description="Compare two directory trees file by file.")
    parser.add_argument("tree_a", metavar="A", help="tree under test")
    parser.add_argument("tree_b", metavar="B", help="reference tree")
    parser.add_argument("--known", metavar="FILE", help="known-differences TSV: pattern<TAB>reason")
    parser.add_argument(
        "--ffprobe",
        metavar="PATH",
        help="ffprobe executable, or a .py script run with the current interpreter (default: ffprobe on PATH)",
    )
    parser.add_argument("--report", metavar="FILE", help="write the grouped report as JSON")
    args = parser.parse_args(argv)
    for root in (args.tree_a, args.tree_b):
        if not os.path.isdir(root):
            parser.error(f"not a directory: {root}")
    try:
        rules = load_known(args.known) if args.known else []
    except (OSError, ValueError) as error:
        parser.error(str(error))
    ffprobe = args.ffprobe or shutil.which("ffprobe")
    entries = compare_trees(args.tree_a, args.tree_b, rules, ffprobe, load_pillow())
    print_report(entries)
    if args.report:
        write_report(args.report, entries)
    return 1 if unexplained(entries) else 0


if __name__ == "__main__":
    sys.exit(main())
