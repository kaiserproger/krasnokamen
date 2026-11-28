"""The per-row civilization signals: each goes red on its row's symptom and green on the fix.

Every case feeds the *same* function the live `civilization-seed-qa` gate feeds, with the
observation the row's own report documents (its `source`: a sprint series of the client's
dialog body, storage totals or resident census). The ref that a row registers
is then read through the ledger's own path: `regressions.is_graphical_ref` must accept
`harness:civilization-seed-qa:<signal>` and `regressions._profile_verdict` must reject a
False verdict and accept a True one.
"""

from __future__ import annotations

import unittest

from tools.harness import regressions
from tools.harness.backends import civ_row_signals as signals

DAY = signals.DAY
ANCHOR = [132, 76, -215]


def sprint(ticks: int, *, storage: dict[str, int] | None = None, projects: list[dict] | None = None,
           dialog: list[str] | None = None, entities: list[dict] | None = None,
           revision: int = 15) -> dict:
    return {"ticks": ticks, "revision": revision, "storage": storage or {},
            "projects": projects or [], "dialog": dialog or [],
            "entities": entities or [], "anchor": ANCHOR}


def villagers(*offsets: tuple[float, float, float]) -> list[dict]:
    return [{"entity_type": "minecraft:villager", "x": ANCHOR[0] + dx,
             "y": ANCHOR[1], "z": ANCHOR[2] + dz, "profession": "minecraft:none"}
            for dx, _, dz in offsets]


def house(done: int = 0, total: int = 60, phase: str = "Prepared") -> dict:
    return {"blueprint": "house", "level": 1, "phase": phase, "done": done, "total": total,
            "origin": [140, 76, -215]}


class RowSignalTest(unittest.TestCase):
    """One pre-fix symptom and one fixed observation per row."""

    def assert_red_green(self, name: str, red: list[dict], green: list[dict]) -> None:
        self.assertIn(name, signals.SIGNALS)
        self.assertTrue(signals.ROWS[name].isupper(), name)
        self.assertIs(signals.SIGNALS[name](red), False, f"{name}: the row's symptom must read False")
        self.assertIs(signals.SIGNALS[name](green), True, f"{name}: the fix must read True")

    def test_lumberjack_delivers_logs_within_a_day(self):
        """CIV-SEED24-CROSS-VILLAGE-TREE-SWEEP: 62 logs carried, 0 delivered for a day."""
        red = [sprint(0, storage={"minecraft:oak_log": 32}, projects=[house()]),
               sprint(DAY // 2, storage={"minecraft:oak_log": 32}, projects=[house()]),
               sprint(DAY, storage={"minecraft:oak_log": 32}, projects=[house()])]
        green = [*red[:2], sprint(DAY, storage={"minecraft:oak_log": 44}, projects=[house(done=6)])]
        self.assert_red_green("lumberjack_delivers_logs_within_a_day", red, green)

    def test_first_farm_gets_seeds_from_cleared_ground(self):
        """CIV-SEED24-FARM-SEEDS-NEED-A-FARM: 'Wheat Seeds 0/12 — needs a farm', 0 seeds stored."""
        waiting = ["Active projects: 0/8", "Waiting for materials",
                   "Farm: Wheat Seeds 0/12 — needs a farm"]
        red = [sprint(0, dialog=waiting), sprint(DAY // 2, dialog=waiting), sprint(DAY, dialog=waiting)]
        green = [*red[:2], sprint(DAY, dialog=waiting, storage={"minecraft:wheat_seeds": 3})]
        self.assert_red_green("first_farm_gets_seeds_from_cleared_ground", red, green)

    def test_wool_reaches_a_wool_bill_without_a_shepherd(self):
        """CIVILIZATION-EMPLOYED-SHEARING: a wool bill at 0 with employed adults and no shepherd."""
        waiting = ["House L1 · Waiting for materials", "White Wool 0/4 — awaiting production"]
        red = [sprint(0, dialog=waiting, entities=villagers((1, 0, 0), (2, 0, 1))),
               sprint(DAY // 2, dialog=waiting, entities=villagers((1, 0, 0), (2, 0, 1))),
               sprint(DAY, dialog=waiting, entities=villagers((1, 0, 0), (2, 0, 1)))]
        green = [*red[:2], sprint(DAY, dialog=waiting, storage={"minecraft:white_wool": 4},
                                  entities=villagers((1, 0, 0), (2, 0, 1)))]
        self.assert_red_green("wool_reaches_a_wool_bill_without_a_shepherd", red, green)

    def test_funded_project_advances_within_a_day(self):
        """CIVILIZATION-FUNDED-PLANNER-IDLE: revision 15, cursor 0 through the funded interval."""
        red = [sprint(0, projects=[house()], revision=15),
               sprint(DAY // 2, projects=[house()], revision=15),
               sprint(DAY, projects=[house()], revision=15)]
        green = [*red[:2], sprint(DAY, projects=[house(done=5, phase="Building")], revision=16)]
        self.assert_red_green("funded_project_advances_within_a_day", red, green)

    def test_no_worker_stranded_past_the_simulated_boundary(self):
        """CIVILIZATION-INACTIVE-SHEEP-TARGET: a resident 70 blocks out, no wool, for a day."""
        far = villagers((70, 0, 0))
        red = [sprint(0, entities=far), sprint(DAY // 2, entities=far), sprint(DAY, entities=far)]
        green = [*red[:2], sprint(DAY, storage={"minecraft:white_wool": 2},
                                  entities=villagers((3, 0, 0)))]
        self.assert_red_green("no_worker_stranded_past_the_simulated_boundary", red, green)

    def test_mixed_wood_load_reaches_storage(self):
        """CIVILIZATION-MIXED-LOG-HAUL-STARVATION: only founding oak for a day; 61 birch carried."""
        red = [sprint(0, storage={"minecraft:oak_log": 32}, projects=[house()]),
               sprint(DAY // 2, storage={"minecraft:oak_log": 32}, projects=[house()]),
               sprint(DAY, storage={"minecraft:oak_log": 32}, projects=[house()])]
        green = [*red[:2], sprint(DAY, storage={"minecraft:oak_log": 32, "minecraft:birch_log": 61},
                                  projects=[house(done=2)])]
        self.assert_red_green("mixed_wood_load_reaches_storage", red, green)

    def test_natural_forest_site_is_felled_and_housing_grows(self):
        """CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION: a House order waits; 185 lots, none free."""
        waiting = ["Active projects: 0/8", "Pending orders: 1/8", "Head: House L1",
                   "Pending #1: House L1 · not enough"]
        red = [sprint(0, dialog=waiting, storage={"minecraft:oak_log": 32}),
               sprint(DAY // 2, dialog=waiting, storage={"minecraft:oak_log": 32}),
               sprint(DAY, dialog=waiting, storage={"minecraft:oak_log": 32})]
        green = [*red[:2], sprint(DAY, dialog=waiting, storage={"minecraft:oak_log": 48},
                                  projects=[house(done=4, phase="Building")])]
        self.assert_red_green("natural_forest_site_is_felled_and_housing_grows", red, green)

    def test_street_completion_releases_the_builder(self):
        """CIVILIZATION-STREET-BUILDER-RETENTION: after the road completes nothing else moves."""
        road = {"blueprint": "road", "level": 1, "phase": "Complete", "done": 30, "total": 30,
                "origin": [132, 76, -215]}
        stalled = {"blueprint": "house", "level": 1, "phase": "Prepared", "done": 0, "total": 60,
                   "origin": [140, 76, -215]}
        red = [sprint(0, projects=[road], storage={"minecraft:oak_log": 32}),
               sprint(DAY // 2, projects=[road, stalled], storage={"minecraft:oak_log": 32}),
               sprint(DAY, projects=[road, stalled], storage={"minecraft:oak_log": 32})]
        green = [*red[:2], sprint(DAY, projects=[road, {**stalled, "done": 3, "phase": "Building"}],
                                  storage={"minecraft:oak_log": 32})]
        self.assert_red_green("street_completion_releases_the_builder", red, green)

    def test_unobserved_is_not_evidence(self):
        """A surface the run never recorded answers None, never red."""
        for name in signals.SIGNALS:
            self.assertIsNone(signals.SIGNALS[name]([]), name)


class RegisteredRefTest(unittest.TestCase):
    """The ledger reads a row's `harness:civilization-seed-qa:<signal>` exactly this way."""

    def test_every_signal_is_a_graphical_profile_ref(self):
        for name in signals.SIGNALS:
            ref = f"harness:civilization-seed-qa:{name}"
            self.assertTrue(regressions.is_graphical_ref(ref), ref)

    def test_a_false_verdict_is_red_and_a_true_one_passes(self):
        for name in signals.SIGNALS:
            red = {"details": {"verdicts": {name: False}}}
            with self.assertRaises(regressions.VerdictNotMet) as caught:
                regressions._profile_verdict(red, "civilization-seed-qa", name)
            self.assertIs(caught.exception.value, False, name)
            regressions._profile_verdict({"details": {"verdicts": {name: True}}},
                                         "civilization-seed-qa", name)
            # None (never observed) is "not evidence", never red.
            with self.assertRaises(regressions.VerdictNotMet) as missing:
                regressions._profile_verdict({"details": {"verdicts": {name: None}}},
                                             "civilization-seed-qa", name)
            self.assertIsNone(missing.exception.value, name)

    def test_unknown_signal_is_refused(self):
        with self.assertRaises(ValueError):
            regressions._profile_command("civilization-seed-qa", "not_a_row_signal")


if __name__ == "__main__":
    unittest.main()
