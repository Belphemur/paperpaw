"""Tests of chips.py: which chip each device has, the names plugin files had
before they were made for a chip, and the guard that devices sharing a file
share a fingerprint.

    python3 -m unittest discover -s tools/plugin-files
"""

import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import chips  # noqa: E402

HERE = pathlib.Path(__file__).resolve().parent


class Chips(unittest.TestCase):
    def test_each_device_has_its_chip(self):
        self.assertEqual(chips.chip_of("x4"), "esp32c3")
        self.assertEqual(chips.chip_of("x3"), "esp32c3")
        self.assertEqual(chips.chip_of("x4-pro"), "esp32s3")
        self.assertEqual(chips.chip_of("x4-classic"), "esp32s3")
        self.assertEqual(chips.chip_of("esp32s3"), "esp32s3")
        self.assertIsNone(chips.chip_of("android"))

    def test_a_device_is_in_one_chip_only(self):
        every = sum(chips.CHIPS.values(), [])
        self.assertEqual(len(every), len(set(every)))

    def test_the_names_files_had_by_device_before(self):
        self.assertEqual(chips.legacy_names("hello-1.0.0-esp32c3.ppn"), ["hello-1.0.0-x4.ppn", "hello-1.0.0-x3.ppn"])
        self.assertEqual(chips.legacy_names("hello-1.0.0-esp32s3.ppn"),
                         ["hello-1.0.0-x4-pro.ppn", "hello-1.0.0-x4-classic.ppn"])
        # An id that holds a chip's name, and files that are not a chip's.
        self.assertEqual(chips.legacy_names("esp32s3-tool-2.0.0-esp32c3.ppn")[0], "esp32s3-tool-2.0.0-x4.ppn")
        for other in ["hello-1.0.0-x4.ppn", "hello-1.0.0.zip", "hello-1.0.0-esp32s3.ppn.sig"]:
            self.assertEqual(chips.legacy_names(other), [], other)

    def test_every_store_plugin_has_a_file_name_a_chip_gives(self):
        import catalogue
        for plugin, _ in catalogue.plugins():
            name = chips.file_name(plugin, catalogue.plugin_version(plugin), "esp32s3")
            self.assertEqual(chips.legacy_names(name)[0], name.replace("esp32s3", "x4-pro"))

    def test_the_command_line_names_the_chip(self):
        run = lambda *args: subprocess.run([sys.executable, str(HERE / "chips.py"), *args], capture_output=True, text=True)
        self.assertEqual(run("chip", "x4-classic").stdout.strip(), "esp32s3")
        self.assertNotEqual(run("chip", "boox").returncode, 0)
        # build.sh stops at a name it does not know before it builds anything.
        shell = subprocess.run(["bash", str(HERE / "build.sh"), "boox", "out"], capture_output=True, text=True)
        self.assertEqual(shell.returncode, 2)
        self.assertIn("usage", shell.stderr)


class Fingerprints(unittest.TestCase):
    def abi(self, dist, device, fingerprint):
        dist.joinpath(f"paperpaw-{device}-0.6.0-abi.txt").write_text(f"{fingerprint}\ninterface: 5\n", encoding="utf-8")

    def release(self, dist, **fingerprints):
        defaults = {"x4": "a" * 16, "x3": "a" * 16, "x4-pro": "b" * 16, "x4-classic": "b" * 16}
        for device, fingerprint in {**defaults, **fingerprints}.items():
            self.abi(dist, device, fingerprint)

    def test_devices_of_a_chip_with_one_fingerprint_pass(self):
        with tempfile.TemporaryDirectory() as dist:
            dist = pathlib.Path(dist)
            self.release(dist)
            self.assertEqual(chips.fingerprints(dist, "0.6.0"), ([], []))

    def test_a_classic_that_differs_from_the_pro_fails(self):
        with tempfile.TemporaryDirectory() as dist:
            dist = pathlib.Path(dist)
            self.release(dist, **{"x4-classic": "c" * 16})
            problems, warnings = chips.fingerprints(dist, "0.6.0")
            self.assertEqual(len(problems), 1)
            self.assertIn("x4-classic", problems[0])
            self.assertEqual(warnings, [])

    def test_an_x3_that_differs_from_the_x4_is_only_told(self):
        with tempfile.TemporaryDirectory() as dist:
            dist = pathlib.Path(dist)
            self.release(dist, x3="d" * 16)
            problems, warnings = chips.fingerprints(dist, "0.6.0")
            self.assertEqual(problems, [])
            self.assertEqual(len(warnings), 1)

    def test_a_chip_without_firmware_or_a_fingerprint_fails(self):
        with tempfile.TemporaryDirectory() as dist:
            dist = pathlib.Path(dist)
            self.abi(dist, "x4", "a" * 16)
            problems, _ = chips.fingerprints(dist, "0.6.0")
            self.assertEqual(len(problems), 1)
            self.assertIn("esp32s3", problems[0])
            self.abi(dist, "x4-pro", "not a fingerprint")
            problems, _ = chips.fingerprints(dist, "0.6.0")
            self.assertTrue(any("no fingerprint" in p for p in problems))


if __name__ == "__main__":
    unittest.main()
