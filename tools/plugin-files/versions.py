#!/usr/bin/env python3
"""Whether each of PaperPaw's own plugins the store offers (plugins.txt)
that changed since the last release has a newer version of its own:
devices update a plugin only to a newer version, so a fix under the
same one would never reach them.

    python3 tools/plugin-files/versions.py [<since>] [--release]

<since> is the release's tag compared with, the newest `v<version>` tag
before HEAD where not given (CI runs it with the history and the tags).
A plugin updated alone (a tag `<id>-v<version>`, store-plugin.yml) is
compared with those releases too: a change none of them had, under a
version no newer than the newest of them, fails, and so does a version
older than one published.
A plugin's files are `plugins/<id>/` and its binary here
(`src/bin/<id>.rs`). A change no reader sees (a comment, a test, a
rename inside) keeps the version when each commit with it says so in a
trailer of its message:

    Plugin-unchanged: <id>, <id>      (or `all`)

The release rebuilds such a plugin anyway, for its firmware, at the
version it has.

The plugins built into the firmware (every other crate of `plugins/` that
is published) are held to the version of the release too, but the other
way: crates.io keeps the sources of a published version, so any change in
`plugins/<id>/` since the release (a comment, a test, a rename included;
`Plugin-unchanged:` does not apply) needs a newer version before the
crates are published (docs/releases.md). That is a warning, so that the
bump can be made once, when the beta's changes are in; with `--release`
(the step before the release's version bump) it fails.

Where the plugin interface was raised since the release
(`native::INTERFACE`), every plugin needs a newer version, changed or
not: its code built before does not run on the new firmware, and devices
fetch the new build only as an update.

The data plugins of data.toml are held to the same: one whose builder or
inputs changed since the release needs a newer version in data.toml (or
the trailer in each commit no reader sees). They have no code, so a
raised interface leaves them as they are.

A commit that changed a data plugin's inputs without changing what the
build writes, and that is in shared history (its message cannot take the
trailer any more), is listed in unchanged.txt: its hash and the plugins,
and the proof (the build's output compared with the release's) as a
comment.
"""

import pathlib
import re
import subprocess
import sys
import tomllib

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
VERSION = re.compile(r"\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?")
TRAILER = re.compile(r"^Plugin-unchanged:\s*(.+)$", re.IGNORECASE | re.MULTILINE)
INTERFACE = re.compile(r"pub const INTERFACE: u16 = (\d+);")
NATIVE = "crates/paperpaw-sdk/src/native.rs"
UNCHANGED = "tools/plugin-files/unchanged.txt"


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True,
                          text=True, encoding="utf-8").stdout


def plugins():
    """The ids of plugins.txt."""
    out = []
    for line in (HERE / "plugins.txt").read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].split()
        if line:
            out.append(line[0])
    return out


def semver(version):
    """A key that orders versions as SemVer does (build metadata left out)."""
    version = version.split("+", 1)[0]
    core, _, pre = version.partition("-")
    numbers = tuple(int(n) for n in core.split("."))
    if not pre:
        return numbers, (1,)
    ids = tuple((0, int(i), "") if i.isdigit() else (1, 0, i) for i in pre.split("."))
    return numbers, (0, ids)


def version_in(manifest, workspace):
    """The [package] version of `manifest` (Cargo.toml's text), the
    workspace's (its text) where it takes that one."""
    version = tomllib.loads(manifest)["package"].get("version")
    if isinstance(version, dict):
        return tomllib.loads(workspace)["workspace"]["package"]["version"]
    return version


def unchanged(message, plugin):
    """Whether the commit `message` says it changed nothing a reader of
    `plugin` sees."""
    for names in TRAILER.findall(message):
        ids = {name.strip() for name in names.split(",")}
        if plugin in ids or "all" in ids:
            return True
    return False


def exempt_commits(text):
    """Of unchanged.txt's `text` (lines `<hash> <id>, <id>  # proof`), the
    (hash, plugin) pairs; a hash may be an abbreviation."""
    out = set()
    for line in text.splitlines():
        words = line.split("#", 1)[0].replace(",", " ").split()
        out.update((words[0], plugin) for plugin in words[1:])
    return out


def exempted(commit, plugin, pairs):
    """Whether `pairs` (exempt_commits) say the `commit` (its full hash)
    changed nothing a reader of `plugin` sees."""
    return any(plugin == who and len(h) >= 7 and commit.startswith(h) for h, who in pairs)


def interface_at(text):
    """The plugin interface `native.rs` (its text) names."""
    found = INTERFACE.search(text)
    return int(found.group(1)) if found else None


def interface_raised(since):
    """The plugin interface now, where it was raised since `since`; None
    where it was not (or `native.rs` is not there to say)."""
    try:
        now = interface_at((ROOT / NATIVE).read_text(encoding="utf-8"))
        then = interface_at(git("show", f"{since}:{NATIVE}"))
    except (OSError, subprocess.CalledProcessError):
        return None
    return now if None not in (now, then) and now != then else None


def problem(plugin, before, now, changes, raised=None):
    """What is wrong with `plugin` at version `now`, `before` at the
    release, `changes` the messages of the commits that changed it since;
    `raised` the plugin interface now where it was raised since; None
    where nothing is."""
    if semver(now) < semver(before):
        return f"its version {now} is older than the release's {before}"
    if raised is not None and semver(now) == semver(before):
        return (
            f"the plugin interface was raised to {raised} since the release, its version "
            f"{now} did not: raise it in plugins/{plugin}/Cargo.toml (and the Cargo.lock files "
            f"that list it), so that devices fetch its new build"
        )
    seen = [m for m in changes if not unchanged(m, plugin)]
    if seen and semver(now) == semver(before):
        first = seen[0].splitlines()[0] if seen[0] else ""
        return (
            f"changed since the release ({len(seen)} commits, e.g. \"{first}\"), its version "
            f"{now} did not: raise it in plugins/{plugin}/Cargo.toml (and the Cargo.lock files "
            f"that list it), or say `Plugin-unchanged: {plugin}` in each commit no reader sees"
        )
    return None


def own_tags(plugin, tags):
    """Of `tags`, the plugin's own releases (`<id>-v<version>`)."""
    prefix = f"{plugin}-v"
    return [t for t in tags if t.startswith(prefix) and VERSION.fullmatch(t[len(prefix):])]


def version_at(tag, manifest):
    """The plugin's version at `tag`; None where it was not there yet."""
    try:
        return version_in(git("show", f"{tag}:{manifest}"), git("show", f"{tag}:Cargo.toml"))
    except subprocess.CalledProcessError:
        return None


def is_ancestor(tag):
    """Whether `tag` is in HEAD's history."""
    done = subprocess.run(["git", "merge-base", "--is-ancestor", tag, "HEAD"], cwd=ROOT,
                          capture_output=True)
    return done.returncode == 0


def same_files(tag, paths):
    """Whether `paths` are at `tag` as they are at HEAD."""
    done = subprocess.run(["git", "diff", "--quiet", tag, "HEAD", "--", *paths], cwd=ROOT)
    return done.returncode == 0


def builtin_plugins(listed):
    """The ids of the crates in plugins/ that are built into the firmware:
    published, and not among `listed` (plugins.txt)."""
    out = []
    for manifest in sorted((ROOT / "plugins").glob("*/Cargo.toml")):
        package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package", {})
        if manifest.parent.name not in listed and package.get("publish", True) is not False:
            out.append(manifest.parent.name)
    return out


def builtin_problem(plugin, before, now, changes):
    """What is wrong with the built-in `plugin` at version `now`, `before`
    at the release (published then), `changes` the messages of the commits
    that changed its `plugins/<id>/` since; None where nothing is. A trailer
    cannot excuse a change: the published sources would differ."""
    if semver(now) < semver(before):
        return f"its version {now} is older than the release's {before}"
    if changes and semver(now) == semver(before):
        first = changes[0].splitlines()[0] if changes[0] else ""
        return (
            f"built into the firmware, changed since the release ({len(changes)} commits, "
            f"e.g. \"{first}\"), its version {now} did not: crates.io keeps the sources of the "
            f"published {before}, so raise it in plugins/{plugin}/Cargo.toml (and the Cargo.lock "
            f"files that list it) before the crates are published"
        )
    return None


DATA = "tools/plugin-files/data.toml"


def data_version_at(tag, plugin):
    """Data plugin `plugin`'s version in data.toml at `tag`; None where it
    was not there yet."""
    try:
        listed = tomllib.loads(git("show", f"{tag}:{DATA}"))
    except subprocess.CalledProcessError:
        return None
    return listed.get(plugin, {}).get("version")


def data_problems(since, tags=()):
    """The data plugins of data.toml whose builder or inputs changed since
    `since` or their own release (`<id>-v<version>` of `tags`) without a
    newer version: (id, what is wrong). Commits listed in unchanged.txt
    count as no change."""
    if not (ROOT / DATA).exists():
        return []
    now = tomllib.loads((ROOT / DATA).read_text(encoding="utf-8"))
    listed = ROOT / UNCHANGED
    pairs = exempt_commits(listed.read_text(encoding="utf-8") if listed.exists() else "")
    out = []
    for plugin, fields in now.items():
        versions = {tag: data_version_at(tag, plugin) for tag in [since, *own_tags(plugin, tags)]}
        versions = {tag: v for tag, v in versions.items() if v is not None}
        if not versions:
            continue  # new since the release
        before = max(versions.values(), key=semver)
        paths = [fields["build"], *fields.get("inputs", [])]
        had = [f"^{tag}" for tag in versions if is_ancestor(tag)]
        log = git("log", "--no-merges", "--format=%H %B%x00", "HEAD", *had, "--", *paths)
        changes = []
        for entry in log.split("\0"):
            commit, _, message = entry.strip().partition(" ")
            if message.strip() and not exempted(commit, plugin, pairs):
                changes.append(message.strip())
        found = problem(plugin, before, fields["version"], changes)
        if found:
            out.append((plugin, found.replace(
                f"plugins/{plugin}/Cargo.toml (and the Cargo.lock files that list it)", DATA)))
    return out


def builtin_problems(since, listed, workspace_now):
    """The built-in plugins whose sources changed since `since` without a
    newer version: (id, what is wrong)."""
    out = []
    for plugin in builtin_plugins(listed):
        manifest = f"plugins/{plugin}/Cargo.toml"
        before = version_at(since, manifest)
        if before is None:
            continue  # new since the release
        now = version_in((ROOT / manifest).read_text(encoding="utf-8"), workspace_now)
        if same_files(since, [f"plugins/{plugin}"]):
            continue  # commits that undid each other leave the published sources
        log = git("log", "--no-merges", "--format=%B%x00", "HEAD", f"^{since}", "--",
                  f"plugins/{plugin}") if is_ancestor(since) else ""
        changes = [m.strip() for m in log.split("\0") if m.strip()]
        found = builtin_problem(plugin, before, now, changes)
        if found:
            out.append((plugin, found))
    return out


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    release = "--release" in sys.argv[1:]
    since = args[0] if args else git(
        "describe", "--tags", "--abbrev=0", "--match", "v[0-9]*", "HEAD").strip()
    tags = git("tag", "--list").split()
    workspace_now = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    raised = interface_raised(since)
    failed = False
    for plugin in plugins():
        manifest = f"plugins/{plugin}/Cargo.toml"
        # The release's build and the plugin's own releases: the newest
        # version of them, and the commits none of those in HEAD's
        # history had.
        versions = {tag: version_at(tag, manifest) for tag in [since, *own_tags(plugin, tags)]}
        versions = {tag: v for tag, v in versions.items() if v is not None}
        if not versions:
            continue  # new since the release
        before = max(versions.values(), key=semver)
        now = version_in((ROOT / manifest).read_text(encoding="utf-8"), workspace_now)
        paths = [f"plugins/{plugin}", f"tools/plugin-files/src/bin/{plugin}.rs"]
        # A plugin released alone from a branch beside this one (its tag
        # not in HEAD's history) with the files HEAD has: nothing new.
        if now == before and any(
                v == now and not is_ancestor(tag) and same_files(tag, paths)
                for tag, v in versions.items()):
            continue
        had = [f"^{tag}" for tag in versions if is_ancestor(tag)]
        log = git("log", "--no-merges", "--format=%B%x00", "HEAD", *had, "--", *paths)
        changes = [m.strip() for m in log.split("\0") if m.strip()]
        found = problem(plugin, before, now, changes, raised)
        if found:
            print(f"::error file={manifest}::{plugin}: {found}")
            failed = True
    for plugin, found in data_problems(since, tags):
        print(f"::error file=tools/plugin-files/data.toml::{plugin}: {found}")
        failed = True
    waiting = builtin_problems(since, plugins(), workspace_now)
    for plugin, found in waiting:
        level = "error" if release else "warning"
        print(f"::{level} file=plugins/{plugin}/Cargo.toml::{plugin}: {found}")
        failed = failed or release
    if failed:
        sys.exit(1)
    print(f"every plugin that changed since {since} or its own release has a newer version")
    if waiting:
        print(f"{len(waiting)} built-in plugins still wait for their bump "
              "(a warning; versions.py --release fails on them)")


if __name__ == "__main__":
    main()
