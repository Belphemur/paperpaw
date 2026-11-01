"""Builds PaperPaw's icon font: the icons it uses, taken from Tabler Icons.

Usage:
    python tools/icons/subset.py path/to/tabler-icons.ttf

The font comes from the npm package `@tabler/icons-webfont` (MIT licence,
see plugins/fonts-default/fonts/TablerIcons-MIT.txt), file
`dist/fonts/tabler-icons.ttf`, version TABLER below. Needs fontTools.

Writes:
- plugins/fonts-default/fonts/PaperPawIcons.ttf: only the icons in ICONS,
  at Tabler's code points (private use area);
- crates/paperpaw-sdk/src/icon_table.rs: the names and code points of all
  of Tabler's icons, sorted by name, for `paperpaw_sdk::icons` (plugins
  name any; icon packs bring the drawings the font lacks, see pack.py).

To add an icon, add its Tabler name to ICONS (https://tabler.io/icons)
and run this again.
"""

import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

TABLER = "3.35.0"

# Icons by their Tabler names; PaperPaw uses the same names.
ICONS = sorted({
    # Places
    "home", "books", "book", "book-2", "bookmark", "bookmarks", "list",
    "list-details", "folder", "folder-open", "file", "file-text", "files",
    "wifi-off", "inbox",
    # Reading
    "search", "typography", "text-size", "letter-case", "adjustments",
    "adjustments-horizontal", "layout", "layout-bottombar", "layout-grid",
    "columns", "highlight", "cursor-text", "clock-play", "arrows-horizontal",
    "pencil",
    "note", "hash", "arrow-bar-to-right", "language", "a-b", "link",
    "clipboard", "copy", "selector", "volume", "tag",
    # Light and display
    "sun", "sun-low", "moon", "temperature", "brightness", "contrast",
    "refresh",
    # Tools
    "chart-bar", "calendar", "clock", "wifi", "bluetooth", "world", "cloud",
    "cloud-download", "download", "upload", "rss", "qrcode", "keyboard",
    "plug", "puzzle", "vocabulary", "transfer", "apps", "camera",
    "book-download", "article", "devices", "device-desktop", "stack-2",
    "automation", "circle-arrow-up", "list-check",
    # System
    "settings", "menu-2", "dots", "dots-vertical", "info-circle", "power",
    "battery", "device-tablet", "user", "lock", "palette", "bug",
    "hourglass", "hand-click", "hand-finger", "wand", "key", "eraser",
    "sparkles", "bulb", "paw", "leaf",
    # Controls
    "chevron-left", "chevron-right", "chevron-up", "chevron-down",
    "arrow-left", "arrow-right", "arrow-up", "arrow-down", "x", "check", "plus",
    "minus", "trash",
    "edit", "star", "eye", "sort-ascending", "filter", "circle",
    "circle-check", "toggle-left", "toggle-right",
})


def tabler_codes(path):
    """Every icon of Tabler's font at `path`: its name and code point."""
    return {name: code for code, name in TTFont(path).getBestCmap().items()}


def icon_font(path, codes):
    """Tabler's font at `path` with only the icons at `codes`, its metrics
    set from the drawings."""
    options = subset.Options()
    options.layout_features = []
    options.name_IDs = ["*"]
    options.glyph_names = True
    options.notdef_outline = True
    subsetter = subset.Subsetter(options)
    font = TTFont(path)
    subsetter.populate(unicodes=codes)
    subsetter.subset(font)
    # Tabler's metrics put every left side bearing at 0 although the
    # drawings start further right; renderers that follow the metrics
    # (as PaperPaw's does) would draw icons off centre. The bearings are
    # set from the drawings, and the ascent and descent to the drawings'
    # extent, so an icon centred by its line box is centred by its ink.
    glyf, hmtx = font["glyf"], font["hmtx"]
    top, bottom = 0, 0
    for name in font.getGlyphOrder():
        glyph = glyf[name]
        if glyph.numberOfContours == 0:
            continue
        glyph.recalcBounds(glyf)
        advance, _ = hmtx[name]
        hmtx[name] = (advance, glyph.xMin)
        top, bottom = max(top, glyph.yMax), min(bottom, glyph.yMin)
    font["hhea"].ascent, font["hhea"].descent = top, bottom
    font["OS/2"].sTypoAscender, font["OS/2"].sTypoDescender = top, bottom
    font["OS/2"].usWinAscent, font["OS/2"].usWinDescent = top, -bottom
    return font


# Where icon_table.rs puts a code point: its distance from CODE_BASE, 17
# bits (Tabler's run past U+FFFF), the lowest 16 in CODES.
CODE_BASE = 0xEA00
# A name's length in LENGTHS, the bit above it bit 16 of its code.
LENGTH_BITS = 6


def icon_table(names):
    """The text of icon_table.rs for `names` (name: code point).

    Without pointers (a table of string slices went partly into RAM on
    the X4): one text of all names, sorted, a name that begins the next
    one kept only in it (`a-b` in `a-b-2`), and three arrays of where
    each starts, how long it is and its code point, 5 bytes a name.
    """
    ordered = sorted(names.items())
    if not all(name.isascii() and name for name, _ in ordered):
        sys.exit("an icon name is empty or not ASCII")
    text, starts = "", []
    for i, (name, _) in enumerate(ordered):
        following = ordered[i + 1][0] if i + 1 < len(ordered) else ""
        # A name that begins the next starts where that one, written
        # next, does.
        starts.append(len(text))
        if not following.startswith(name):
            text += name
    if len(text) > 0xFFFF:
        sys.exit(f"the names take {len(text)} bytes: more than CODES' u16 starts reach")
    lengths, codes = [], []
    for name, code in ordered:
        distance = code - CODE_BASE
        if len(name) >= 1 << LENGTH_BITS or not 0 <= distance < 1 << 17:
            sys.exit(f"icon {name} (U+{code:04X}) does not fit icon_table.rs")
        lengths.append(len(name) | (distance >> 16) << LENGTH_BITS)
        codes.append(distance & 0xFFFF)
    lines = [
        "// Generated by tools/icons/subset.py from Tabler Icons "
        f"{TABLER}; do not edit.",
        "",
        "/// The names of all of Tabler's icons, sorted, one after another; a",
        "/// name that begins the next one is kept only in it. The icon font",
        "/// holds those PaperPaw uses, packs add others.",
        "pub(crate) static NAMES: &str = concat!(",
    ]
    lines += [f'    "{text[i:i + 80]}",' for i in range(0, len(text), 80)]
    lines += [");", ""]
    lines += [
        "/// Where icon `i`'s code point is counted from (see [`CODES`]).",
        f"pub(crate) const CODE_BASE: u32 = 0x{CODE_BASE:X};",
        "",
        "/// The bits of a name's length in [`LENGTHS`].",
        f"pub(crate) const LENGTH_BITS: u32 = {LENGTH_BITS};",
        "",
    ]
    lines += array("Where icon `i`'s name starts in [`NAMES`].", "STARTS", "u16", starts)
    lines += array(
        "How long icon `i`'s name is (the lowest [`LENGTH_BITS`] bits) and,\n"
        "/// above them, bit 16 of its code point's distance from [`CODE_BASE`].",
        "LENGTHS",
        "u8",
        lengths,
    )
    lines += array(
        "The lowest 16 bits of icon `i`'s code point's distance from\n/// [`CODE_BASE`].",
        "CODES",
        "u16",
        codes,
    )
    return "\n".join(lines[:-1]) + "\n"


def array(doc, name, kind, values):
    """A static array, its values as many to a line as rustfmt puts."""
    out = [f"/// {doc}", f"pub(crate) static {name}: [{kind}; {len(values)}] = ["]
    line = "   "
    for value in values:
        item = f" {value},"
        if len(line) + len(item) > 100:
            out.append(line)
            line = "   "
        line += item
    out += [line, "];", ""]
    return out


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    root = Path(__file__).resolve().parents[2]
    names = tabler_codes(sys.argv[1])
    missing = [name for name in ICONS if name not in names]
    if missing:
        sys.exit(f"not in Tabler Icons {TABLER}: {', '.join(missing)}")
    codes = {name: names[name] for name in ICONS}

    font = icon_font(sys.argv[1], codes.values())
    # Named for what it is now.
    for record in font["name"].names:
        if record.nameID in (1, 4, 6):
            record.string = "PaperPaw Icons" if record.nameID != 6 else "PaperPawIcons"
    out = root / "plugins/fonts-default/fonts/PaperPawIcons.ttf"
    font.save(out)

    # Every icon's name, so that plugins name any of them; the drawings of
    # those the font lacks come from icon packs (tools/icons/pack.py).
    table = root / "crates/paperpaw-sdk/src/icon_table.rs"
    table.write_text(icon_table(names), encoding="utf-8", newline="\n")
    print(f"{len(codes)} icons in the font, {len(names)} names, {out.stat().st_size} bytes")


if __name__ == "__main__":
    main()
