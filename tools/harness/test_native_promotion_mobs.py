"""Fail-closed snow-golem interaction oracles; not graphical receipts."""
from __future__ import annotations

import unittest
from unittest import mock

from .backends import native_promotion_mobs as mobs


class MobPromotionTests(unittest.TestCase):
    def test_received_metadata_fails_closed(self):
        self.assertFalse(mobs._snow_initial_pumpkin({}))
        self.assertTrue(mobs._snow_initial_pumpkin({"entity_type": "minecraft:snow_golem", "received_metadata": {}}))
        self.assertFalse(mobs._snow_initial_pumpkin({"entity_type": "minecraft:snow_golem",
                                                   "received_metadata": {"16": {"kind": "byte", "value": 0}}}))
        self.assertFalse(mobs._snow_flag({}, True))
        self.assertFalse(mobs._snow_flag({}, False))
        self.assertFalse(mobs._snow_flag({"received_metadata": {"16": {"kind": "bool", "value": False}}}, False))
        self.assertTrue(mobs._snow_flag({"received_metadata": {"16": {"kind": "byte", "value": 16}}}, True))
        self.assertTrue(mobs._snow_flag({"received_metadata": {"16": {"kind": "byte", "value": 0}}}, False))
    def test_requires_window_and_known_scenario(self):
        steps = mock.Mock()
        steps.client.p4_window = None
        with self.assertRaisesRegex(RuntimeError, "windowed Xvfb"):
            mobs.run(steps, (0, 0, 0), "snow-golem-shear")
        with self.assertRaises(ValueError):
            mobs.run(steps, (0, 0, 0), "unknown")

    def test_snow_rejects_wrong_start_and_missing_received_transition(self):
        class Steps:
            def __init__(self):
                self.report = {}
                self.calls = []

            def remaining(self):
                return 30

            def observe(self):
                return {"inventory": []}

            def check(self, label, condition):
                if not condition:
                    raise RuntimeError(label)

            def fixture(self, command):
                self.calls.append(command)

            def call(self, name, arguments=None):
                self.calls.append(name)
                return {}

            def wait(self, predicate, label, **kwargs):
                self.check(label, predicate(self.observe()))

        def golem(flag):
            return {"entity_id": 7, "entity_uuid": "golem", "entity_type": "minecraft:snow_golem",
                    "received_metadata": {"16": {"kind": "byte", "value": flag}}}

        steps = Steps()
        with mock.patch.object(mobs, "_rows", return_value=[]), \
             mock.patch.object(mobs, "_echo"), \
             mock.patch.object(mobs, "_entity_at", return_value=golem(0)), \
             mock.patch.object(mobs, "_interact") as interact:
            with self.assertRaisesRegex(RuntimeError, "snow_pumpkin_initial_received_or_vanilla_default"):
                mobs._snow(steps, (0, 0, 0))
            interact.assert_not_called()
        steps = Steps()
        with mock.patch.object(mobs, "_rows", return_value=[]), \
             mock.patch.object(mobs, "_echo"), \
             mock.patch.object(mobs, "_entity_at", return_value=golem(16)), \
             mock.patch.object(mobs, "_same_row", return_value=golem(16)), \
             mock.patch.object(mobs, "_give"), mock.patch.object(mobs, "_aim"), \
             mock.patch.object(mobs, "_capture"), mock.patch.object(mobs, "_interact") as interact:
            with self.assertRaisesRegex(RuntimeError, "snow_same_uuid_received_pumpkin_removed"):
                mobs._snow(steps, (0, 0, 0))
            interact.assert_called_once()
        steps = Steps()
        with mock.patch.object(mobs, "_rows", return_value=[]), \
             mock.patch.object(mobs, "_echo"), \
             mock.patch.object(mobs, "_entity_at", return_value=golem(16)), \
             mock.patch.object(mobs, "_same_row", return_value=golem(0)), \
             mock.patch.object(mobs, "item_count", side_effect=(0, 1)), \
             mock.patch.object(mobs, "_give"), mock.patch.object(mobs, "_aim"), \
             mock.patch.object(mobs, "_capture"), mock.patch.object(mobs, "_interact"):
            mobs._snow(steps, (0, 0, 0))
        self.assertIn("minecraft_press_inputs", steps.calls)
        self.assertTrue(any("Rotation:[180f,0f]" in call for call in steps.calls))



if __name__ == "__main__":
    unittest.main()
