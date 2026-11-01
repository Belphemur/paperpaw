"""Tests of data.toml's entries that are ready but not offered yet (their
lines commented out until the owner decides):

    python3 -m unittest discover -s tools/plugin-files
"""

import pathlib
import sys
import tomllib
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import data  # noqa: E402

READY = "# Ready for the store, not offered yet"


def ready():
    """The entries below the marker's text, their comment signs taken away."""
    text = (data.HERE / "data.toml").read_text(encoding="utf-8")
    block = text[text.index(READY):]
    block = block[block.index("# ["):]
    lines = [line[2:] if line.startswith("# ") else "" for line in block.splitlines()]
    return tomllib.loads("\n".join(lines))


class Ready(unittest.TestCase):
    def test_none_of_them_is_offered(self):
        self.assertFalse({"script-greek", "script-cyrillic"} & set(data.plugins()))

    def test_the_packs_of_literata_are_ready(self):
        for plugin in ["script-greek", "script-cyrillic"]:
            with self.subTest(plugin):
                self.check(plugin)

    def check(self, plugin):
        fields = ready()[plugin]
        self.assertEqual(fields["version"], "1.0.0")
        self.assertEqual(fields["build"], "tools/script-pack.py")
        self.assertTrue(fields["off_by_default"])
        self.assertEqual(fields["licence"], "OFL-1.1")
        # Its name and description in both languages, its files there.
        for key in ["name", "description"]:
            self.assertTrue(fields[key] and fields["de"][key])
        for path in [fields["build"], *fields["inputs"]]:
            self.assertTrue((data.ROOT / path).exists(), path)
        # What the pack builder writes for it.
        self.assertIn(f"id = {plugin}\n", data.plugin_ini(plugin, fields))


if __name__ == "__main__":
    unittest.main()
