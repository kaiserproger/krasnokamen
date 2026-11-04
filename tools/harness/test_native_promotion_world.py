"""Unit tests of probe wiring and fences, not real-client evidence."""
from __future__ import annotations

import unittest
from unittest import mock

from .backends import native_promotion_world as world


def row(kind: str, number: int) -> dict:
    return {"entity_id": number, "entity_uuid": f"uuid-{number}", "entity_type": f"minecraft:{kind}"}


class Steps:
    def __init__(self) -> None:
        self.client = mock.Mock(spec=["p4_window", "p4_restart_server"])
        self.client.p4_window = {"display": ":99", "window_id": 1}
        self.report = {}
        self.events = []
        self.state = {"in_play": True, "dimension": "minecraft:the_end",
                      "player": {"entity_uuid": "player", "health": 20},
                      "recent_chat": ["Difficulty set to peaceful"]}
        self.dragon = row("ender_dragon", 9)

    def observe(self):
        return self.state

    def remaining(self):
        return 30

    def fixture(self, command):
        if command == "difficulty":
            self.state["recent_chat"].insert(0, "The difficulty is Peaceful")
        self.events.append(("fixture", command))

    def check(self, name, condition):
        self.events.append(("check", name, bool(condition)))
        if not condition:
            raise RuntimeError(name)

    def wait(self, predicate, label, **kwargs):
        self.check(label, predicate(self.state))
        return self.state

    def call(self, name, arguments=None):
        self.events.append(("call", name, arguments))
        if name == "minecraft_wait_for_visible_entity":
            return self.dragon.copy()
        if name == "minecraft_send_chat":
            self.state["recent_chat"].insert(0, arguments["message"])
            return {}
        if name in ("minecraft_wait_for_entity_removed", "minecraft_wait_ticks"):
            return {}
        raise AssertionError(name)


class PromotionWorldTests(unittest.TestCase):
    def test_both_kinds_reject_missing_graphical_hook_before_actions(self):
        for kind in ("end-restart", "entity-gallery"):
            with self.subTest(kind=kind):
                steps = Steps()
                steps.client.p4_window = None
                with self.assertRaisesRegex(RuntimeError, "windowed Xvfb"):
                    world.run(steps, (0, 64, 0), kind)
                self.assertEqual(steps.events, [])

    def test_end_requires_restart_hook_before_actions(self):
        steps = Steps()
        steps.client.p4_restart_server = None
        with self.assertRaisesRegex(RuntimeError, "p4_restart_server"):
            world.run(steps, (0, 64, 0), "end-restart")
        self.assertEqual(steps.events, [])

    def test_end_fences_dragon_uuid_across_restart(self):
        steps = Steps()
        def restart(probe):
            probe.events.append(("restart",))
            probe.dragon = row("ender_dragon", 10)
        with mock.patch.object(world, "_echo", return_value=steps.state), \
             mock.patch.object(world, "_restart", side_effect=restart), \
             mock.patch.object(world, "_aim"), mock.patch.object(world, "_capture") as capture:
            with self.assertRaisesRegex(RuntimeError, "restart_retains_exact_dragon_uuid"):
                world.run(steps, (0, 64, 0), "end-restart")
            self.assertEqual(capture.call_count, 1)
            self.assertIn(("restart",), steps.events)

    def test_gallery_orders_compaction_before_live_echo_and_fences_survivors(self):
        steps = Steps()
        rows = [row("cow", 1), row("snowball", 2), row("arrow", 3), row("armor_stand", 4)]
        def echo(probe, label):
            probe.events.append(("echo", label))
            return probe.state
        with mock.patch.object(world, "_echo", side_effect=echo), \
             mock.patch.object(world, "_entity_at", side_effect=rows), \
             mock.patch.object(world, "_rows", return_value=[rows[0], rows[2], rows[3]]), \
             mock.patch.object(world, "_aim"), mock.patch.object(world, "_capture") as capture:
            world.run(steps, (0, 64, 0), "entity-gallery")
        removal = next(i for i, event in enumerate(steps.events)
                       if event[:2] == ("call", "minecraft_wait_for_entity_removed"))
        echo_index = steps.events.index(("echo", "gallery_post_compaction_ordered_echo"))
        unfreeze = steps.events.index(("fixture", "tick unfreeze"))
        self.assertLess(unfreeze, removal)
        self.assertLess(removal, echo_index)
        self.assertEqual(steps.events[removal][2]["entity_uuid"], "uuid-2")
        self.assertEqual(capture.call_count, 2)
        self.assertIn(("check", "gallery_armor_stand_same_uuid_after_compaction", True), steps.events)

    def test_gallery_rejects_changed_survivor_and_unfreezes_on_setup_failure(self):
        steps = Steps()
        rows = [row("cow", 1), row("snowball", 2), row("arrow", 3), row("armor_stand", 4)]
        with mock.patch.object(world, "_echo", return_value=steps.state), \
             mock.patch.object(world, "_entity_at", side_effect=rows), \
             mock.patch.object(world, "_rows", return_value=[rows[0], rows[2], row("armor_stand", 5)]), \
             mock.patch.object(world, "_aim"), mock.patch.object(world, "_capture"):
            with self.assertRaisesRegex(RuntimeError, "gallery_armor_stand_same_uuid"):
                world.run(steps, (0, 64, 0), "entity-gallery")
        steps = Steps()
        with mock.patch.object(world, "_echo", return_value=steps.state), \
             mock.patch.object(world, "_entity_at", side_effect=RuntimeError("missing fixture")):
            with self.assertRaisesRegex(RuntimeError, "missing fixture"):
                world.run(steps, (0, 64, 0), "entity-gallery")
        self.assertIn(("fixture", "tick unfreeze"), steps.events)

    def test_end_accepts_same_uuid_with_new_protocol_id(self):
        steps = Steps()
        def restart(probe):
            probe.dragon["entity_id"] = 42
        with mock.patch.object(world, "_echo", return_value=steps.state), \
             mock.patch.object(world, "_restart", side_effect=restart), \
             mock.patch.object(world, "_aim"), mock.patch.object(world, "_capture") as capture:
            world.run(steps, (0, 64, 0), "end-restart")
        self.assertEqual(steps.report["dragon_before_restart"]["entity_uuid"],
                         steps.report["dragon_after_restart"]["entity_uuid"])
        self.assertEqual(steps.report["dragon_after_restart"]["entity_id"], 42)
        self.assertEqual(capture.call_count, 2)

    def test_gallery_rejects_different_player_after_compaction(self):
        steps = Steps()
        rows = [row("cow", 1), row("snowball", 2), row("arrow", 3), row("armor_stand", 4)]
        def echo(probe, label):
            if label == "gallery_post_compaction_ordered_echo":
                return {**probe.state, "player": {"entity_uuid": "other", "health": 20}}
            return probe.state
        with mock.patch.object(world, "_echo", side_effect=echo), \
             mock.patch.object(world, "_entity_at", side_effect=rows), \
             mock.patch.object(world, "_aim"), mock.patch.object(world, "_capture"):
            with self.assertRaisesRegex(RuntimeError, "gallery_same_live_player"):
                world.run(steps, (0, 64, 0), "entity-gallery")


    def test_unknown_kind_rejected(self):
        with self.assertRaisesRegex(ValueError, "unknown world promotion"):
            world.run(Steps(), (0, 64, 0), "other")


if __name__ == "__main__":
    unittest.main()
