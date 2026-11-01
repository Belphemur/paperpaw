"""Tests of handbook.py, on this checkout:

    python3 -m unittest discover -s tools -p "test_handbook.py"
"""

import contextlib
import io
import pathlib
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import handbook  # noqa: E402


class Handbook(unittest.TestCase):
    def test_tests_at_a_file_s_end_are_left_out(self):
        code = "fn a() {}\n"
        tests = "#[cfg(test)]\nmod tests {\n    fn b() { let _ = '{'; }\n}\n"
        self.assertEqual(handbook.without_tests(code + "\n" + tests), code + "\n")
        more = "\n#[cfg(test)]\nmod hostile_tests;\n"
        self.assertEqual(handbook.without_tests(code + "\n" + tests + more), code + "\n")
        # Where something follows that is no test, the file stays whole.
        after = code + "\n" + tests + "\nfn c() {}\n"
        self.assertEqual(handbook.without_tests(after), after)
        self.assertEqual(handbook.without_tests(code), code)

    def test_it_holds_the_sdk_and_the_plugins_without_tests_or_assets(self):
        with tempfile.TemporaryDirectory() as out:
            registry = pathlib.Path(out, "registry.json")
            registry.write_text("{}\n", encoding="utf-8")
            book = pathlib.Path(out, "handbook.zip")
            with contextlib.redirect_stdout(io.StringIO()):
                handbook.main(str(registry), "test", str(book))
            with zipfile.ZipFile(book) as z:
                names = z.namelist()
                readme = z.read("paperpaw-handbook-test/README.md").decode("utf-8")
                battery = z.read("paperpaw-handbook-test/plugins/battery/src/lib.rs").decode("utf-8")
            size = book.stat().st_size
        top = "paperpaw-handbook-test/"
        for name in ["api/paperpaw-sdk.txt", "src/paperpaw-sdk/lib.rs", "docs/plugins.md",
                     "plugins/battery/Cargo.toml", "plugins/battery/how/en.txt",
                     "plugins/battery/i18n/en.ini"]:
            self.assertIn(top + name, names)
        for name in names:
            parts = pathlib.PurePosixPath(name).parts
            self.assertFalse({"tests", "testdata", "snapshots", "target"} & set(parts), name)
            self.assertNotIn(pathlib.PurePosixPath(name).suffix, handbook.ASSETS, name)
            self.assertFalse(name.endswith(("tests.rs", "_tests.rs")), name)
        # Neither the kernel nor a shell.
        self.assertFalse(any(n.startswith((top + "src/paperpaw-core/", top + "shells/")) for n in names))
        self.assertNotIn("#[cfg(test)]\nmod tests {", battery)
        for part in ["what a plugin may use", "examples of good solutions", "left out on"]:
            self.assertIn(part, readme)
        # Grown past 3 MB with the plugins' docs and sources (flash cards, store).
        self.assertLess(size, 3_200_000, "near 3 MB")


if __name__ == "__main__":
    unittest.main()
