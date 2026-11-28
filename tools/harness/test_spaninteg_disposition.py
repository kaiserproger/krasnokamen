"""The wt/SpainInteg decision must stay a complete, checkable record (owner D142).

The parked branch is a patch series, not a merge candidate; every non-merge
commit on it has to stay assigned to a family row or be explicitly dropped, so
the decision cannot silently decay into "some commits are somewhere".
"""

import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
DISPOSITION = ROOT / "receipts/D142/SPAIN-DISPOSITION.md"
ROWS = ROOT / "docs/bugs/rows"

# The branch's 22 non-merge commits measured against base d9c3a591.
COMMITS = [
    "a2ec37ac2", "0cd64cdbe", "6fb805771", "e1d84f9c5", "ed0697168", "1fd4bc9f6",
    "2e2213ce7", "d8f9990f6", "696379ab6", "d46fe2af6", "33554901a", "18ea1a5c3",
    "720176338", "a0ae2b282", "98eab23e3", "2195981eb", "ece50bfc5", "7e9ad6c24",
    "702bb6e41", "83e97e4e8", "ccc408d56", "d5123e2e1",
]

FAMILY_ROWS = [
    "SPAININTEG-F1-TERRAIN-REACH",
    "SPAININTEG-F2-FOUNDING-OUTPOSTS",
    "SPAININTEG-F3-CONSTRUCTION-SUPPLY",
    "SPAININTEG-F4-BUILDER-SHUN",
    "SPAININTEG-F5-RELEASED-CARRY",
    "SPAININTEG-F6-PERIMETER-ROUTE",
    "SPAININTEG-F7-DIAGNOSTICS",
]


def closure_section() -> str:
    text = DISPOSITION.read_text()
    marker = "## Closure (D142 final"
    assert marker in text, "the disposition has no final closure section"
    return text.split(marker, 1)[1]


class SpainDispositionTests(unittest.TestCase):
    def test_every_commit_is_assigned_in_the_closure_table(self):
        section = closure_section()
        missing = [c for c in COMMITS if c not in section]
        self.assertEqual(missing, [], f"commits missing from the closure table: {missing}")

    def test_every_family_has_a_ledger_row(self):
        section = closure_section()
        for row in FAMILY_ROWS:
            self.assertIn(row, section, f"{row} is not named as its family's row")
            self.assertTrue((ROWS / f"{row}.toml").is_file(), f"{row} has no ledger row file")

    def test_the_decision_still_refuses_a_wholesale_merge(self):
        section = closure_section()
        self.assertIn("patch series", section)
        self.assertRegex(section, r"not merged wholesale")

    def test_each_open_family_states_where_it_stands(self):
        section = closure_section()
        for row in FAMILY_ROWS:
            row_text = (ROWS / f"{row}.toml").read_text()
            status = re.search(r'^status = "(.*)"', row_text, re.MULTILINE).group(1)
            if status == "fixed":
                continue
            line = next((ln for ln in section.splitlines() if row in ln), "")
            self.assertTrue(line.endswith("|"), f"{row} has no state cell in the closure table")


if __name__ == "__main__":
    unittest.main()
