"""SUMMON-ITEMS-HORSE-CARGO: a refused summon fixture is the scenario's named assertion.

The pre-fix server refuses the `Items`/`ChestedHorse` summon fields, so the mount never appears.
The wait for it must fail as `<scenario> assertion failed: summoned_cargo_mount_appears` (which
the ledger reads as that scenario's red), not as an unlabelled tool timeout.
"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from . import ledger
from .backends.native_ledger_summon_horse_cargo import (
    APPLE, CARGO_FIRST, DIAMOND, DONKEY, LAST_CARGO, MENU, MENU_SLOTS, run)
from .backends.probe_steps import ProbeSteps

SCENARIO = "native-v2-ledger-summon-horse-cargo"
ASSERTION = "summoned_cargo_mount_appears"


class RefusedSummonClient:
    """Accepts fixture chat; the summoned mount never becomes visible (pre-fix server)."""

    def call_tool(self, name: str, arguments: dict) -> dict:
        if name == "minecraft_wait_for_visible_entity":
            raise RuntimeError('minecraft_wait_for_visible_entity failed: "no matching entity"')
        return {"ok": True}


class SummonedDonkeyClient:
    """Accepts the whole scene: the donkey appears and its cargo menu is received."""

    def __init__(self) -> None:
        self.donkey = {
            "entity_id": 21, "entity_uuid": "00000000-0000-0000-0000-000000000021",
            "entity_type": DONKEY, "x": 1.5, "y": 80.0, "z": 2.5,
            "bounding_box": {"min_y": 80.0, "max_y": 81.6},
        }

    def call_tool(self, name: str, arguments: dict) -> dict:
        if name == "minecraft_wait_for_visible_entity":
            return dict(self.donkey)
        if name == "minecraft_observe":
            return {
                "in_play": True, "state_version": 4, "recent_chat": [],
                "player": {"x": 0.5, "y": 80.0, "z": 0.5},
                "container": {
                    "container_id": 1, "menu_class": MENU, "slot_count": MENU_SLOTS,
                    "slots": [
                        {"slot": CARGO_FIRST, "item_id": DIAMOND, "count": 3},
                        {"slot": LAST_CARGO, "item_id": APPLE, "count": 1},
                    ],
                },
            }
        return {"ok": True}


def steps_for(client: object) -> ProbeSteps:
    return ProbeSteps(client, Path(tempfile.mkdtemp()), 5.0, SCENARIO)


class SummonHorseCargoControlTests(unittest.TestCase):
    def test_refused_summon_fails_as_the_named_scenario_assertion(self) -> None:
        steps = steps_for(RefusedSummonClient())
        with self.assertRaisesRegex(RuntimeError,
                                    rf"{SCENARIO} assertion failed: {ASSERTION}") as raised:
            run(steps, (0, 80, 0))
        self.assertFalse(steps.report["assertions"][ASSERTION])
        # The receipt form the ledger accepts as this scenario's red (ledger.py:724-748).
        receipt = {"status": "failed", "error": f"RuntimeError: {raised.exception}",
                   "details": {"named_scenario": {"id": SCENARIO}}}
        self.assertEqual(ledger._scenario_assertion_failed(receipt, SCENARIO), ASSERTION)

    def test_accepted_summon_records_the_assertion_and_the_cargo_checks(self) -> None:
        steps = steps_for(SummonedDonkeyClient())
        run(steps, (0, 80, 0))
        self.assertTrue(steps.report["assertions"][ASSERTION])
        for name in ("cargo_menu_has_53_slots",
                     "summoned_items_land_in_the_first_cargo_slot",
                     "summoned_items_land_in_the_last_cargo_column",
                     "cargo_summon_does_not_fill_the_saddle_slot"):
            with self.subTest(assertion=name):
                self.assertTrue(steps.report["assertions"][name])


if __name__ == "__main__":
    unittest.main()
