#!/usr/bin/env python3
"""Makes a synthetic library of tiny valid EPUBs for the large-library
benchmark (#79): mixed folders and depths, long and non-Latin names,
series, some covers. Deterministic for a seed; nothing of it is committed.

usage: make-library.py <out-dir> <books> [--seed N] [--covers PERCENT]

The books go under <out-dir>/books/. Each holds a title, an author,
a language, a series and number for about a third, and a cover (a small
PNG) for --covers percent of them.
"""

import argparse
import random
import struct
import zipfile
import zlib
from pathlib import Path

WORDS = (
    "night river garden stone winter silver house letter road shadow light "
    "city sea island mountain forest bridge voice dream fire glass storm "
    "summer heart secret window journey crown wolf orchard harbour lantern "
    "promise empire tide clock mirror song kingdom echo ember map thief"
).split()
FIRST = (
    "Anna Ben Clara David Emma Felix Greta Hugo Ida Jonas Karl Lena Max Nora "
    "Oskar Paula Quinn Rosa Simon Tara Ulla Viktor Wilma Xaver Yara Zoe"
).split()
LAST = (
    "Adler Becker Corwin Dahl Eriksen Fischer Gray Hartmann Ivanov Jansen "
    "Keller Lindqvist Moreau Nakamura Olsen Petrov Quint Rossi Schmidt "
    "Tanaka Ulrich Vogel Weber Young Zimmermann"
).split()
# Names in other scripts and with marks, as found on real cards.
UNICODE = [
    ("Анна Каренина", "Лев Толстой"),
    ("Der Zauberberg – Übersicht", "Thomas Mann"),
    ("Les Misérables", "Victor Hugo"),
    ("百年の孤独", "ガルシア＝マルケス"),
    ("رحلة إلى الشرق", "نجيب محفوظ"),
    ("Ἰλιάς", "Ὅμηρος"),
    ("Smørrebrød og æbleskiver", "Ørsted Ågård"),
    ("Đường về nhà", "Nguyễn Du"),
    ("Ściana płaczu", "Łukasz Żółć"),
    ("한국어 소설", "김영하"),
]
LANGS = ["en", "de", "fr", "es", "it", "nl", "sv", "ru", "ja", "ar"]


def png(width, height, seed):
    """A small grey PNG, rows of a pattern from `seed`."""
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw.extend(((x * 7 + y * 3 + seed) & 0xFF) for x in range(width))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    head = struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", head)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def xml_text(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def epub(path, number, title, author, lang, series, cover):
    """Writes one EPUB 3 with a chapter and, where asked, a cover."""
    manifest = [
        '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>',
        '<item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/>',
    ]
    meta = []
    if series:
        name, number = series
        meta.append(f'<meta name="calibre:series" content="{xml_text(name)}"/>')
        meta.append(f'<meta name="calibre:series_index" content="{number}"/>')
    if cover is not None:
        manifest.append(
            '<item id="cover" href="cover.png" media-type="image/png" properties="cover-image"/>'
        )
        meta.append('<meta name="cover" content="cover"/>')
    opf = f"""<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="id">urn:paperpaw-bench:{number}</dc:identifier>
<dc:title>{xml_text(title)}</dc:title>
<dc:creator>{xml_text(author)}</dc:creator>
<dc:language>{lang}</dc:language>
{"".join(meta)}
</metadata>
<manifest>{"".join(manifest)}</manifest>
<spine><itemref idref="c1"/></spine>
</package>"""
    container = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"""
    nav = """<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<body><nav epub:type="toc"><ol><li><a href="c1.xhtml">1</a></li></ol></nav></body></html>"""
    chapter = f"""<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>{xml_text(title)}</title></head>
<body><h1>{xml_text(title)}</h1><p>A short book to fill the library.</p></body></html>"""
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w") as z:
        z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip")
        deflate = zipfile.ZIP_DEFLATED
        z.writestr("META-INF/container.xml", container, deflate)
        z.writestr("OEBPS/content.opf", opf, deflate)
        z.writestr("OEBPS/nav.xhtml", nav, deflate)
        z.writestr("OEBPS/c1.xhtml", chapter, deflate)
        if cover is not None:
            z.writestr("OEBPS/cover.png", cover, zipfile.ZIP_STORED)


def safe(name):
    """A name FAT takes: no reserved characters, no trailing dot or space."""
    for c in '<>:"/\\|?*':
        name = name.replace(c, "_")
    return name.rstrip(". ") or "_"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out")
    parser.add_argument("books", type=int)
    parser.add_argument("--seed", type=int, default=79)
    parser.add_argument("--covers", type=int, default=10)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    root = Path(args.out) / "books"
    # Authors: some write many books (a long tail, as on real cards).
    authors = [f"{rng.choice(FIRST)} {rng.choice(LAST)}" for _ in range(max(20, args.books // 8))]
    series_of = {}
    used = set()
    covers = 0
    for n in range(args.books):
        author = authors[int(len(authors) * rng.random() ** 3)]
        title = " ".join(rng.choice(WORDS).capitalize() for _ in range(rng.randint(1, 4)))
        if rng.random() < 0.3:
            title = "The " + title
        lang = rng.choice(LANGS[:6])
        if rng.random() < 0.08:
            title, author = rng.choice(UNICODE)
            title = f"{title} {n}"
            lang = rng.choice(LANGS)
        series = None
        if rng.random() < 0.33:
            name = series_of.setdefault(author, f"{rng.choice(WORDS).capitalize()} Saga")
            series = (name, rng.randint(1, 12))
        stem = f"{title} - {author}"
        if rng.random() < 0.05:
            stem = f"{stem} ({' '.join(rng.choice(WORDS) for _ in range(25))})"[:200]
        # Depth 0 to 4: the folder, an author's, their series, a genre above.
        depth = rng.choices([0, 1, 2, 3, 4], weights=[20, 35, 25, 15, 5])[0]
        parts = [f"Genre {rng.randint(1, 12)}", author, series[0] if series else "Single", "Box"]
        folder = root.joinpath(*[safe(p) for p in parts[:depth]]) if depth else root
        name = safe(stem)
        while (folder, name) in used:
            name += "_"
        used.add((folder, name))
        cover = None
        if rng.randrange(100) < args.covers:
            cover = png(60, 90, n)
            covers += 1
        epub(folder / f"{name}.epub", n, title, author, lang, series, cover)
    print(f"{args.books} books, {len(authors)} authors, {covers} covers under {root}")


if __name__ == "__main__":
    main()
