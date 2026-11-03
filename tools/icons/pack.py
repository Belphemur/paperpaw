"""Builds an icon pack: a data plugin with the drawings of Tabler icons
PaperPaw's icon font lacks, for plugins that name them.

Usage:
    python tools/icons/pack.py OUT_DIR path/to/tabler-icons.ttf ID NAME...

ID names the plugin (e.g. `icons-weather`), NAME the icons by their
Tabler names (https://tabler.io/icons). The font is Tabler's, as for
subset.py (MIT licence, its text goes along). Needs fontTools.

The plugin is written to OUT_DIR/ID: a pack read a glyph at a time from
the card (`fonts/icons.ppfp`, at the sizes of PaperPaw's fonts) that adds
the icons to `fonts-default.icons`, so they are drawn wherever icons are,
on every device. Copy the folder into the device's plugins folder, or
publish it as a plugin.
"""

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from subset import icon_font, tabler_codes

ROOT = Path(__file__).resolve().parents[2]
FONTGEN = ROOT / "target/release/paperpaw-fontgen"
# The font the pack adds to, and the sizes of the shipped fonts' packs.
EXTENDS = "fonts-default.icons"
SIZES = "20,24,28,32,36,40,48"


def main():
    if len(sys.argv) < 5:
        sys.exit(__doc__)
    out_dir, source, id, names = Path(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4:]
    codes = tabler_codes(source)
    unknown = [name for name in names if name not in codes]
    if unknown:
        sys.exit(f"not in Tabler's icons: {', '.join(unknown)}")
    out = out_dir / id
    shutil.rmtree(out, ignore_errors=True)
    (out / "fonts").mkdir(parents=True)
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "paperpaw-fontgen"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        ttf = Path(temp) / "icons.ttf"
        icon_font(source, [codes[name] for name in names]).save(ttf)
        chars = ",".join(f"U+{codes[name]:X}" for name in names)
        subprocess.run([str(FONTGEN), str(ttf), str(out / "fonts" / "icons.ppfp"),
                        "--chars", chars, "--sizes", SIZES, "--hinting", "none"],
                       check=True)
    (out / "plugin.ini").write_text(
        "[plugin]\n"
        f"id = {id}\n"
        f"name = Icons: {', '.join(names)}\n"
        f"description = Tabler's icons {', '.join(names)}, for plugins that show them.\n"
        "version = 0.1.0\n"
        "\n[extends]\n"
        f"icons.ppfp = {EXTENDS}\n",
        encoding="utf-8",
    )
    shutil.copy(ROOT / "plugins/fonts-default/fonts/TablerIcons-MIT.txt",
                out / "fonts" / "TablerIcons-MIT.txt")
    size = sum(f.stat().st_size for f in out.rglob("*") if f.is_file())
    print(f"{out}: {len(names)} icons, {size / 1024:.1f} KiB")


if __name__ == "__main__":
    main()
