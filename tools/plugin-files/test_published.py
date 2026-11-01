"""Tests of published.py, with GitHub's list and the downloads made up.

    python3 -m unittest discover -s tools/plugin-files
"""

import json
import pathlib
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import published  # noqa: E402


def ppn(fingerprint, body, compiler=0x6A1F03C2, chip=5):
    """A plugin file's header with what it runs on, and a body."""
    header = bytearray(80)
    header[:4] = b"PPNP"
    struct.pack_into("<HHQ", header, 4, 2, chip, fingerprint)
    struct.pack_into("<I", header, 44, compiler)
    return bytes(header) + body


def release(tag, created, names):
    return {"tag_name": tag, "created_at": created, "draft": False,
            "assets": [{"name": n, "browser_download_url": f"https://r/{tag}/{n}"} for n in names]}


class Published(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.folder = pathlib.Path(self.dir.name)
        # beta.2 had vocabulary 1.0.0, its own release 1.0.1.
        self.list = json.dumps([[
            release("vocabulary-v1.0.1", "2026-10-25", ["vocabulary-1.0.1-x4.ppn", "vocabulary-1.0.1.zip"]),
            release("v0.5.0-beta.2", "2026-10-24", ["vocabulary-1.0.0-x4.ppn", "stats-1.0.0-x4.ppn"]),
        ]])
        self.online = {
            "https://r/vocabulary-v1.0.1/vocabulary-1.0.1-x4.ppn": ppn(0xF1F6, b"published"),
            "https://r/vocabulary-v1.0.1/vocabulary-1.0.1.zip": b"folder",
        }

    def tearDown(self):
        self.dir.cleanup()

    def test_github_s_pages_and_the_first_release_of_a_name(self):
        pages = self.list + json.dumps([release("v0.5.0-beta.1", "2026-10-20", ["stats-1.0.0-x4.ppn"])])
        found = published.assets(pages)
        self.assertEqual(found["stats-1.0.0-x4.ppn"][0], "v0.5.0-beta.1")
        self.assertEqual(found["vocabulary-1.0.1-x4.ppn"][0], "vocabulary-v1.0.1")

    def test_a_published_version_for_the_same_firmware_keeps_its_bytes(self):
        built = self.folder / "vocabulary-1.0.1-x4.ppn"
        built.write_bytes(ppn(0xF1F6, b"built again, other bytes"))
        fresh = self.folder / "vocabulary-1.0.2-x4.ppn"
        fresh.write_bytes(ppn(0xF1F6, b"new"))
        said = published.files(self.folder, published.assets(self.list), self.online.__getitem__)
        self.assertEqual(built.read_bytes(), ppn(0xF1F6, b"published"))
        self.assertEqual(fresh.read_bytes(), ppn(0xF1F6, b"new"))
        self.assertEqual(said, ["vocabulary-1.0.1-x4.ppn: as published in vocabulary-v1.0.1"])

    def test_another_firmware_gets_a_new_build_of_the_version(self):
        for other in (ppn(0xBEEF, b"x"), ppn(0xF1F6, b"x", compiler=1)):
            built = self.folder / "vocabulary-1.0.1-x4.ppn"
            built.write_bytes(other)
            published.files(self.folder, published.assets(self.list), self.online.__getitem__)
            self.assertEqual(built.read_bytes(), other)

    def test_versions_published_under_a_device_s_name_keep_their_bytes(self):
        # beta.12 and before named a file by device; its bytes are the chip's.
        names = ["hello-1.0.0-x4.ppn", "hello-1.0.0-x4-pro.ppn", "hello-1.0.0-x4-classic.ppn"]
        found = published.assets(json.dumps([[release("v0.5.0-beta.12", "2026-10-30", names)]]))
        online = {
            "https://r/v0.5.0-beta.12/hello-1.0.0-x4.ppn": ppn(0xA1, b"risc-v published"),
            "https://r/v0.5.0-beta.12/hello-1.0.0-x4-pro.ppn": ppn(0xB2, b"xtensa published", chip=9),
            "https://r/v0.5.0-beta.12/hello-1.0.0-x4-classic.ppn": ppn(0xB2, b"xtensa published", chip=9),
        }
        riscv = self.folder / "hello-1.0.0-esp32c3.ppn"
        riscv.write_bytes(ppn(0xA1, b"built again"))
        xtensa = self.folder / "hello-1.0.0-esp32s3.ppn"
        xtensa.write_bytes(ppn(0xB2, b"built again", chip=9))
        said = published.files(self.folder, found, online.__getitem__)
        self.assertEqual(riscv.read_bytes(), ppn(0xA1, b"risc-v published"))
        self.assertEqual(xtensa.read_bytes(), ppn(0xB2, b"xtensa published", chip=9))
        self.assertEqual(said, [
            "hello-1.0.0-esp32c3.ppn: as published in v0.5.0-beta.12 (hello-1.0.0-x4.ppn)",
            "hello-1.0.0-esp32s3.ppn: as published in v0.5.0-beta.12 (hello-1.0.0-x4-pro.ppn)",
        ])

    def test_a_chip_s_new_name_wins_over_the_old_ones_and_another_firmware_is_new(self):
        names = ["hello-1.0.0-esp32s3.ppn", "hello-1.0.0-x4-pro.ppn"]
        found = published.assets(json.dumps([[release("v0.6.0", "2026-11-05", names)]]))
        online = {
            "https://r/v0.6.0/hello-1.0.0-esp32s3.ppn": ppn(0xC3, b"new name", chip=9),
            "https://r/v0.6.0/hello-1.0.0-x4-pro.ppn": ppn(0xB2, b"old name", chip=9),
        }
        built = self.folder / "hello-1.0.0-esp32s3.ppn"
        built.write_bytes(ppn(0xB2, b"built", chip=9))
        # The old name holds the same firmware's file: taken from it, though
        # the new name has another firmware's.
        said = published.files(self.folder, found, online.__getitem__)
        self.assertEqual(built.read_bytes(), ppn(0xB2, b"old name", chip=9))
        self.assertIn("(hello-1.0.0-x4-pro.ppn)", said[0])
        built.write_bytes(ppn(0xD4, b"built", chip=9))
        said = published.files(self.folder, found, online.__getitem__)
        self.assertEqual(built.read_bytes(), ppn(0xD4, b"built", chip=9))
        self.assertIn("a new build", said[0])

    def test_a_published_folder_must_be_the_same(self):
        folder = self.folder / "vocabulary-1.0.1.zip"
        folder.write_bytes(b"folder")
        found = published.assets(self.list)
        self.assertEqual(published.folders(self.folder, found, self.online.__getitem__), [])
        folder.write_bytes(b"another How it works")
        self.assertEqual(len(published.folders(self.folder, found, self.online.__getitem__)), 1)


if __name__ == "__main__":
    unittest.main()
