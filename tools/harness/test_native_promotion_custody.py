"""Prerequisite and oracle fences for the graphical custody probes."""
import json
import tempfile
import uuid
from pathlib import Path
import unittest
from unittest import mock

from tools.harness.backends import native_promotion_custody as custody


class PromotionCustodyTests(unittest.TestCase):
    def test_pack_wire_records_actual_offer_status_history_and_play_order(self):
        report = {
            "configuration": {
                "clientbound": {"minecraft:resource_pack_push": {"protocol_id": 9}},
                "serverbound": {"minecraft:resource_pack": {"protocol_id": 6}}},
            "play": {"clientbound": {"minecraft:login": {"protocol_id": 50}}}}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "packets.json").write_text(json.dumps(report))
            wire = custody.PackWireReceipt(root / "packets.json", root)
            identity = uuid.uuid4()
            connection = object()
            wire.server_packet(connection, "configuration", 9, identity.bytes + b"\x01u\x00\x00\x00")
            self.assertEqual(wire.receipt()["offer_id"], str(identity))
            self.assertNotIn("statuses", wire.receipt())
            wire.client_packet(connection, "configuration", 6, identity.bytes + b"\x03")
            wire.client_packet(connection, "configuration", 6, identity.bytes + b"\x02")
            wire.server_packet(connection, "play", 50, b"")
            receipt = wire.receipt()
            self.assertEqual(receipt["statuses"], [
                {"id": str(identity), "action": 3, "sequence": 2},
                {"id": str(identity), "action": 2, "sequence": 3}])
            self.assertFalse(receipt["offer_required"])
            self.assertLess(receipt["offer_sequence"], receipt["statuses"][0]["sequence"])
            self.assertLess(receipt["statuses"][1]["sequence"], receipt["play_sequence"])
            self.assertEqual(json.loads(wire.path.read_text()), receipt)
            with self.assertRaisesRegex(ValueError, "trailing bytes"):
                wire.client_packet(connection, "configuration", 6, identity.bytes + b"\x01x")
            with self.assertRaisesRegex(ValueError, "multiple connections"):
                wire.server_packet(object(), "play", 50, b"")

    def steps(self, **client_fields):
        steps = mock.Mock()
        steps.client = mock.Mock(spec=list(client_fields))
        for name, value in client_fields.items():
            setattr(steps.client, name, value)
        steps.report = {}

        def check(name, condition):
            if not condition:
                raise RuntimeError(name)

        steps.check.side_effect = check
        return steps

    def test_both_scenarios_refuse_headless_before_any_actions(self):
        for scenario in (custody.MOUNT_SCENARIO, custody.PACK_SCENARIO):
            with self.subTest(scenario=scenario):
                steps = self.steps()
                with self.assertRaisesRegex(RuntimeError, "windowed Xvfb"):
                    custody.run(steps, (0, 64, 0), scenario)
                steps.call.assert_not_called()

    def test_mount_requires_conversion_hook(self):
        steps = self.steps(p4_window={"window_id": 1})
        with self.assertRaisesRegex(RuntimeError, "offline clean export/reimport"):
            custody.run(steps, (0, 64, 0), custody.MOUNT_SCENARIO)
        steps.call.assert_not_called()

    def test_pack_requires_matching_accepted_failed_download_before_play(self):
        steps = self.steps(p4_window={"window_id": 1})
        with self.assertRaisesRegex(RuntimeError, "wire receipt hook"):
            custody.run(steps, (0, 64, 0), custody.PACK_SCENARIO)
        accepted = {"id": "pack-id", "action": 3, "sequence": 2}
        failed = {"id": "pack-id", "action": 2, "sequence": 3}
        for change in ({}, {"offer_required": True},
                       {"statuses": [accepted, dict(failed, id="other")]},
                       {"statuses": [dict(accepted, action=1), failed]},
                       {"statuses": [accepted]}, {"statuses": [failed, accepted]},
                       {"statuses": [accepted, dict(failed, sequence=None)]},
                       {"play_sequence": 2}):
            receipt = {"offer_required": False, "offer_id": "pack-id",
                       "statuses": [accepted, failed], "offer_sequence": 1,
                       "play_sequence": 4}
            receipt.update(change)
            steps = self.steps(p4_window={"window_id": 1},
                               promotion_pack_receipt=lambda: receipt)
            steps.observe.return_value = {"in_play": True}
            with self.subTest(change=change):
                if change:
                    with self.assertRaisesRegex(RuntimeError, "accepted_then_failed_download"):
                        custody.run(steps, (0, 64, 0), custody.PACK_SCENARIO)
                    steps.observe.assert_not_called()
                else:
                    with mock.patch.object(custody, "_echo"), mock.patch.object(custody, "_capture"):
                        custody.run(steps, (0, 64, 0), custody.PACK_SCENARIO)

    def test_cargo_oracle_rejects_resurrected_or_extra_stacks(self):
        state = {"container": {"slot_count": 53, "carried": {"count": 0},
                               "slots": [{"slot": 2, "item_id": "minecraft:emerald", "count": 5}]}}
        self.assertTrue(custody._cargo_exact(state, "minecraft:emerald", 5))
        state["container"]["slots"].append({"slot": 3, "item_id": "minecraft:diamond", "count": 7})
        self.assertFalse(custody._cargo_exact(state, "minecraft:emerald", 5))
        state["container"]["slots"].pop()
        state["container"]["carried"]["count"] = 1
        self.assertFalse(custody._cargo_exact(state, "minecraft:emerald", 5))


if __name__ == "__main__":
    unittest.main()
