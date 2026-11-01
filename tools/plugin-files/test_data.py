"""Tests of data.toml's script packs: the ones offered in the store, and the
one that is ready but not offered yet (its lines commented out until the
owner decides):

    python3 -m unittest discover -s tools/plugin-files
"""

import pathlib
import sys
import tomllib
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import data  # noqa: E402

READY = "# Ready for the store, not offered yet"

# The versions they are at now: a change of a pack bumps it here too.
PACKS = {
    "script-arabic": "1.1.0",
    "script-hebrew": "1.1.0",
    "script-greek": "1.0.0",
    "script-cyrillic": "1.0.0",
}


def ready():
    """The entries below the marker's text, their comment signs taken away."""
    text = (data.HERE / "data.toml").read_text(encoding="utf-8")
    block = text[text.index(READY):]
    block = block[block.index("# # script-korean"):]
    lines = [line[2:] if line.startswith("# ") else "" for line in block.splitlines()]
    return tomllib.loads("\n".join(lines))


class ScriptPacks(unittest.TestCase):
    def test_they_are_offered(self):
        self.assertTrue(set(PACKS) <= set(data.plugins()))

    def test_each_is_complete(self):
        listed = data.plugins()
        for pack, version in PACKS.items():
            fields = listed[pack]
            self.assertEqual(fields["version"], version, pack)
            self.assertEqual(fields["build"], "tools/script-pack.py")
            self.assertTrue(fields["off_by_default"])
            self.assertEqual(fields["licence"], "OFL-1.1")
            # Its name and description in both languages, its files there.
            for key in ["name", "description"]:
                self.assertTrue(fields[key] and fields["de"][key], (pack, key))
            for path in [fields["build"], *fields["inputs"]]:
                self.assertTrue((data.ROOT / path).exists(), (pack, path))
            # What the pack builder writes for it.
            self.assertIn(f"id = {pack}\n", data.plugin_ini(pack, fields))


class Ready(unittest.TestCase):
    def test_it_is_not_offered(self):
        self.assertNotIn("script-korean", data.plugins())

    def test_the_korean_pack_is_ready(self):
        fields = ready()["script-korean"]
        self.assertEqual(fields["version"], "1.1.0")
        self.assertEqual(fields["build"], "tools/script-pack.py")
        self.assertTrue(fields["off_by_default"])
        self.assertEqual(fields["licence"], "OFL-1.1")
        for key in ["name", "description"]:
            self.assertTrue(fields[key] and fields["de"][key])
        # Its keyboard is among what its build reads.
        self.assertIn("tools/plugin-files/keyboards/korean.txt", fields["inputs"])
        for path in [fields["build"], *fields["inputs"]]:
            self.assertTrue((data.ROOT / path).exists(), path)
        self.assertIn("id = script-korean\n", data.plugin_ini("script-korean", fields))
        # The font's licence reserves its name: the store's texts do not
        # use it (NOTICE).
        texts = [fields["name"], fields["description"], *fields["de"].values()]
        self.assertFalse([t for t in texts if "Nanum" in t])


if __name__ == "__main__":
    unittest.main()
