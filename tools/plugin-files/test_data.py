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
        self.assertFalse({"script-greek", "script-korean"} & set(data.plugins()))

    def test_the_greek_pack_is_ready(self):
        fields = ready()["script-greek"]
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
        self.assertIn("id = script-greek\n", data.plugin_ini("script-greek", fields))

    def test_the_korean_pack_is_ready(self):
        fields = ready()["script-korean"]
        self.assertEqual(fields["version"], "1.0.0")
        self.assertEqual(fields["build"], "tools/script-pack.py")
        self.assertTrue(fields["off_by_default"])
        self.assertEqual(fields["licence"], "OFL-1.1")
        for key in ["name", "description"]:
            self.assertTrue(fields[key] and fields["de"][key])
        for path in [fields["build"], *fields["inputs"]]:
            self.assertTrue((data.ROOT / path).exists(), path)
        self.assertIn("id = script-korean\n", data.plugin_ini("script-korean", fields))
        # The font's licence reserves its name: the store's texts do not
        # use it (NOTICE).
        texts = [fields["name"], fields["description"], *fields["de"].values()]
        self.assertFalse([t for t in texts if "Nanum" in t])


if __name__ == "__main__":
    unittest.main()
