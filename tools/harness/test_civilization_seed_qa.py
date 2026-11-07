"""Evidence scanning must not turn generated/natural geometry into stage proof."""
import unittest
from unittest.mock import Mock, patch
from tempfile import TemporaryDirectory
from pathlib import Path

from .backends import civilization_seed_qa as qa


class CivilizationEvidenceTests(unittest.TestCase):
    def test_verdict_requires_all_evidence_and_stall_wins(self):
        stages = {name: True for name in ("new_house", "births", "second_building", "iron_smelted")}
        self.assertTrue(qa.acceptance_verdict(stages, stalled=False)["all"])
        for name in stages:
            with self.subTest(missing=name):
                self.assertFalse(qa.acceptance_verdict({**stages, name: False}, stalled=False)["all"])
        self.assertFalse(qa.acceptance_verdict(stages, stalled=True)["all"])

    def test_population_requires_named_server_count(self):
        lines = ["village_00000001 | owner=VillageAuto anchor=[131, 78, -217] radius=136 members=7 revision=12"]
        self.assertEqual(qa.population(lines, "village_00000001"), 7)
        with self.assertRaisesRegex(RuntimeError, "no member count"):
            qa.population(lines, "other")

    def test_birth_requires_complete_baseline_and_new_baby(self):
        old = {"entity_type": "minecraft:villager", "entity_uuid": "old", "received_baby": True}
        baby = {"entity_type": "minecraft:villager", "entity_uuid": "new", "received_baby": True}
        entities = {"entities": [old, baby]}
        self.assertIs(qa.newborn(entities, {"old"}, True), baby)
        self.assertIsNone(qa.newborn(entities, {"old"}, False))
        self.assertIsNone(qa.newborn({"entities": [old]}, {"old"}, True))
        self.assertIsNone(qa.newborn({"entities": [{**baby, "received_baby": False}]}, {"old"}, True))

    def test_baseline_tick_comes_from_a_fresh_server_console_response(self):
        with TemporaryDirectory() as folder:
            log = Path(folder) / "server.log"
            log.write_text("The time is 10\n")
            server = Mock()
            def send(_, command):
                self.assertEqual(command, "time query gametime")
                with log.open("a") as output:
                    output.write("unrelated\nThe time is 120\n")
            with (patch.object(qa.item_merge, "_console", side_effect=send),
                  patch.object(qa.village.runtime, "wait_for_path_condition",
                               side_effect=lambda path, timeout, callback: callback()[1])):
                self.assertEqual(qa.baseline_server_tick(server, log, 10.0), 120)

    def test_birth_events_exclude_children_born_in_other_settlements(self):
        def member(uuid, birth_tick=None):
            runtime = {"settlement": 1}
            if birth_tick is not None:
                runtime.update(birth_tick=birth_tick, birth_sequence=birth_tick * 65536 + 1,
                               birth_parents=["a", "b"])
            return {"uuid": uuid, "active": True, "payload": {
                "id": "minecraft:villager", "Health": 20,
                "solaris:VillagerRuntime": runtime}}
        snapshot = {"read_only": True, "checkpoint_tick": 200, "globals": {
            "settlement/1": {"payload": {"value": {"id": 1, "name": "Test"}}}},
            "entities": [member("a"), member("b"), member("baby", 150), member("child", 160)]}
        log = ("INFO settlement_audit: settlement_claim settlement=1 tick=100\n"
               "INFO settlement_audit: settlement_birth child=baby settlement=1 household=1-1 tick=150\n"
               "INFO settlement_audit: settlement_birth child=child settlement=1 household=1-1 tick=160\n")
        verify = lambda text, snap=snapshot, baby="baby": qa.verify_birth_snapshot(
            snap, "Test", 2, 4, baby, 120, text)
        self.assertTrue(verify(log)["verified"])
        self.assertFalse(verify(log.replace("child=child settlement=1", "child=child settlement=2"))["verified"])
        self.assertFalse(verify(log.replace("tick=160", "tick=100"))["verified"])
        self.assertFalse(verify(log, baby="unseen")["verified"])
        self.assertFalse(verify(log, {**snapshot, "entities": snapshot["entities"][:-1]})["verified"])
        self.assertFalse(verify(log.replace("settlement_claim", "missing_claim"))["verified"])
        self.assertFalse(qa.verify_birth_snapshot(snapshot, "Test", 2, 4, "baby", 155, log)["verified"])

    def test_new_certificate_requires_matching_observed_project(self):
        building = {"id": 2, "kind": "house", "level": 1, "natural": False}
        project = {"blueprint": "house", "level": 1, "origin": [1, 70, 1]}
        blueprints = {"house": {"kind": "house"}}
        with patch.object(qa.village, "_blueprint_placements", return_value=[{}]):
            self.assertEqual(qa.completed_project(building, {1}, [project], blueprints), project)
            self.assertIsNone(qa.completed_project(building, {2}, [project], blueprints))
            self.assertIsNone(qa.completed_project({**building, "natural": True}, set(), [project], blueprints))
            self.assertIsNone(qa.completed_project(building, set(), [], blueprints))
            self.assertIsNone(qa.completed_project({**building, "kind": "mason"}, set(), [project], blueprints))
            self.assertIsNone(qa.completed_project({**building, "level": 2}, set(), [project], blueprints))
        with patch.object(qa.village, "_blueprint_placements", return_value=[]):
            self.assertIsNone(qa.completed_project(building, set(), [project], blueprints))

    def test_stage_capture_closes_menu_and_aims_at_target(self):
        probe = Mock()
        probe.observe.return_value = {"player": {"x": 0, "y": 64, "z": 0},
                                      "target": {"type": "block", "x": 0, "y": 65, "z": 4}}
        with patch.object(qa.native_client, "capture", return_value={"retained_path": "baby.png"}):
            shot = qa.capture_target(probe, "artifact", "baby", (0, 65.62, 4))
        self.assertEqual(shot["retained_path"], "baby.png")
        self.assertEqual(probe.call.call_args_list[0].args[0], "minecraft_close_screen")
        self.assertEqual(probe.call.call_args_list[2].args,
                         ("minecraft_look", {"yaw_deg": 0, "pitch_deg": 0}))

    def test_occluded_stage_target_never_produces_screenshot(self):
        probe = Mock()
        probe.observe.return_value = {"player": {"x": 0, "y": 64, "z": 0},
                                      "target": {"type": "block", "x": 0, "y": 65, "z": 2}}
        with patch.object(qa.native_client, "capture") as capture:
            with self.assertRaisesRegex(RuntimeError, "occluded or out of reach"):
                qa.capture_target(probe, "artifact", "bed", (0, 65.62, 4))
        capture.assert_not_called()

    def test_baby_capture_checks_identity_after_approach(self):
        probe = Mock()
        baby = {"entity_id": 17, "entity_uuid": "new", "received_baby": True,
                "x": 0, "y": 65.12, "z": 4}
        probe.call.side_effect = lambda name, args=None: {"entities": [baby]} if name == "minecraft_list_entities" else {}
        probe.observe.return_value = {"player": {"x": 0, "y": 64, "z": 0},
                                      "target": {"type": "entity", "entity_uuid": "new"}}
        with patch.object(qa.native_client, "capture", return_value={"retained_path": "baby.png"}):
            result = qa.capture_target(probe, "artifact", "baby", (0, 65.62, 4), entity=baby)
        self.assertEqual(result["observed_target"]["entity_uuid"], "new")
        probe.call.assert_any_call("minecraft_approach_entity", {"entity_id": 17, "timeout_seconds": 120.0})
        probe.observe.return_value["target"]["entity_uuid"] = "other"
        with patch.object(qa.native_client, "capture") as capture:
            with self.assertRaisesRegex(RuntimeError, "occluded or out of reach"):
                qa.capture_target(probe, "artifact", "baby", (0, 65.62, 4), entity=baby)
        capture.assert_not_called()

    def test_simulation_wait_uses_scenario_deadline_not_mcp_request_cap(self):
        with patch("tools.harness.backends.probe_steps.time.monotonic", return_value=100.0):
            probe = qa.ProbeSteps(Mock(), None, 7200.0, "sprint")
        with patch("tools.harness.backends.probe_steps.time.monotonic", return_value=300.0):
            self.assertEqual(probe.remaining(), 120.0)
            self.assertEqual(probe.remaining(limit=None), 7000.0)
        with patch("tools.harness.backends.probe_steps.time.monotonic", return_value=7300.0):
            with self.assertRaisesRegex(TimeoutError, "sprint deadline expired"):
                probe.remaining(limit=None)

    def test_civilization_sprint_wait_can_outlast_one_mcp_request(self):
        player = {"player": {"x": 1.0, "y": 70.0, "z": 1.0}}
        with patch("tools.harness.backends.probe_steps.time.monotonic", return_value=100.0):
            probe = qa.ProbeSteps(Mock(), None, 7200.0, "sprint")
        probe.wait = Mock(return_value=player)
        probe.call = Mock(return_value={})

        def planner(probe, detail, chest, read_container, capture):
            capture()
            return {"buildings_now": []}

        def sprint(server, log, ticks, timeout):
            self.assertEqual(ticks, 300)
            self.assertEqual(timeout, 7200.0)
            raise RuntimeError("completion event awaited")

        with patch.object(qa.time, "monotonic", return_value=100.0), \
             patch.object(qa, "walk_to_village", return_value=player), \
             patch.object(qa.village, "_send", side_effect=[
                 ["Settlements: Test."], ["owner=autonomous anchor=[1, 70, 1]"], ["Claimed Test"],
                 ["Test | owner=VillageAuto members=5"], ["Test | owner=VillageAuto members=5"]]), \
             patch.object(qa.village, "_recent_chat", side_effect=lambda lines, _: lines), \
             patch.object(qa.village, "_founding_storage", return_value=(1, 70, 1)), \
             patch.object(qa.village, "_open_chest", return_value={}), \
             patch.object(qa, "fund", side_effect=lambda p, d, s, c: d.update(certified_buildings_before_stock=[])), \
             patch.object(qa.item_merge, "_console"), \
             patch.object(qa.native_client, "capture", return_value={"path": "frame.png"}), \
             patch.object(qa.village, "_planner_status", side_effect=planner), \
             patch.object(qa.village, "_probe_ecology", return_value={}), \
             patch.object(qa.village, "_gametime", return_value=0), \
             patch.object(qa, "building_blocks", return_value=[]), \
             patch.object(qa, "workstation_blocks", return_value=[]), \
             patch.object(qa, "baseline_server_tick", return_value=120), \
             patch.object(qa.item_merge, "_sprint", side_effect=sprint):
            with self.assertRaisesRegex(RuntimeError, "completion event awaited"):
                qa.scenario(probe, {}, "artifact", Mock(), "server.log", Mock())

    def scan(self, buildings, response):
        probe = Mock()
        probe.call.return_value = response
        with patch.object(qa.village, "_listed_buildings", return_value=buildings):
            evidence = qa.building_blocks(probe, [])
        return probe, evidence

    def test_retains_beds_and_furnaces_with_truncation_flag(self):
        building = {"natural": False, "min": [0, 64, 0], "max": [2, 67, 2]}
        blocks = [{"block_id": name} for name in (
            "minecraft:red_bed", "minecraft:furnace", "minecraft:blast_furnace", "minecraft:stone")]
        probe, evidence = self.scan([building], {"blocks": blocks, "truncated": True})
        self.assertEqual(evidence, [{"building": building, "blocks": blocks[:3], "truncated": True}])
        probe.call.assert_called_once_with("minecraft_scan_blocks", {
            "min_x": 0, "min_y": 64, "min_z": 0, "max_x": 2, "max_y": 67, "max_z": 2})

    def test_natural_zone_does_not_become_building_evidence(self):
        probe, evidence = self.scan([{"natural": True}], {})
        probe.call.assert_not_called()
        self.assertEqual(evidence, [])

    def test_oversize_scan_is_explicitly_unobserved(self):
        building = {"natural": False, "min": [0, 64, 0], "max": [31, 95, 31]}
        probe, evidence = self.scan([building], {})
        probe.call.assert_not_called()
        self.assertEqual(evidence, [{"building": building, "skipped": "scan volume exceeds 4096"}])

    def test_empty_scan_never_proves_beds_or_furnaces(self):
        building = {"natural": False, "min": [0, 64, 0], "max": [2, 67, 2]}
        _, evidence = self.scan([building], {"blocks": [], "truncated": False})
        self.assertEqual(evidence[0]["blocks"], [])
        self.assertFalse(evidence[0]["truncated"])

    def test_waypoint_uses_observed_ground_and_clear_dry_headroom(self):
        def cell(y, empty, dry=True):
            return {"y": y, "collision_empty": empty, "fluid": {"empty": dry}}
        self.assertEqual(qa.standable_height([cell(70, False), cell(71, True),
                                              cell(72, True)], 69), 71)
        self.assertIsNone(qa.standable_height([cell(70, False), cell(71, True),
                                               cell(72, False)], 69))
        self.assertIsNone(qa.standable_height([cell(70, False), cell(71, True, False),
                                               cell(72, True)], 69))

    def test_storage_floor_may_be_below_chest_without_overlapping_building(self):
        floor = [{"x": x, "y": 76, "z": z, "collision_empty": False}
                 for x in range(131, 134) for z in range(-218, -215)]
        self.assertEqual(qa.storage_bounds((132, 78, -217), floor, []),
                         ([131, 76, -218], [133, 79, -216]))
        self.assertIsNone(qa.storage_bounds((132, 78, -217), floor[:-1], []))
        self.assertIsNone(qa.storage_bounds((132, 78, -217), floor,
                                           [{"min": [132, 76, -217], "max": [132, 78, -217]}]))

    def test_iron_capture_opens_furnace_and_requires_visible_output(self):
        probe = Mock()
        empty = {"container": {"container_id": 4,
                               "menu_class": "net.minecraft.world.inventory.FurnaceMenu", "slots": []}}
        full = {"container": {"container_id": 4,
                              "menu_class": "net.minecraft.world.inventory.FurnaceMenu",
                              "slots": [{"slot": 2, "item_id": "minecraft:iron_ingot", "count": 2}]}}

        def wait(predicate, label, **kwargs):
            self.assertFalse(predicate(empty))
            self.assertTrue(predicate(full))
            return full

        probe.wait.side_effect = wait
        with patch.object(qa.native_client, "capture", return_value={"retained_path": "iron.png"}) as capture:
            result = qa.capture_iron_menu(probe, "artifact", (1, 2, 3), 600)
        self.assertEqual(result["container"], full["container"])
        capture.assert_called_once_with(probe.client, "artifact", "civilization-iron-000600")
        self.assertEqual([call.args[0] for call in probe.call.call_args_list],
                         ["minecraft_close_screen", "minecraft_set_hotbar_slot",
                          "minecraft_use_item_on", "minecraft_close_screen"])

    def test_observer_is_safe_only_after_ordinary_claim_and_stock_transfer(self):
        probe = Mock()
        player = {"player": {"x": 1.0, "y": 70.0, "z": 1.0}}
        probe.wait.return_value = player
        probe.remaining.return_value = 30.0
        census = 0

        def call(name, args):
            nonlocal census
            if name == "minecraft_list_entities":
                census += 1
                if census == 2:
                    raise RuntimeError("observation begins")
            return {}

        probe.call.side_effect = call
        detail = {"fixture_commands": []}
        probe.report = detail
        events = []
        with patch.object(qa, "walk_to_village", return_value=player), \
             patch.object(qa.village, "_send", side_effect=[
                 ["Settlements: Test."],
                 ["owner=autonomous anchor=[1, 70, 1]"],
                 ["Claimed Test"], ["Test | owner=VillageAuto members=5"]]), \
             patch.object(qa.village, "_recent_chat", side_effect=lambda lines, _: lines), \
             patch.object(qa.village, "_founding_storage", return_value=(1, 70, 1)), \
             patch.object(qa.village, "_open_chest", return_value={}), \
             patch.object(qa, "fund", side_effect=lambda *args: events.append("stock")), \
             patch.object(qa.item_merge, "_console",
                          side_effect=lambda _, command: events.append(command)), \
             patch.object(qa.native_client, "capture", return_value={}), \
             patch.object(qa, "baseline_server_tick", return_value=120):
            with self.assertRaisesRegex(RuntimeError, "observation begins"):
                qa.scenario(probe, detail, "artifact", Mock(), "server.log", Mock())
        self.assertEqual(events, ["stock", f"gamemode creative {qa.village.USERNAME}"])
        self.assertEqual(detail["fixture_commands"],
                         [f"console: gamemode creative {qa.village.USERNAME}"])


if __name__ == "__main__":
    unittest.main()
