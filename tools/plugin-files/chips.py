#!/usr/bin/env python3
"""Which chip each device has, and so which plugin file it runs (ADR 0024):
a code plugin's file is built for a chip, not for a device. A device loads
a file whose header has its chip and its firmware's fingerprint, nothing
else (`shells/xteink/src/native/mod.rs`); the fingerprint hashes the
interface, the target, the pointer size and the layouts, not the device. So
the X4 and the X3 (ESP32-C3) run one file, and the X4 Pro and the X4 Classic
(ESP32-S3) run one, and a release holds one file for each chip:
`<id>-<version>-<chip>.ppn` with the chip `esp32c3` or `esp32s3`. A device
of a chip already here adds no plugin file to a release.

Releases before this one named the files by device (`<id>-<version>-x4.ppn`,
`-x4-pro.ppn`, `-x4-classic.ppn`; their bytes were the same for devices of
one chip). `legacy_names` gives those names, for taking a version's file as
published (published.py).

    python3 tools/plugin-files/chips.py chip <device-or-chip>
        the chip of a device (a chip is its own)
    python3 tools/plugin-files/chips.py fingerprints <dist> <version>
        checks that the firmwares of one chip name one fingerprint, as the
        release's `paperpaw-<device>-<version>-abi.txt` say, so that one
        file fits all of them
"""

import pathlib
import re
import sys

# Chip: its devices, the X4 first. The X3 has no plugin files of its own (the
# plugin library does not list it yet) and would take the X4's.
CHIPS = {
    "esp32c3": ["x4", "x3"],
    "esp32s3": ["x4-pro", "x4-classic"],
}


def fail(message):
    print(f"chips.py: {message}", file=sys.stderr)
    sys.exit(1)


def chip_of(name):
    """The chip of a device, or `name` itself where it is a chip."""
    if name in CHIPS:
        return name
    for chip, devices in CHIPS.items():
        if name in devices:
            return chip
    return None


def file_name(plugin, version, chip):
    return f"{plugin}-{version}-{chip}.ppn"


def legacy_names(name):
    """The names the same file had when releases named plugin files by
    device, for the name of a file of a chip (`[]` for any other)."""
    match = re.fullmatch(r"(.+)-(esp32c3|esp32s3)\.ppn", name)
    if not match:
        return []
    stem, chip = match.groups()
    return [f"{stem}-{device}.ppn" for device in CHIPS[chip]]


def fingerprints(dist, version, listed=("x4", "x4-pro", "x4-classic")):
    """The problems with the firmwares of one chip not all having the one
    fingerprint (first line of `paperpaw-<device>-<version>-abi.txt`), and
    the warnings: devices of `listed` must agree, the others (the X3) are
    only told."""
    problems, warnings = [], []
    for chip, devices in CHIPS.items():
        seen = {}
        for device in devices:
            path = pathlib.Path(dist) / f"paperpaw-{device}-{version}-abi.txt"
            if path.exists():
                lines = path.read_text(encoding="utf-8").splitlines()
                seen[device] = lines[0].strip() if lines else ""
        if not seen:
            problems.append(f"no firmware of the {chip} (its abi file) in {dist}")
            continue
        for device, fingerprint in seen.items():
            if not re.fullmatch(r"[0-9a-f]{16}", fingerprint):
                problems.append(f"{device}: no fingerprint in its abi file")
        (first, expected), *others = seen.items()
        for device, fingerprint in others:
            if fingerprint != expected:
                text = (
                    f"{device} has the fingerprint {fingerprint}, {first} {expected}: "
                    f"one plugin file for the {chip} does not fit both "
                    "(name a file for each device again: tools/plugin-files/chips.py)"
                )
                (problems if device in listed else warnings).append(text)
    return problems, warnings


def main(argv):
    if len(argv) == 3 and argv[1] == "chip":
        chip = chip_of(argv[2])
        if chip is None:
            fail(f"{argv[2]} is no device or chip: " + ", ".join(sum(CHIPS.values(), list(CHIPS))))
        print(chip)
    elif len(argv) == 4 and argv[1] == "fingerprints":
        problems, warnings = fingerprints(argv[2], argv[3])
        for text in warnings:
            print(f"::warning::{text}")
        if problems:
            fail("\n".join(problems))
        print("devices of one chip have one fingerprint")
    else:
        fail("usage: chips.py chip <device-or-chip> | fingerprints <dist> <version>")


if __name__ == "__main__":
    main(sys.argv)
