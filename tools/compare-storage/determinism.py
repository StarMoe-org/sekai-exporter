"""Check that two output directories of the same input are byte for byte identical.

Usage:
    python determinism.py RUN1 RUN2 [--metrics FILE] [--skip-publication]

Every file in RUN1 must exist in RUN2 with the same bytes, and the reverse. With --metrics, FILE
is the metrics file of the second run, JSON Lines with one object per line. Each span of the
"convert" stage ({"kind": "span", "stage": "convert"}) is one recomputed derivation, and their
count must be 0. With --skip-publication, the documents of a publication (index roots, index
shards and listings) are left out of both directories, so that the store of a run that published
can be compared with the store of a run that did not.

Exit status: 0 if everything matches, 1 otherwise, 2 for usage errors.
"""

import argparse
import json
import os
import sys

from compare import join_relative, list_files, same_bytes

PUBLICATION_FORMATS = {"index-root", "index-shard", "listing"}


def is_publication(path):
    if not path.endswith(".json"):
        return False
    try:
        with open(path, encoding="utf-8") as handle:
            document = json.load(handle)
    except ValueError:
        return False
    return isinstance(document, dict) and document.get("format") in PUBLICATION_FORMATS


def count_convert_lines(path):
    count = 0
    with open(path, encoding="utf-8") as handle:
        for number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                record = json.loads(line)
            except ValueError as error:
                raise ValueError(f"{path}:{number}: {error}") from error
            if (
                isinstance(record, dict)
                and record.get("kind") == "span"
                and record.get("stage") == "convert"
            ):
                count += 1
    return count


def main(argv=None):
    parser = argparse.ArgumentParser(description="Compare two output directories byte for byte.")
    parser.add_argument("run1", metavar="RUN1", help="first output directory")
    parser.add_argument("run2", metavar="RUN2", help="second output directory")
    parser.add_argument(
        "--metrics", metavar="FILE", help="JSON Lines metrics file; convert lines must not occur"
    )
    parser.add_argument(
        "--skip-publication",
        action="store_true",
        help="leave out index roots, index shards and listings",
    )
    args = parser.parse_args(argv)
    for root in (args.run1, args.run2):
        if not os.path.isdir(root):
            parser.error(f"not a directory: {root}")

    count = None
    if args.metrics:
        try:
            count = count_convert_lines(args.metrics)
        except (OSError, ValueError) as error:
            print(f"error: {error}", file=sys.stderr)
            return 2

    files1 = list_files(args.run1)
    files2 = list_files(args.run2)
    if args.skip_publication:
        files1 = [rel for rel in files1 if not is_publication(join_relative(args.run1, rel))]
        files2 = [rel for rel in files2 if not is_publication(join_relative(args.run2, rel))]
    set1, set2 = set(files1), set(files2)
    failed = False
    for rel in sorted(set1 - set2):
        print(f"missing in RUN2: {rel}")
        failed = True
    for rel in sorted(set2 - set1):
        print(f"missing in RUN1: {rel}")
        failed = True
    differing = 0
    for rel in sorted(set1 & set2):
        if not same_bytes(join_relative(args.run1, rel), join_relative(args.run2, rel)):
            print(f"differs: {rel}")
            differing += 1
    if differing:
        failed = True
    print(f"RUN1 {len(files1)} files, RUN2 {len(files2)} files, differing {differing}")
    if count is not None:
        print(f"recomputed derivations: {count}")
        if count:
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
