"""Unit tests for compare.py and determinism.py.

Run from the repository root with:
    python -B -m unittest discover -s tools/compare-storage -v

Every tree is generated in a temporary directory from synthetic bytes. Tests that need Pillow
are skipped when it is not installed.
"""

import contextlib
import io
import json
import os
import struct
import tempfile
import unittest

import compare
import determinism

PIL_AVAILABLE = compare.load_pillow() is not None

FAKE_FFPROBE = """import json
import os
import sys

if "-show_format" not in sys.argv or "-show_streams" not in sys.argv:
    sys.exit(3)
here = os.path.dirname(os.path.abspath(__file__))
with open(os.path.join(here, "probe_data.json"), encoding="utf-8") as handle:
    table = json.load(handle)
key = os.path.basename(os.path.dirname(sys.argv[-1])) + "/" + os.path.basename(sys.argv[-1])
sys.stdout.write(json.dumps(table[key]))
"""


def png_header(width, height):
    """Only a PNG signature and IHDR chunk: enough for the header-level checks."""
    return (
        b"\x89PNG\r\n\x1a\n"
        + b"\x00\x00\x00\x0dIHDR"
        + struct.pack(">II", width, height)
        + b"\x08\x06\x00\x00\x00\x00\x00\x00\x00"
    )


def jpeg_header(width, height):
    """Start of image and a baseline frame header: enough for the header-level checks."""
    return b"\xff\xd8\xff\xc0" + struct.pack(">HBHH", 8, 8, height, width) + b"\x01\x01\x11\x00"


def webp_vp8x_header(width, height):
    """A RIFF WEBP container with an extended VP8X chunk."""
    return (
        b"RIFF"
        + struct.pack("<I", 22)
        + b"WEBP"
        + b"VP8X"
        + struct.pack("<I", 10)
        + b"\x00\x00\x00\x00"
        + (width - 1).to_bytes(3, "little")
        + (height - 1).to_bytes(3, "little")
    )


def media_probe(codec="aac", sample_rate="44100", duration="1.000", bit_rate="128000"):
    return {
        "format": {
            "format_name": "mov,mp4,m4a,3gp,3g2,mj2",
            "duration": duration,
            "bit_rate": bit_rate,
        },
        "streams": [
            {"codec_type": "audio", "codec_name": codec, "sample_rate": sample_rate, "channels": 2}
        ],
    }


class TreeTestCase(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = temporary.name

    def build(self, files_a, files_b):
        """Writes the two trees as root/A and root/B and returns their paths."""
        roots = (os.path.join(self.root, "A"), os.path.join(self.root, "B"))
        for base, files in zip(roots, (files_a, files_b)):
            os.makedirs(base, exist_ok=True)
            for rel, data in files.items():
                path = os.path.join(base, *rel.split("/"))
                os.makedirs(os.path.dirname(path), exist_ok=True)
                if isinstance(data, str):
                    data = data.encode("utf-8")
                with open(path, "wb") as handle:
                    handle.write(data)
        return roots

    def entries(self, files_a, files_b, **options):
        root_a, root_b = self.build(files_a, files_b)
        return {entry["path"]: entry for entry in compare.compare_trees(root_a, root_b, **options)}

    def fake_ffprobe(self, table):
        tools = os.path.join(self.root, "tools")
        os.makedirs(tools, exist_ok=True)
        with open(os.path.join(tools, "probe_data.json"), "w", encoding="utf-8") as handle:
            json.dump(table, handle)
        script = os.path.join(tools, "fake_ffprobe.py")
        with open(script, "w", encoding="utf-8") as handle:
            handle.write(FAKE_FFPROBE)
        return script

    def run_main(self, argv):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = compare.main(argv)
        return code, output.getvalue()


class CompareTreeTests(TreeTestCase):
    def test_equal_bytes_are_same(self):
        result = self.entries({"a.bin": b"\x00\x01"}, {"a.bin": b"\x00\x01"})
        self.assertEqual(result["a.bin"]["category"], "same")

    def test_entries_are_sorted_by_path(self):
        result = self.entries({"z.bin": b"1", "b/x.bin": b"1"}, {"a.bin": b"1", "b/x.bin": b"1"})
        self.assertEqual(list(result), ["a.bin", "b/x.bin", "z.bin"])

    def test_json_key_order_and_number_forms_are_ignored(self):
        result = self.entries(
            {"v.json": '{"x": 1, "y": [1, 2.0]}'},
            {"v.json": '{"y": [1.0, 2], "x": 1}'},
        )
        self.assertEqual(result["v.json"]["category"], "same")

    def test_json_value_difference_reports_first_path(self):
        result = self.entries(
            {"v.json": '{"a": [0, 0, 0, {"b": 1}]}'},
            {"v.json": '{"a": [0, 0, 0, {"b": 2}]}'},
        )
        self.assertEqual(result["v.json"]["category"], "different")
        self.assertTrue(result["v.json"]["detail"].startswith("$.a[3].b:"))

    def test_json_boolean_is_not_a_number(self):
        result = self.entries({"v.json": '{"f": true}'}, {"v.json": '{"f": 1}'})
        self.assertEqual(result["v.json"]["category"], "different")
        self.assertTrue(result["v.json"]["detail"].startswith("$.f:"))

    def test_invalid_json_is_different(self):
        result = self.entries({"v.json": "{"}, {"v.json": "{}"})
        self.assertEqual(result["v.json"]["category"], "different")
        self.assertIn("not valid JSON", result["v.json"]["detail"])

    def test_missing_and_extra_paths(self):
        result = self.entries({"only_a.txt": "a"}, {"only_b.txt": "b"})
        self.assertEqual(result["only_a.txt"]["category"], "extra")
        self.assertEqual(result["only_b.txt"]["category"], "missing")

    def test_byte_different_binary_is_different(self):
        result = self.entries({"d.bin": b"\x00\x01"}, {"d.bin": b"\x00\x02"})
        self.assertEqual(result["d.bin"]["category"], "different")

    def test_image_header_formats(self):
        self.assertEqual(compare.image_header(png_header(3, 5)), ("png", 3, 5))
        self.assertEqual(compare.image_header(jpeg_header(7, 9)), ("jpeg", 7, 9))
        self.assertEqual(compare.image_header(webp_vp8x_header(11, 13)), ("webp", 11, 13))
        self.assertIsNone(compare.image_header(b"plain text, not an image"))

    def test_image_without_pillow_is_not_judged_same(self):
        result = self.entries(
            {"p.png": png_header(3, 5) + b"a", "j.jpg": jpeg_header(2, 2) + b"a"},
            {"p.png": png_header(3, 5) + b"b", "j.jpg": jpeg_header(2, 2) + b"b"},
            pillow=None,
        )
        self.assertEqual(result["p.png"]["category"], "different")
        self.assertIn("pixels not compared", result["p.png"]["detail"])
        self.assertEqual(result["j.jpg"]["category"], "different")

    def test_image_without_pillow_size_mismatch_is_different(self):
        result = self.entries(
            {"p.png": png_header(3, 5)},
            {"p.png": png_header(4, 5)},
            pillow=None,
        )
        self.assertEqual(result["p.png"]["category"], "different")
        self.assertIn("png 3x5 vs png 4x5", result["p.png"]["detail"])

    @unittest.skipUnless(PIL_AVAILABLE, "Pillow is not installed")
    def test_image_pillow_counts_differing_pixels(self):
        from PIL import Image

        base = Image.new("RGBA", (4, 4), (10, 20, 30, 255))
        changed = base.copy()
        changed.putpixel((1, 2), (0, 0, 0, 255))
        buffer_a, buffer_b = io.BytesIO(), io.BytesIO()
        base.save(buffer_a, "PNG")
        changed.save(buffer_b, "PNG")
        result = self.entries(
            {"p.png": buffer_a.getvalue()},
            {"p.png": buffer_b.getvalue()},
            pillow=compare.load_pillow(),
        )
        self.assertEqual(result["p.png"]["category"], "different")
        self.assertEqual(result["p.png"]["detail"], "1 pixel(s) differ")

    @unittest.skipUnless(PIL_AVAILABLE, "Pillow is not installed")
    def test_image_pillow_equal_pixels_are_same_despite_encoding(self):
        from PIL import Image

        image = Image.new("RGBA", (16, 16), (200, 100, 50, 255))
        buffer_a, buffer_b = io.BytesIO(), io.BytesIO()
        image.save(buffer_a, "PNG", compress_level=0)
        image.save(buffer_b, "PNG", compress_level=9)
        self.assertNotEqual(buffer_a.getvalue(), buffer_b.getvalue())
        result = self.entries(
            {"p.png": buffer_a.getvalue()},
            {"p.png": buffer_b.getvalue()},
            pillow=compare.load_pillow(),
        )
        self.assertEqual(result["p.png"]["category"], "same")

    def test_media_same_contract_with_different_bytes_is_same(self):
        script = self.fake_ffprobe({"A/clip.m4a": media_probe(), "B/clip.m4a": media_probe()})
        result = self.entries({"clip.m4a": b"AAAA"}, {"clip.m4a": b"BBBB"}, ffprobe=script)
        self.assertEqual(result["clip.m4a"]["category"], "same")

    def test_media_codec_difference_is_reported_by_field(self):
        script = self.fake_ffprobe({"A/clip.m4a": media_probe(codec="mp3"), "B/clip.m4a": media_probe()})
        result = self.entries({"clip.m4a": b"AAAA"}, {"clip.m4a": b"BBBB"}, ffprobe=script)
        self.assertEqual(result["clip.m4a"]["category"], "different")
        self.assertTrue(result["clip.m4a"]["detail"].startswith("streams[0].codec_name:"))

    def test_media_duration_within_one_aac_frame_is_same(self):
        # One AAC frame at 44100 Hz is 1024 / 44100 s, about 0.0232 s.
        script = self.fake_ffprobe(
            {"A/clip.m4a": media_probe(duration="1.000"), "B/clip.m4a": media_probe(duration="1.010")}
        )
        result = self.entries({"clip.m4a": b"AAAA"}, {"clip.m4a": b"BBBB"}, ffprobe=script)
        self.assertEqual(result["clip.m4a"]["category"], "same")

    def test_media_duration_beyond_one_frame_is_different(self):
        script = self.fake_ffprobe(
            {"A/clip.m4a": media_probe(duration="1.000"), "B/clip.m4a": media_probe(duration="1.100")}
        )
        result = self.entries({"clip.m4a": b"AAAA"}, {"clip.m4a": b"BBBB"}, ffprobe=script)
        self.assertEqual(result["clip.m4a"]["category"], "different")
        self.assertTrue(result["clip.m4a"]["detail"].startswith("duration:"))

    def test_media_bit_rate_compared_in_32_kbps_tiers(self):
        script = self.fake_ffprobe(
            {"A/clip.m4a": media_probe(bit_rate="128000"), "B/clip.m4a": media_probe(bit_rate="127000")}
        )
        result = self.entries({"clip.m4a": b"AAAA"}, {"clip.m4a": b"BBBB"}, ffprobe=script)
        self.assertEqual(result["clip.m4a"]["category"], "same")

        script = self.fake_ffprobe(
            {"A/clip.m4a": media_probe(bit_rate="128000"), "B/clip.m4a": media_probe(bit_rate="160000")}
        )
        result = self.entries({"clip.m4a": b"AAAA"}, {"clip.m4a": b"BBBB"}, ffprobe=script)
        self.assertEqual(result["clip.m4a"]["category"], "different")
        self.assertTrue(result["clip.m4a"]["detail"].startswith("bit_rate:"))

    def test_media_without_ffprobe_is_different(self):
        result = self.entries({"clip.mp3": b"AAAA"}, {"clip.mp3": b"BBBB"}, ffprobe=None)
        self.assertEqual(result["clip.mp3"]["category"], "different")
        self.assertIn("ffprobe not available", result["clip.mp3"]["detail"])

    def test_ffprobe_script_runs_through_command_line(self):
        script = self.fake_ffprobe({"A/clip.mp4": media_probe(), "B/clip.mp4": media_probe()})
        root_a, root_b = self.build({"clip.mp4": b"AAAA"}, {"clip.mp4": b"BBBB"})
        code, output = self.run_main([root_a, root_b, "--ffprobe", script])
        self.assertEqual(code, 0)
        self.assertIn("unexplained: 0", output)

    def test_known_differences_explain_matching_paths(self):
        files_a = {"extra/x.bin": b"1", "common.bin": b"a"}
        files_b = {"gone.txt": b"2", "common.bin": b"b"}
        known = os.path.join(self.root, "known.tsv")
        with open(known, "w", encoding="utf-8") as handle:
            handle.write("# reviewed differences\n\nextra/*\tonly in the tree under test\n")
            handle.write("common.bin\tkept on purpose\n")
        result = self.entries(files_a, files_b, known_rules=compare.load_known(known))
        self.assertEqual(result["extra/x.bin"]["known"], "only in the tree under test")
        self.assertEqual(result["common.bin"]["known"], "kept on purpose")
        self.assertIsNone(result["gone.txt"]["known"])
        unexplained = compare.unexplained(list(result.values()))
        self.assertEqual([entry["path"] for entry in unexplained], ["gone.txt"])

    def test_known_file_parse_error_is_usage_error(self):
        known = os.path.join(self.root, "bad.tsv")
        with open(known, "w", encoding="utf-8") as handle:
            handle.write("no tab on this line\n")
        root_a, root_b = self.build({"a": "1"}, {"a": "1"})
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
            compare.main([root_a, root_b, "--known", known])
        self.assertEqual(caught.exception.code, 2)

    def test_exit_status_follows_unexplained_paths(self):
        root_a, root_b = self.build({"a.bin": b"1"}, {"a.bin": b"2"})
        code, output = self.run_main([root_a, root_b])
        self.assertEqual(code, 1)
        self.assertIn("[UNEXPLAINED]", output)

        known = os.path.join(self.root, "known.tsv")
        with open(known, "w", encoding="utf-8") as handle:
            handle.write("a.*\tcovered by test\n")
        code, output = self.run_main([root_a, root_b, "--known", known])
        self.assertEqual(code, 0)
        self.assertIn("[known: covered by test]", output)

    def test_report_groups_by_category_and_top_directory(self):
        root_a, root_b = self.build(
            {"sub/x.json": '{"v": 1}', "top.bin": b"1", "sub/only.txt": "q"},
            {"sub/x.json": '{"v": 2}', "top.bin": b"1"},
        )
        report_path = os.path.join(self.root, "report.json")
        code, _ = self.run_main([root_a, root_b, "--report", report_path])
        self.assertEqual(code, 1)
        with open(report_path, encoding="utf-8") as handle:
            report = json.load(handle)
        self.assertEqual(report["counts"], {"missing": 0, "extra": 1, "different": 1, "same": 1})
        self.assertEqual(report["unexplained"], 2)
        self.assertEqual([entry["path"] for entry in report["groups"]["different"]["sub"]], ["sub/x.json"])
        self.assertEqual([entry["path"] for entry in report["groups"]["extra"]["sub"]], ["sub/only.txt"])

    def test_missing_directory_is_usage_error(self):
        root_a, _ = self.build({"a": "1"}, {"a": "1"})
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
            compare.main([root_a, os.path.join(self.root, "does-not-exist")])
        self.assertEqual(caught.exception.code, 2)


class DeterminismTests(TreeTestCase):
    def runs(self, files1, files2):
        run1 = os.path.join(self.root, "run1")
        run2 = os.path.join(self.root, "run2")
        for base, files in ((run1, files1), (run2, files2)):
            os.makedirs(base, exist_ok=True)
            for rel, data in files.items():
                path = os.path.join(base, *rel.split("/"))
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, "wb") as handle:
                    handle.write(data)
        return run1, run2

    def run_main(self, argv):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = determinism.main(argv)
        return code, output.getvalue()

    def write_metrics(self, text):
        path = os.path.join(self.root, "metrics.jsonl")
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(text)
        return path

    def test_identical_runs_pass(self):
        run1, run2 = self.runs({"d/a.bin": b"1", "b.txt": b"x"}, {"d/a.bin": b"1", "b.txt": b"x"})
        code, output = self.run_main([run1, run2])
        self.assertEqual(code, 0)
        self.assertIn("differing 0", output)

    def test_differing_and_missing_files_fail(self):
        run1, run2 = self.runs({"d/a.bin": b"1", "only1.bin": b"x"}, {"d/a.bin": b"2"})
        code, output = self.run_main([run1, run2])
        self.assertEqual(code, 1)
        self.assertIn("differs: d/a.bin", output)
        self.assertIn("missing in RUN2: only1.bin", output)

    def test_convert_metrics_fail_the_run(self):
        run1, run2 = self.runs({"a.bin": b"1"}, {"a.bin": b"1"})
        metrics = self.write_metrics(
            '{"kind": "span", "stage": "plan"}\n{"kind": "span", "stage": "convert"}\n'
            '{"kind": "total", "stage": "convert"}\n{"kind": "span", "stage": "convert"}\n'
        )
        code, output = self.run_main([run1, run2, "--metrics", metrics])
        self.assertEqual(code, 1)
        self.assertIn("recomputed derivations: 2", output)

    def test_metrics_without_convert_lines_pass(self):
        run1, run2 = self.runs({"a.bin": b"1"}, {"a.bin": b"1"})
        metrics = self.write_metrics('{"kind": "span", "stage": "plan"}\n\n{"kind": "total", "stage": "convert"}\n')
        code, output = self.run_main([run1, run2, "--metrics", metrics])
        self.assertEqual(code, 0)
        self.assertIn("recomputed derivations: 0", output)

    def test_skip_publication_leaves_out_publication_documents(self):
        documents = {
            f"{n}.json": b'{"format": "%s"}' % name.encode()
            for n, name in enumerate(["index-root", "index-chunk", "index-shard", "path-index"])
        }
        documents["aa/idx.sqlite"] = b"SQLite format 3\x00"
        documents["bb/chunk.paths"] = b"SKPI"
        same = {"a.json": b'{"format": "asset"}'}
        run1, run2 = self.runs({**same, **documents}, same)
        self.assertEqual(self.run_main([run1, run2])[0], 1)
        code, output = self.run_main([run1, run2, "--skip-publication"])
        self.assertEqual(code, 0)
        self.assertIn("RUN1 1 files, RUN2 1 files", output)

    def test_skip_publication_still_compares_other_documents(self):
        run1, run2 = self.runs(
            {"a.json": b'{"format": "asset"}', "b.bin": b"1"}, {"b.bin": b"1"}
        )
        code, output = self.run_main([run1, run2, "--skip-publication"])
        self.assertEqual(code, 1)
        self.assertIn("missing in RUN2: a.json", output)

    def test_invalid_metrics_line_is_usage_error(self):
        run1, run2 = self.runs({"a.bin": b"1"}, {"a.bin": b"1"})
        metrics = self.write_metrics("{not json\n")
        with contextlib.redirect_stderr(io.StringIO()):
            code, _ = self.run_main([run1, run2, "--metrics", metrics])
        self.assertEqual(code, 2)


if __name__ == "__main__":
    unittest.main()
