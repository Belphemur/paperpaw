#!/usr/bin/env python3
"""Reads the battery plugin's log (`.paperpaw/plugins/battery/log.csv` on the
card) and says what each time asleep cost: the charge before and after, how
long it lasted, and the loss per hour.

    python tools/battery-night.py log.csv [log.old.csv ...]
    python tools/battery-night.py --min-hours 4 log.csv

The log (see plugins/battery/src/log.rs) has a `start` line when the reader
wakes or starts, a `sample` line about once a minute awake, and an `end` line
when it goes to sleep. Between an `end` and the next `start` the reader slept
(or was switched off); the loss is read from the `end` line and the first
`sample` after waking, because the `start` line of a light sleep repeats the
old reading. A negative loss is a charge. The `charging` flag of the X4 Pro
also says "on outer power" and was set during the overnight loss of 26
October 2026: so it is only noted ("flag"), not used to drop a sleep.

Read-only; it never changes the file. Percentages come from the battery's
voltage through a curve (X4) or a gauge (X4 Pro), so a loss of one or two
percent is within the noise: judge by hours and millivolts too.
"""
import argparse
import csv
import sys
from dataclasses import dataclass


@dataclass
class Row:
    kind: str
    ms: int
    millivolts: int | None
    percent: int | None
    charging: bool
    unix: int | None


def number(text):
    return int(text) if text not in (None, "") else None


def read(lines):
    """The rows of a log; lines that are not rows (the header, damage) are skipped."""
    rows = []
    for fields in csv.reader(lines):
        if len(fields) < 13 or fields[0] not in ("start", "sample", "end"):
            continue
        try:
            rows.append(
                Row(
                    fields[0],
                    int(fields[1]),
                    number(fields[2]),
                    number(fields[3]),
                    fields[4] == "1",
                    number(fields[12]),
                )
            )
        except ValueError:
            continue
    return rows


@dataclass
class Sleep:
    ended_at: int | None  # unix seconds of the `end` line
    seconds: int | None  # until the first sample after waking, if both clocks are known
    percent: tuple[int | None, int | None]
    millivolts: tuple[int | None, int | None]
    cold: bool  # the next start began again from zero (switched off or restarted)
    flagged: bool  # `charging` was 1 at the end or after waking

    def hours(self):
        return None if self.seconds is None else self.seconds / 3600

    def loss(self):
        """(percent, millivolts) lost (negative: gained), None where unknown."""
        (p0, p1), (v0, v1) = self.percent, self.millivolts
        return (
            p0 - p1 if p0 is not None and p1 is not None else None,
            v0 - v1 if v0 is not None and v1 is not None else None,
        )


def sleeps(rows):
    """Every `end` followed by a wake: what was left, what was found."""
    found = []
    for i, end in enumerate(rows):
        if end.kind != "end":
            continue
        rest = rows[i + 1 :]
        start = next((r for r in rest if r.kind == "start"), None)
        if start is None:
            continue
        between = rest[: rest.index(start)]
        if any(r.kind != "start" for r in between):
            # More than one `end` in a row, or samples: not one sleep.
            continue
        sample = next((r for r in rest[rest.index(start) :] if r.kind == "sample"), None)
        if sample is None:
            continue
        seconds = None
        if end.unix is not None and sample.unix is not None and sample.unix >= end.unix:
            seconds = sample.unix - end.unix
        found.append(
            Sleep(
                ended_at=end.unix,
                seconds=seconds,
                percent=(end.percent, sample.percent),
                millivolts=(end.millivolts, sample.millivolts),
                cold=start.millivolts is None,
                flagged=end.charging or sample.charging,
            )
        )
    return found


def show(found, min_hours):
    out = []
    out.append("asleep(h)  how      percent     millivolts   loss/h (% , mV)")
    for s in found:
        h = s.hours()
        if h is None or h < min_hours:
            continue
        loss_p, loss_v = s.loss()
        per = lambda v: "   -" if v is None else f"{v / h:5.2f}"
        out.append(
            f"{h:8.2f}   {'switched off' if s.cold else 'light sleep':12} "
            f"{fmt(s.percent)}   {fmt(s.millivolts)}   {per(loss_p)} {per(loss_v)}"
            + ("   (charging flag set: a gain is a charge)" if s.flagged else "")
        )
    if len(out) == 1:
        out.append(f"(no sleep of {min_hours:g} hours or more with a clock on both sides)")
    return "\n".join(out)


def fmt(pair):
    a, b = pair
    one = lambda v: "?" if v is None else str(v)
    return f"{one(a):>5} -> {one(b):<5}"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("logs", nargs="+", help="log.csv (and log.old.csv before it)")
    parser.add_argument("--min-hours", type=float, default=1.0, help="shortest sleep shown (default 1)")
    args = parser.parse_args(argv)
    lines = []
    for name in args.logs:
        with open(name, encoding="utf-8", newline="") as f:
            lines.extend(f.read().splitlines())
    print(show(sleeps(read(lines)), args.min_hours))
    return 0


if __name__ == "__main__":
    sys.exit(main())
