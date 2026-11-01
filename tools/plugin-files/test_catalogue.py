"""Tests of catalogue.py, without a build: plugin files made up here.

    python3 -m unittest discover -s tools/plugin-files
"""

import contextlib
import io
import pathlib
import struct
import sys
import tempfile
import unittest
import unittest.mock
import zipfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import catalogue  # noqa: E402

KEY = bytes(range(32))


def signed(fingerprint):
    """A plugin file as catalogue.py reads it: the header with its
    fingerprint, and a signature's trailer with the author's key."""
    header = b"PPNP" + bytes(4) + struct.pack("<Q", fingerprint)
    header += bytes(80 - len(header))
    trailer = b"PPNS" + bytes(8) + KEY
    return header + trailer + bytes(catalogue.SIGNATURE_LEN - len(trailer))


def built(dist):
    """A file for the X4's chip of each plugin of the store, named by its
    version."""
    for plugin, _ in catalogue.plugins():
        version = catalogue.plugin_version(plugin)
        dist.joinpath(f"{plugin}-{version}-esp32c3.ppn").write_bytes(signed(0x85073A61323717FF))


class HowItWorks(unittest.TestCase):
    def test_every_plugin_of_the_store_has_one_in_english(self):
        for plugin, _ in catalogue.plugins():
            files = dict(catalogue.how_files(plugin))
            self.assertIn("how/en.txt", files, plugin)
            self.assertTrue(all("\r" not in text for text in files.values()), plugin)

    def test_only_languages_are_taken_with_lf(self):
        with tempfile.TemporaryDirectory() as root:
            how = pathlib.Path(root, "demo", "how")
            how.mkdir(parents=True)
            how.joinpath("en.txt").write_bytes(b"First.\r\n\r\nSecond.\r\n")
            how.joinpath("de.txt").write_text("Erstens.\n", encoding="utf-8")
            how.joinpath("README.txt").write_text("Not a language.\n", encoding="utf-8")
            how.joinpath("en.md").write_text("Not a text file.\n", encoding="utf-8")
            files = catalogue.how_files("demo", pathlib.Path(root))
        self.assertEqual(files, [("how/de.txt", "Erstens.\n"), ("how/en.txt", "First.\n\nSecond.\n")])

    def test_the_store_s_folders_carry_it(self):
        with tempfile.TemporaryDirectory() as dist:
            dist = pathlib.Path(dist)
            built(dist)
            argv = ["catalogue.py", str(dist), "0.4.0", "https://example.invalid/r", "--date", "2026-10-23"]
            with unittest.mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                catalogue.main()
            folder = f"battery-{catalogue.plugin_version('battery')}.zip"
            first = dist.joinpath(folder).read_bytes()
            with zipfile.ZipFile(dist / folder) as z:
                names = z.namelist()
                english = z.read("how/en.txt").decode("utf-8")
            self.assertEqual(names, ["plugin.ini", "how/de.txt", "how/en.txt"])
            self.assertEqual(english, catalogue.ROOT.joinpath("plugins/battery/how/en.txt").read_text(encoding="utf-8"))
            # The same bytes again: the release's checksums hold.
            with unittest.mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                catalogue.main()
            self.assertEqual(dist.joinpath(folder).read_bytes(), first)
            sums = dist.joinpath("SHA256SUMS").read_text(encoding="utf-8")
            self.assertIn(folder, sums)



class ChipFiles(unittest.TestCase):
    """A file is made for a chip: the devices of a chip name the same one."""

    def run_catalogue(self, dist):
        argv = ["catalogue.py", str(dist), "0.6.0", "https://example.invalid/r", "--only", "battery",
                "--date", "2026-11-02"]
        with unittest.mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
            catalogue.main()
        return dist.joinpath("catalogue", "battery.toml").read_text(encoding="utf-8")

    def builds(self, entry):
        import tomllib
        return tomllib.loads(entry)["versions"][0]["builds"]

    def test_the_x4_pro_and_the_x4_classic_name_one_file(self):
        with tempfile.TemporaryDirectory() as dist:
            dist = pathlib.Path(dist)
            own = catalogue.plugin_version("battery")
            riscv, xtensa = f"battery-{own}-esp32c3.ppn", f"battery-{own}-esp32s3.ppn"
            dist.joinpath(riscv).write_bytes(signed(0x85073A61323717FF))
            dist.joinpath(xtensa).write_bytes(signed(0x1111111111111111))
            entry = self.run_catalogue(dist)
            builds = {b["device"]: b for b in self.builds(entry)}
            self.assertEqual(list(builds), ["x4", "x4-pro", "x4-classic"])
            self.assertTrue(builds["x4"]["url"].endswith("/" + riscv))
            self.assertEqual(builds["x4"]["abi"], "85073a61323717ff")
            for device in ("x4-pro", "x4-classic"):
                self.assertTrue(builds[device]["url"].endswith("/" + xtensa), device)
                self.assertEqual(builds[device]["abi"], "1111111111111111")
            self.assertEqual(builds["x4-pro"]["sha256"], builds["x4-classic"]["sha256"])
            self.assertNotEqual(builds["x4"]["sha256"], builds["x4-pro"]["sha256"])
            self.assertIn('devices = ["x4", "x4-pro", "x4-classic"]', entry)
            # Each file once in the release's checksums.
            sums = dist.joinpath("SHA256SUMS").read_text(encoding="utf-8")
            self.assertEqual(sums.count(".ppn"), 2)
            self.assertNotIn("-x4", sums)

    def test_a_chip_without_a_file_has_no_device_of_it(self):
        with tempfile.TemporaryDirectory() as dist:
            dist = pathlib.Path(dist)
            own = catalogue.plugin_version("battery")
            dist.joinpath(f"battery-{own}-esp32s3.ppn").write_bytes(signed(0x1111111111111111))
            entry = self.run_catalogue(dist)
            self.assertEqual([b["device"] for b in self.builds(entry)], ["x4-pro", "x4-classic"])
            self.assertIn('devices = ["x4-pro", "x4-classic"]', entry)

    def test_files_named_by_device_are_no_longer_taken_as_built(self):
        with tempfile.TemporaryDirectory() as dist:
            dist = pathlib.Path(dist)
            own = catalogue.plugin_version("battery")
            dist.joinpath(f"battery-{own}-x4.ppn").write_bytes(signed(0x85073A61323717FF))
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                self.run_catalogue(dist)


class DataPlugins(unittest.TestCase):
    def folders(self, root):
        """The data plugins' folders as data.py builds them, made up."""
        for plugin, fields in catalogue.data.plugins().items():
            folder = root / plugin
            (folder / "fonts").mkdir(parents=True)
            folder.joinpath("plugin.ini").write_text(
                catalogue.data.plugin_ini(plugin, fields) + "formats = 3\n", encoding="utf-8")
            folder.joinpath("fonts", f"{plugin}.ppfp").write_bytes(b"PPFP" + bytes(range(200)))
            for path, text in catalogue.data.catalogs(plugin, fields):
                (folder / path).parent.mkdir(parents=True, exist_ok=True)
                (folder / path).write_text(text, encoding="utf-8")

    def test_a_release_s_data_plugins_come_with_their_folder_alone(self):
        with tempfile.TemporaryDirectory() as dist, tempfile.TemporaryDirectory() as made:
            dist, made = pathlib.Path(dist), pathlib.Path(made)
            built(dist)
            self.folders(made)
            argv = ["catalogue.py", str(dist), "0.5.0-beta.8", "https://example.invalid/r",
                    "--date", "2026-10-25", "--data", str(made)]
            with unittest.mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                catalogue.main()
            entry = dist.joinpath("catalogue", "literata-bold-italic.toml").read_text(encoding="utf-8")
            self.assertIn('kind = "data"', entry)
            self.assertIn("off_by_default = true", entry)
            self.assertIn('name = "Literata Fett Kursiv"', entry)
            self.assertIn('url = "https://example.invalid/r/literata-bold-italic-1.0.0.zip"', entry)
            for word in ["crate", "author_key", "builds"]:
                self.assertNotIn(word, entry)
            with zipfile.ZipFile(dist / "literata-bold-italic-1.0.0.zip") as z:
                self.assertEqual(z.namelist(), [
                    "plugin.ini", "fonts/literata-bold-italic.ppfp", "i18n/de.ini", "i18n/en.ini"])
                self.assertTrue(z.read("fonts/literata-bold-italic.ppfp").startswith(b"PPFP"))
            first = dist.joinpath("literata-bold-italic-1.0.0.zip").read_bytes()
            sums = dist.joinpath("SHA256SUMS").read_text(encoding="utf-8")
            self.assertIn("literata-styles-1.0.0.zip", sums)
            # The same bytes again.
            with unittest.mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                catalogue.main()
            self.assertEqual(dist.joinpath("literata-bold-italic-1.0.0.zip").read_bytes(), first)

    def test_one_data_plugin_alone_between_releases(self):
        with tempfile.TemporaryDirectory() as dist, tempfile.TemporaryDirectory() as made:
            dist, made = pathlib.Path(dist), pathlib.Path(made)
            self.folders(made)
            tag = "literata-bold-italic-v1.0.0"
            argv = ["catalogue.py", str(dist), "0.5.0-beta.8", f"https://example.invalid/{tag}",
                    "--only", "literata-bold-italic", "--tag", tag, "--data", str(made)]
            with unittest.mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                catalogue.main()
            # Its entry alone, no plugin with code built or needed.
            self.assertEqual([p.name for p in dist.joinpath("catalogue").iterdir()], ["literata-bold-italic.toml"])
            entry = dist.joinpath("catalogue", "literata-bold-italic.toml").read_text(encoding="utf-8")
            self.assertIn(f'url = "https://example.invalid/{tag}/literata-bold-italic-1.0.0.zip"', entry)
            sums = dist.joinpath("SHA256SUMS").read_text(encoding="utf-8")
            self.assertEqual(sums.count("\n"), 1)
            self.assertIn("literata-bold-italic-1.0.0.zip", sums)

    def test_data_py_builds_one_alone(self):
        listed = catalogue.data.plugins()
        built = []

        def run(command, **_):
            # A builder as literata-packs.py is: every plugin it has.
            built.append(command[1])
            self.folders(pathlib.Path(command[2]))

        with tempfile.TemporaryDirectory() as out:
            argv = ["data.py", out, "--only", "literata-bold-italic"]
            with unittest.mock.patch.object(sys, "argv", argv), \
                    unittest.mock.patch.object(catalogue.data.subprocess, "run", run), \
                    contextlib.redirect_stdout(io.StringIO()) as said:
                catalogue.data.main()
        self.assertEqual(built, [str(catalogue.data.ROOT / listed["literata-bold-italic"]["build"])])
        self.assertIn("1 data plugins", said.getvalue())
        with unittest.mock.patch.object(sys, "argv", ["data.py", "x", "--only", "nothing"]), \
                self.assertRaises(SystemExit):
            catalogue.data.main()

    def test_a_folder_not_at_its_version_is_refused(self):
        with tempfile.TemporaryDirectory() as dist, tempfile.TemporaryDirectory() as made:
            dist, made = pathlib.Path(dist), pathlib.Path(made)
            built(dist)
            self.folders(made)
            ini = made / "literata-bold-italic" / "plugin.ini"
            ini.write_text(ini.read_text(encoding="utf-8").replace("version = 1.0.0", "version = 0.9.0"))
            argv = ["catalogue.py", str(dist), "0.5.0-beta.8", "https://example.invalid/r", "--data", str(made)]
            with unittest.mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()), \
                    contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                catalogue.main()


class Versions(unittest.TestCase):
    def test_a_release_s_plugins_carry_their_own_versions(self):
        with tempfile.TemporaryDirectory() as dist:
            dist = pathlib.Path(dist)
            built(dist)
            argv = ["catalogue.py", str(dist), "0.5.0-beta.2", "https://example.invalid/r", "--date", "2026-10-24"]
            with unittest.mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                catalogue.main()
            own = catalogue.plugin_version("battery")
            entry = dist.joinpath("catalogue", "battery.toml").read_text(encoding="utf-8")
            # Its crate's version, not PaperPaw's; the SDK and the source
            # the release's.
            self.assertIn(f'version = "{own}"', entry)
            self.assertNotIn('version = "0.5.0-beta.2"', entry)
            self.assertIn('sdk = "0.5"', entry)
            self.assertIn('revision = "v0.5.0-beta.2"', entry)
            with zipfile.ZipFile(dist / f"battery-{own}.zip") as z:
                self.assertIn(f"version = {own}" + chr(10), z.read("plugin.ini").decode("utf-8"))

    def test_one_plugin_alone_between_releases(self):
        with tempfile.TemporaryDirectory() as dist:
            dist = pathlib.Path(dist)
            own = catalogue.plugin_version("vocabulary")
            dist.joinpath(f"vocabulary-{own}-esp32c3.ppn").write_bytes(signed(0x85073A61323717FF))
            argv = ["catalogue.py", str(dist), "0.5.0-beta.2", "https://example.invalid/r",
                    "--only", "vocabulary", "--tag", f"vocabulary-v{own}"]
            with unittest.mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                catalogue.main()
            # Its entry alone, so the store keeps the others' versions.
            self.assertEqual([p.name for p in dist.joinpath("catalogue").iterdir()], ["vocabulary.toml"])
            entry = dist.joinpath("catalogue", "vocabulary.toml").read_text(encoding="utf-8")
            self.assertIn(f'version = "{own}"', entry)
            self.assertIn('sdk = "0.5"', entry)
            self.assertIn(f'revision = "vocabulary-v{own}"', entry)
            sums = dist.joinpath("SHA256SUMS").read_text(encoding="utf-8")
            self.assertNotIn("battery", sums)

    def test_a_plugin_without_a_version_of_its_own_is_refused(self):
        with tempfile.TemporaryDirectory() as root:
            crate = pathlib.Path(root, "demo")
            crate.mkdir()
            crate.joinpath("Cargo.toml").write_text('[package]\nname = "demo"\nversion.workspace = true\n')
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                catalogue.plugin_version("demo", pathlib.Path(root))
            crate.joinpath("Cargo.toml").write_text('[package]\nname = "demo"\nversion = "1.2.0"\n')
            self.assertEqual(catalogue.plugin_version("demo", pathlib.Path(root)), "1.2.0")

    def test_only_a_plugin_of_the_store(self):
        with tempfile.TemporaryDirectory() as dist:
            argv = ["catalogue.py", dist, "0.5.0-beta.3", "https://example.invalid/r", "--only", "nonesuch"]
            with unittest.mock.patch.object(sys, "argv", argv), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    catalogue.main()


if __name__ == "__main__":
    unittest.main()
