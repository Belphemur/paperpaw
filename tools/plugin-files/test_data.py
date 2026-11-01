"""Tests of data.toml's script packs (offered in the store):

    python3 -m unittest discover -s tools/plugin-files
"""

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import data  # noqa: E402

# The versions they are at now: a change of a pack bumps it here too.
PACKS = {
    "script-arabic": "1.1.0",
    "script-hebrew": "1.1.0",
    "script-greek": "1.0.0",
    "script-cyrillic": "1.0.0",
}


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


if __name__ == "__main__":
    unittest.main()
