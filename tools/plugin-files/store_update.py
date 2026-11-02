#!/usr/bin/env python3
"""What store-plugin.yml checks before it builds one of PaperPaw's own code
plugins between releases:

    tools/plugin-files/store_update.py <plugin> <base-tag> [--accept <path>,<path>...]

Run in a checkout of the source to publish (`HEAD`), with the base's tag
fetched. It prints the plugin's version (its own, its manifest's) on
standard output and fails, saying why on standard error, when

- the source is not a descendant of the base's tag (a branch made from it);
- the plugin is in the base (an update): its version is not above the
  base's. A plugin that is not in the base (a new one, first published
  between releases) needs no older version to compare with;
- the source differs from the base's anywhere but in the plugin, the
  plugin files' tooling (`tools/plugin-files/`), the CHANGELOG and docs:
  anything else (the kernel, the SDK, the shells, other plugins,
  `crates/`) is firmware, or what a firmware shares with a plugin (the
  tables of shared functions), and the published firmware is the base's,
  not this source's. The paths can be accepted by name (`--accept`, a path
  or a folder), each one a decision of whoever runs the workflow; the
  root `Cargo.toml` and `Cargo.lock` (the workspace's members and
  versions) are only reported.

The plugin's fingerprint is not checked here: `build.sh --fingerprint`
fails a file whose fingerprint is not the base firmware's.
"""

import argparse
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import versions  # noqa: E402

# Where a change cannot reach the firmware of the base.
OWN_FOLDERS = ("tools/plugin-files/", "docs/")
OWN_FILES = ("CHANGELOG.md", "README.md")
# Reported, not refused: the workspace's list of members and versions.
REPORTED_FILES = ("Cargo.toml", "Cargo.lock")


def classify(paths, plugin, accepted=()):
    """The changed `paths` in three lists: (own, reported, refused)."""
    own, reported, refused = [], [], []
    accepted = [a.rstrip("/") for a in accepted if a.strip()]
    for path in paths:
        if path.startswith(f"plugins/{plugin}/") or path.startswith(OWN_FOLDERS) or path in OWN_FILES:
            own.append(path)
        elif path in REPORTED_FILES:
            reported.append(path)
        elif any(path == a or path.startswith(a + "/") for a in accepted):
            reported.append(path)
        else:
            refused.append(path)
    return own, reported, refused


def run(*args):
    return subprocess.run(args, cwd=versions.ROOT, capture_output=True, text=True, encoding="utf-8")


def base_manifest(plugin, base):
    """The plugin's manifest in the base, or None where the base has no such plugin."""
    result = run("git", "show", f"{base}:plugins/{plugin}/Cargo.toml")
    return result.stdout if result.returncode == 0 else None


def check(plugin, base, accepted=()):
    """The plugin's version; a list of problems, and one of notes."""
    problems, notes = [], []
    manifest = (versions.ROOT / "plugins" / plugin / "Cargo.toml")
    if not manifest.is_file():
        return None, [f"plugins/{plugin}/Cargo.toml: there is no such plugin"], notes
    now = versions.version_in(manifest.read_text(encoding="utf-8"),
                              (versions.ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    if run("git", "rev-parse", "--verify", "--quiet", f"{base}^{{commit}}").returncode != 0:
        return now, [f"{base} is not a tag or commit here: fetch the base's tag"], notes
    if run("git", "merge-base", "--is-ancestor", base, "HEAD").returncode != 0:
        problems.append(f"the source is not made from {base}: make a branch from the base's tag with the plugin")
    then_manifest = base_manifest(plugin, base)
    if then_manifest is None:
        notes.append(f"{plugin} is not in {base}: a new plugin, first published at {now}")
    else:
        then = versions.version_in(then_manifest, run("git", "show", f"{base}:Cargo.toml").stdout)
        if versions.semver(now) <= versions.semver(then):
            problems.append(f"{plugin} is at {now}, the base has {then}: raise it in plugins/{plugin}/Cargo.toml")
    changed = run("git", "diff", "--name-only", base, "HEAD").stdout.split()
    own, reported, refused = classify(changed, plugin, accepted)
    if reported:
        notes.append("changed outside the plugin, reported (accepted or only the workspace's lists): "
                     + ", ".join(reported))
    if refused:
        problems.append(f"changed outside the plugin since {base}, so not the firmware's source: "
                        + ", ".join(refused)
                        + " (a change there may differ from the published firmware; name a path in `accept` "
                        "only when it cannot)")
    return now, problems, notes


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("plugin")
    parser.add_argument("base")
    parser.add_argument("--accept", default="", help="paths or folders outside the plugin that may differ")
    args = parser.parse_args(argv)
    version, problems, notes = check(args.plugin, args.base, args.accept.split(","))
    for note in notes:
        print(f"::notice::{note}", file=sys.stderr)
    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    if problems:
        return 1
    print(version)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
