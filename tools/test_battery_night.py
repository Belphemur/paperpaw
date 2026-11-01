"""Offline checks for tools/battery-night.py (a log made up here; no device)."""
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("battery_night", Path(__file__).with_name("battery-night.py"))
night = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(night)

HEADER = "kind,ms,millivolts,percent,charging,fast,partial,full,kilopixels,inputs,panel_ms,pages,unix\n"


def line(kind, ms, mv, pct, charging, unix):
    return f"{kind},{ms},{mv},{pct},{charging},0,0,0,0,0,0,0,{unix}"


def rows(*lines):
    return night.read((HEADER + "\n".join(lines)).splitlines())


class Sleeps(unittest.TestCase):
    def test_a_light_sleep_is_read_from_the_end_and_the_first_sample(self):
        # `start` of a light sleep repeats the old reading: not used.
        found = night.sleeps(
            rows(
                line("sample", 1000, 4000, 90, 0, 1_000_000),
                line("end", 2000, 4000, 90, 0, 1_000_060),
                line("start", 2100, 4000, 90, 0, 1_000_060 + 36_000),
                line("sample", 62_000, 3960, 88, 0, 1_000_060 + 36_060),
            )
        )
        self.assertEqual(len(found), 1)
        s = found[0]
        self.assertFalse(s.cold)
        self.assertEqual(s.seconds, 36_060)
        self.assertEqual(s.loss(), (2, 40))

    def test_a_cold_start_has_no_reading_on_its_start_line(self):
        found = night.sleeps(
            rows(
                line("end", 5000, 4100, 95, 0, 2_000_000),
                "start,7000,,,0,0,0,0,0,0,0,0,",
                line("sample", 60_000, 4090, 94, 0, 2_000_000 + 7200),
            )
        )
        self.assertTrue(found[0].cold)
        self.assertAlmostEqual(found[0].hours(), 2.0, places=2)

    def test_a_flagged_sleep_still_gives_its_loss_and_a_gain_is_negative(self):
        # The Pro's flag says "outer power" and was set during a real loss.
        lost = night.sleeps(
            rows(
                line("end", 5000, 4337, 100, 1, 3_000_000),
                "start,7000,,,0,0,0,0,0,0,0,0,",
                line("sample", 60_000, 3500, 15, 1, 3_000_000 + 34_320),
            )
        )[0]
        self.assertTrue(lost.flagged)
        self.assertEqual(lost.loss(), (85, 837))
        gained = night.sleeps(
            rows(
                line("end", 5000, 4100, 90, 1, 3_000_000),
                "start,7000,,,0,0,0,0,0,0,0,0,",
                line("sample", 60_000, 4300, 99, 1, 3_000_000 + 3600),
            )
        )[0]
        self.assertEqual(gained.loss(), (-9, -200))

    def test_without_a_clock_the_time_is_unknown_and_nothing_is_shown(self):
        found = night.sleeps(
            rows(
                line("end", 5000, 4100, 95, 0, ""),
                line("start", 5100, 4100, 95, 0, ""),
                line("sample", 60_000, 4090, 94, 0, ""),
            )
        )
        self.assertIsNone(found[0].seconds)
        self.assertIn("no sleep", night.show(found, 1.0))

    def test_damaged_lines_and_the_header_are_skipped(self):
        got = night.read([HEADER.strip(), "garbage", "end,x,,,0,0,0,0,0,0,0,0,", line("sample", 1, 4000, 90, 0, 5)])
        self.assertEqual([r.kind for r in got], ["sample"])

    def test_two_ends_in_a_row_are_not_one_sleep(self):
        found = night.sleeps(
            rows(
                line("end", 1000, 4000, 90, 0, 100),
                line("end", 2000, 4000, 90, 0, 200),
                line("start", 3000, 4000, 90, 0, 9000),
                line("sample", 4000, 3990, 89, 0, 9010),
            )
        )
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0].ended_at, 200)


if __name__ == "__main__":
    unittest.main()
