#!/usr/bin/env python3
"""A plugin's version is published once: the files of PaperPaw's own
store plugins a release built, against those its releases published
before under the same names (`<id>-<version>-<chip>.ppn`, the folder
`<id>-<version>.zip`), as GitHub lists them. Releases before the files
were made for a chip named them by device (`<id>-<version>-x4.ppn`,
`-x4-pro.ppn`, `-x4-classic.ppn`; chips.py): a file of a chip is also looked
for under those names, so a version published then keeps its bytes:

    gh api --paginate --slurp repos/<owner>/<repo>/releases > releases.json
    python3 tools/plugin-files/published.py files <dir> releases.json
    python3 tools/plugin-files/published.py folders <dir> releases.json

`files`, before catalogue.py: a file of a version published before, for
the same chip, firmware fingerprint and compiler, is taken as published
(its bytes, so a release never offers other bytes under a version that
has them); one for another fingerprint (the plugin interface changed) is
a new build of that version and stays. `folders`, after catalogue.py: a
folder of a version published before must be the same bytes, else it
fails (versions.py keeps a plugin that changed from its old version).
"""

import json
import pathlib
import sys
import urllib.request

import chips

# Where the header says what a file runs on (paperpaw-native's Header):
# its chip, its fingerprint, its compiler.
RUNS_ON = [(6, 8), (8, 16), (44, 48)]


def fail(message):
    print(f"published.py: {message}", file=sys.stderr)
    sys.exit(1)


def assets(text):
    """Name to (tag, address) of every asset of the releases in GitHub's
    list `text` (pages as `--slurp` or `--paginate` give them), drafts
    left out; a name in several, the first release's."""
    decoder = json.JSONDecoder()
    pages, at = [], 0
    text = text.strip()
    while at < len(text):
        page, at = decoder.raw_decode(text, at)
        pages.append(page)
        while at < len(text) and text[at].isspace():
            at += 1
    releases = []
    for page in pages:
        for item in page:
            releases.extend(item if isinstance(item, list) else [item])
    found = {}
    # Oldest first: GitHub lists the newest first.
    for release in sorted(releases, key=lambda r: r.get("created_at") or ""):
        if release.get("draft"):
            continue
        for asset in release.get("assets", []):
            found.setdefault(asset["name"], (release["tag_name"], asset["browser_download_url"]))
    return found


def runs_on(data):
    return [data[a:b] for a, b in RUNS_ON]


def files(folder, published, fetch):
    """Takes each file of a version published before for the same
    firmware as published; what it did, a line a file."""
    said = []
    for path in sorted(folder.glob("*.ppn")):
        # Under its name, or under those a device's had before.
        names = [n for n in [path.name, *chips.legacy_names(path.name)] if n in published]
        if not names:
            continue
        now = runs_on(path.read_bytes())
        other = None
        for name in names:
            tag, url = published[name]
            before = fetch(url)
            if runs_on(before) == now:
                path.write_bytes(before)
                said.append(f"{path.name}: as published in {tag}" + (f" ({name})" if name != path.name else ""))
                break
            other = tag
        else:
            said.append(f"{path.name}: a new build, for another firmware than {other}'s")
    return said


def folders(folder, published, fetch):
    """The folders of versions published before that differ from it."""
    return [
        f"{path.name} differs from the one published in {published[path.name][0]}: "
        "a plugin that changed needs a newer version"
        for path in sorted(folder.glob("*.zip"))
        if path.name in published and fetch(published[path.name][1]) != path.read_bytes()
    ]


def download(url):
    with urllib.request.urlopen(url, timeout=60) as reply:
        return reply.read()


def main():
    if len(sys.argv) != 4 or sys.argv[1] not in ("files", "folders"):
        fail("usage: published.py files|folders <dir> <releases.json>")
    what, folder = sys.argv[1], pathlib.Path(sys.argv[2])
    published = assets(pathlib.Path(sys.argv[3]).read_text(encoding="utf-8"))
    if what == "files":
        for line in files(folder, published, download):
            print(line)
    else:
        differ = folders(folder, published, download)
        if differ:
            fail("\n".join(differ))


if __name__ == "__main__":
    main()
