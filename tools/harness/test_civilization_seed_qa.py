"""Evidence scanning must not turn generated/natural geometry into stage proof."""
import copy
import math
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, call, patch

from .backends import civilization_seed_qa as qa
from .backends import village_autonomy_qa as village


class CivilizationEvidenceTests(unittest.TestCase):
    def test_birth_events_exclude_children_born_in_other_settlements(self):
        def member(uuid, birth_tick=None):
            runtime = {"settlement": 1, "household": "1-1"}
            if birth_tick is not None:
                runtime.update(birth_tick=birth_tick, birth_sequence=birth_tick * 65536 + 1,
                               birth_parents=["a", "b"])
            return {"uuid": uuid, "active": True, "payload": {
                "id": "minecraft:villager", "Health": 20,
                "krasnokamen:VillagerRuntime": runtime}}
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
        for field, foreign in (("settlement", 2), ("household", "1-2")):
            with self.subTest(foreign_binding=field):
                rebound = copy.deepcopy(snapshot)
                rebound["entities"][-1]["payload"]["krasnokamen:VillagerRuntime"][field] = foreign
                self.assertFalse(verify(log, rebound)["verified"])

    def test_house_capture_opens_both_halves_before_exact_bed_and_returns_outside(self):
        for already_open in (False, True):
            with self.subTest(already_open=already_open):
                probe = Mock()
                door = {"block_id": "minecraft:oak_door", "x": 4, "y": 65, "z": 6,
                        "properties": {"half": "lower", "facing": "north",
                                       "open": "true" if already_open else "false"}}
                probe.observe.return_value = {"player": {"x": 1.5, "y": 64.0, "z": 2.5}}
                probe.call.return_value = door
                target = (5.5, 65.5, 7.5)
                def capture(*args):
                    self.assertEqual(args, (probe, "artifact", "house", target))
                    waits = [call.args[1] for call in probe.call.call_args_list
                             if call.args[0] == "minecraft_wait_for_block_state"]
                    self.assertEqual(waits, [
                        {"x": 4, "y": 65 + offset, "z": 6, "block_id": "minecraft:oak_door",
                         "properties": {"half": half, "open": "true"}, "timeout_seconds": 10.0}
                        for offset, half in ((0, "lower"), (1, "upper"))])
                    return {"retained_path": "house.png"}
                with patch.object(qa, "capture_target", side_effect=capture):
                    shot = qa.capture_house(probe, "artifact", "house", target, [door])
                self.assertEqual(shot["retained_path"], "house.png")
                uses = [call.args for call in probe.call.call_args_list
                        if call.args[0] == "minecraft_use_item_on"]
                self.assertEqual(uses, [] if already_open else [
                    ("minecraft_use_item_on", {"x": 4, "y": 65, "z": 6,
                                              "face": "south", "hand": "main_hand"})])
                self.assertEqual(probe.call.call_args_list[-1].args, (
                    "minecraft_navigate_to_block", {"x": 1, "y": 64, "z": 2, "timeout_seconds": 120.0}))

    def test_house_capture_refuses_missing_or_changed_entrance_before_capture(self):
        probe = Mock()
        probe.observe.return_value = {"player": {"x": 1.5, "y": 64.0, "z": 2.5}}
        door = {"block_id": "minecraft:oak_door", "x": 4, "y": 65, "z": 6,
                "properties": {"half": "lower"}}
        probe.call.return_value = {"block_id": "minecraft:air"}
        with patch.object(qa, "capture_target") as capture:
            with self.assertRaisesRegex(RuntimeError, "no received entrance door"):
                qa.capture_house(probe, "artifact", "house", (5.5, 65.5, 7.5), [])
            with self.assertRaisesRegex(RuntimeError, "entrance changed"):
                qa.capture_house(probe, "artifact", "house", (5.5, 65.5, 7.5), [door])
        capture.assert_not_called()

    def test_building_capture_uses_supported_exterior_not_sealed_volume_midpoint(self):
        building = {"id": 4294967302, "kind": "builder",
                    "min": [148, 75, -262], "max": [152, 78, -256]}
        # Received column from the failed seed-24 Builder capture: y77 has
        # air below it; y76 has the cobblestone floor and two clear cells.
        column = [{"x": 150, "y": y, "z": -259, "block_id": block,
                   "collision_empty": empty, "fluid": {"empty": True}}
                  for y, block, empty in (
                      (75, "minecraft:cobblestone", False),
                      (76, "minecraft:air", True),
                      (77, "minecraft:air", True),
                      (78, "minecraft:birch_slab", False))]
        # An exterior supported column is an observer-fixture input, not a
        # claim that the recorded interior is reachable through its closed door.
        exterior = [{**block, "z": -255} for block in column]
        blocks = column + exterior
        class NavigationRequested(Exception):
            pass
        probe = Mock()
        probe.observe.return_value = {"player": {"x": 131.9, "y": 76.9375, "z": -215.5}}
        def call(name, arguments):
            if name == "minecraft_scan_blocks":
                return {"blocks": blocks, "truncated": False}
            if name == "minecraft_navigate_to_block":
                raise NavigationRequested(arguments)
            return {}
        probe.call.side_effect = call
        with self.assertRaises(NavigationRequested) as navigation:
            qa.capture_target(probe, "artifact", "builder", (150.5, 77.0, -258.5), building=building)
        self.assertEqual(navigation.exception.args[0],
                         {"x": 150, "y": 76, "z": -255, "timeout_seconds": 120.0})

    def test_building_capture_refuses_a_column_without_supported_standing_space(self):
        probe = Mock()
        probe.observe.return_value = {"player": {"x": 131.9, "y": 76.9375, "z": -215.5}}
        probe.call.return_value = {"blocks": [
            {"x": 150, "y": y, "z": -255, "collision_empty": True, "fluid": {"empty": True}}
            for y in range(74, 81)], "truncated": False}
        with self.assertRaisesRegex(RuntimeError, "supported standing"):
            qa.capture_target(probe, "artifact", "builder", (150.5, 77.0, -258.5),
                              building={"min": [148, 75, -262], "max": [152, 78, -256]})
        self.assertNotIn("minecraft_navigate_to_block",
                         [call.args[0] for call in probe.call.call_args_list])

    def test_baby_aim_retries_while_a_wall_stands_between_observer_and_baby(self):
        probe = Mock()
        baby = {"entity_id": 17, "entity_uuid": "new", "entity_type": "minecraft:villager",
                "received_baby": True, "x": 0, "y": 65.12, "z": 4}
        probe.call.side_effect = lambda name, args=None: {
            "entities": [baby], "visible_count": 1, "truncated": False, "next_cursor": None
        } if name == "minecraft_list_entities" else {}
        wall = {"type": "block", "x": 1, "y": 65, "z": 2}
        hit = {"type": "entity", "entity_uuid": "new"}
        player = {"x": 0, "y": 64, "z": 0}
        probe.observe.side_effect = [
            {"player": player},                              # initial position
            {"player": player}, {"target": wall},            # attempt 1: occluded
            {"player": player}, {"target": wall},            # attempt 2: occluded
            {"player": player}, {"target": hit},             # attempt 3: seen
        ]
        with patch.object(qa.native_client, "capture", return_value={"retained_path": "baby.png"}):
            result = qa.capture_target(probe, "artifact", "baby", (0, 65.62, 4), entity=baby)
        self.assertEqual(result["observed_target"], hit)
        approaches = [c for c in probe.call.call_args_list if c.args[0] == "minecraft_approach_entity"]
        self.assertEqual(len(approaches), 3)

    def test_baby_beyond_the_walking_observer_gets_the_observer_teleported_and_returned(self):
        baby = {"entity_id": 17, "entity_uuid": "new", "entity_type": "minecraft:villager",
                "received_baby": True, "x": 160.5, "y": 75.0, "z": -280.5}
        replies = iter([{"in_reach": False}, {"in_reach": True}])

        def call(name, args=None):
            if name == "minecraft_list_entities":
                return {"entities": [baby], "visible_count": 1, "truncated": False, "next_cursor": None}
            if name == "minecraft_approach_entity":
                return next(replies)
            return {}

        probe = Mock()
        probe.call.side_effect = call
        player = {"x": 131.0, "y": 77.0, "z": -215.0}
        hit = {"type": "entity", "entity_uuid": "new"}
        probe.observe.side_effect = [{"player": player}, {"player": player}, {"target": hit}]
        teleport = Mock()
        with patch.object(qa.native_client, "capture", return_value={"retained_path": "baby.png"}):
            result = qa.capture_target(probe, "artifact", "baby", (160.5, 75.5, -280.5),
                                       entity=baby, teleport=teleport)
        self.assertTrue(result["observer_teleported"])
        # The capture stands beside the baby (first BABY_TELEPORT_SIDES entry,
        # +1.5 x): from the baby's own block the client sees its own body and the
        # report showed the wall. The return teleport goes back to the walker.
        self.assertEqual([c.args for c in teleport.call_args_list],
                         [(162.0, 75.0, -280.5), (131.0, 77.0, -215.0)])

    def test_sparse_building_whose_centre_aim_misses_is_aimed_at_the_nearest_solid_block(self):
        # Seed-24 Lumber camp (runs 0yj9abeh, p1pmn34d): the centre-nearest solid
        # block gave {'type': 'none'} from the exterior stand; a solid block
        # nearer the observer is in reach and inside the certificate.
        building = {"id": 4294967303, "kind": "lumber_camp",
                    "min": [150, 75, -262], "max": [156, 79, -256]}
        floor = [{"x": 150, "y": y, "z": -255, "block_id": block,
                  "collision_empty": empty, "fluid": {"empty": True}}
                 for y, block, empty in ((75, "minecraft:cobblestone", False),
                                         (76, "minecraft:air", True),
                                         (77, "minecraft:air", True),
                                         (78, "minecraft:birch_slab", False))]
        solid = [{"x": 153, "y": 77, "z": -259}, {"x": 150, "y": 76, "z": -256}]
        looks = []
        probe = Mock()
        def observe():
            seen = {"type": "none"}
            if len(looks) >= 2:
                seen = {"type": "block", "x": 150, "y": 76, "z": -256}
            return {"player": {"x": 150.5, "y": 76.0, "z": -254.5}, "target": seen}
        probe.observe.side_effect = observe
        def call(name, arguments):
            if name == "minecraft_scan_blocks":
                return {"blocks": floor, "truncated": False}
            if name == "minecraft_look":
                looks.append(arguments)
            return {}
        probe.call.side_effect = call
        with patch.object(qa.native_client, "capture", return_value={}):
            result = qa.capture_target(probe, "artifact", "building", (153.5, 77.5, -258.5),
                                       building=building, solid=solid)
        self.assertEqual(len(looks), 2, "the missed centre aim is followed by the nearest solid block")
        self.assertEqual(result["observed_target"], {"type": "block", "x": 150, "y": 76, "z": -256})

    def test_sparse_building_aim_waits_while_a_resident_stands_in_the_line_of_sight(self):
        # Seed-24 run chswt0i9: a villager walked through the aim at the Lumber
        # camp and the capture aborted on the occluder; it passes, so aim again.
        building = {"min": [150, 75, -262], "max": [156, 79, -256]}
        floor = [{"x": 150, "y": y, "z": -255, "block_id": block,
                  "collision_empty": empty, "fluid": {"empty": True}}
                 for y, block, empty in ((75, "minecraft:cobblestone", False),
                                         (76, "minecraft:air", True), (77, "minecraft:air", True),
                                         (78, "minecraft:birch_slab", False))]
        solid = [{"x": 153, "y": 77, "z": -259}]
        waits = []
        probe = Mock()
        def observe():
            seen = {"type": "entity", "entity_uuid": "walker"}
            if len(waits) >= 2:
                seen = {"type": "block", "x": 153, "y": 77, "z": -259}
            return {"player": {"x": 150.5, "y": 76.0, "z": -254.5}, "target": seen}
        probe.observe.side_effect = observe
        def call(name, arguments):
            if name == "minecraft_scan_blocks":
                return {"blocks": floor, "truncated": False}
            if name == "minecraft_wait_ticks":
                waits.append(arguments)
            return {}
        probe.call.side_effect = call
        with patch.object(qa.native_client, "capture", return_value={}):
            result = qa.capture_target(probe, "artifact", "building", (153.5, 77.5, -258.5),
                                       building=building, solid=solid)
        self.assertEqual(len(waits), 2)
        self.assertEqual(result["observed_target"]["type"], "block")

    def test_building_the_walking_observer_cannot_reach_is_observed_from_a_teleport(self):
        # Seed-24 run m8v6480s: navigate_to_block answered "target is unreachable
        # from the current loaded client position" for the Lumber camp stand.
        building = {"min": [150, 75, -262], "max": [156, 79, -256]}
        floor = [{"x": 150, "y": y, "z": -255, "block_id": block,
                  "collision_empty": empty, "fluid": {"empty": True}}
                 for y, block, empty in ((75, "minecraft:cobblestone", False),
                                         (76, "minecraft:air", True), (77, "minecraft:air", True),
                                         (78, "minecraft:birch_slab", False))]
        player = {"x": 131.0, "y": 77.0, "z": -215.0}
        probe = Mock()
        probe.observe.side_effect = lambda: {
            "player": player, "target": {"type": "block", "x": 153, "y": 77, "z": -259}}
        def call(name, arguments):
            if name == "minecraft_scan_blocks":
                return {"blocks": floor, "truncated": False}
            if name == "minecraft_navigate_to_block":
                raise RuntimeError("minecraft_navigate_to_block failed: target is unreachable")
            return {}
        probe.call.side_effect = call
        teleport = Mock()
        with patch.object(qa.native_client, "capture", return_value={}):
            result = qa.capture_target(probe, "artifact", "building", (153.5, 77.5, -258.5),
                                       building=building, solid=[{"x": 153, "y": 77, "z": -259}],
                                       teleport=teleport)
        self.assertTrue(result["observer_teleported"])
        self.assertEqual(teleport.call_args_list[-1].args, (131.0, 77.0, -215.0))

    def test_sparse_building_with_no_solid_block_in_reach_still_refuses_a_screenshot(self):
        building = {"min": [150, 75, -262], "max": [156, 79, -256]}
        probe = Mock()
        probe.observe.return_value = {"player": {"x": 150.5, "y": 76.0, "z": -254.5},
                                      "target": {"type": "none"}}
        probe.call.return_value = {"blocks": [
            {"x": 150, "y": y, "z": -255, "block_id": block, "collision_empty": empty,
             "fluid": {"empty": True}}
            for y, block, empty in ((75, "minecraft:cobblestone", False),
                                    (76, "minecraft:air", True), (77, "minecraft:air", True),
                                    (78, "minecraft:birch_slab", False))], "truncated": False}
        with patch.object(qa.native_client, "capture") as capture:
            with self.assertRaisesRegex(RuntimeError, "occluded or out of reach"):
                qa.capture_target(probe, "artifact", "building", (153.5, 77.5, -258.5),
                                  building=building, solid=[{"x": 153, "y": 77, "z": -259}])
        capture.assert_not_called()

    def test_sparse_building_is_aimed_at_a_solid_block_not_its_air_centre(self):
        building = {"min": [113, 72, -226], "max": [119, 76, -220]}
        solid = [{"x": 113, "y": 72, "z": -226}, {"x": 116, "y": 73, "z": -223}, {"x": 119, "y": 76, "z": -220}]
        self.assertEqual(qa.building_aim_point(building, solid), (116.5, 73.5, -222.5))
        self.assertEqual(qa.building_aim_point(building, []), (116.5, 74.5, -222.5))

    def test_simulation_wait_uses_scenario_deadline_not_mcp_request_cap(self):
        with patch("tools.harness.backends.probe_steps.time.monotonic", return_value=100.0):
            probe = qa.ProbeSteps(Mock(), None, 7200.0, "sprint")
        with patch("tools.harness.backends.probe_steps.time.monotonic", return_value=300.0):
            self.assertEqual(probe.remaining(), 120.0)
            self.assertEqual(probe.remaining(limit=None), 7000.0)
        with patch("tools.harness.backends.probe_steps.time.monotonic", return_value=7300.0):
            with self.assertRaisesRegex(TimeoutError, "sprint deadline expired"):
                probe.remaining(limit=None)

    def observe(self, sprint, error="completion event awaited", workstations=(),
                read_container=None, wait=None, planner=None):
        """Drive scenario() through its snapshots until the expected error."""
        player = {"player": {"x": 1.0, "y": 70.0, "z": 1.0}}
        with patch("tools.harness.backends.probe_steps.time.monotonic", return_value=100.0):
            probe = qa.ProbeSteps(Mock(), None, 7200.0, "sprint")
        probe.wait = Mock(side_effect=wait or (lambda *_, **__: player))
        probe.observe = Mock(return_value=player)
        probe.call = Mock(return_value={
            "entities": [], "visible_count": 0, "truncated": False, "next_cursor": None,
        })

        def empty_planner(probe, detail, chest, read_container, capture):
            capture()
            return {"buildings_now": []}

        def send(probe, command):
            return {"settlement list": ["Settlements: Test."],
                    "settlement claim": ["Claimed Test"]}.get(
                command, ["Test | owner=autonomous anchor=[1, 70, 1] members=5"])

        stations = iter([[], *([list(workstations)] if workstations else [])])
        detail = {}
        with patch.object(qa.time, "monotonic", return_value=100.0), \
             patch.object(qa, "walk_to_village", return_value=player), \
             patch.object(qa.village, "_send", side_effect=send), \
             patch.object(qa.village, "_recent_chat", side_effect=lambda lines, _: lines), \
             patch.object(qa.village, "_founding_storage", return_value=(1, 70, 1)), \
             patch.object(qa.village, "_open_chest", return_value={}), \
             patch.object(qa, "fund", side_effect=lambda p, d, s, c: d.update(certified_buildings_before_stock=[])), \
             patch.object(qa.item_merge, "_console"), \
             patch.object(qa.native_client, "capture", return_value={"path": "frame.png"}), \
             patch.object(qa.village, "_planner_status", side_effect=planner or empty_planner), \
             patch.object(qa.village, "_probe_ecology", return_value={}), \
             patch.object(qa.village, "_gametime", return_value=0), \
             patch.object(qa, "building_blocks", return_value=[]), \
             patch.object(qa, "workstation_blocks", side_effect=lambda *_: next(stations, [])), \
             patch.object(qa, "baseline_server_tick", return_value=120), \
             patch.object(qa.item_merge, "_sprint", side_effect=sprint):
            with self.assertRaisesRegex(RuntimeError, error):
                qa.scenario(probe, detail, "artifact", Mock(), "server.log", read_container or Mock())
        return detail

    def test_waiting_project_materials_do_not_hide_a_stalled_active_project(self):
        # Seed-24 run ufgbv77d: Road 0/33 held the only project slot from sprint
        # 45000 to the 96000 limit while the waiting Farm's dirt rose 3 -> 16; the
        # whole dialog text changed, so the one-day stall oracle never fired.
        road = {"blueprint": "road", "level": 1, "done": 0, "total": 33, "origin": [158, 71, -257]}
        dirt = iter(range(1000))

        def planner(probe, detail, chest, read_container, capture):
            capture()
            return {"buildings_now": [], "projects": [road], "project_ui": [
                {"id": "dialog/body/0", "role": "text", "label": "Road L1 · Building\n0/33\n158 71 -257"},
                {"id": "dialog/body/1", "role": "text",
                 "label": f"Farm:\nDirt {next(dirt)}/30 — awaiting production"}]}

        detail = self.observe(lambda *_: "", "one full game day", planner=planner)
        self.assertEqual(detail["stop"]["since_sprint_tick"], 0)
        self.assertEqual(detail["stop"]["at_sprint_tick"], qa.STALL_TICKS)

    def test_waiting_projects_count_as_progress_without_an_active_project(self):
        dirt = iter(range(1000))

        def planner(probe, detail, chest, read_container, capture):
            capture()
            return {"buildings_now": [], "projects": [], "project_ui": [
                {"id": "dialog/body/0", "role": "text",
                 "label": f"Farm:\nDirt {next(dirt)}/30 — awaiting production"}]}

        self.observe(lambda *_: "", "bounded observation limit", planner=planner)

    def test_a_visible_project_reaches_the_completion_match(self):
        # `_planner_status` reports `projects` (a list), never `project`; the loop
        # read the latter, so no project was ever recorded and `completed_project`
        # could not match a new building: new_house and second_building were
        # unreachable for any village behaviour (2026-11-24 review N1).
        house = {"blueprint": "house", "level": 1, "done": 3, "total": 9, "origin": [1, 70, 1]}
        lamp = {"blueprint": "lamp", "level": 1, "done": 0, "total": 3, "origin": [2, 70, 2]}

        def planner(probe, detail, chest, read_container, capture):
            capture()
            return {"buildings_now": [], "projects": [house, lamp], "project_ui": []}

        detail = self.observe(lambda *_: "", "sustained unchanged project status and buildings "
                            "for one full game day", planner=planner)
        # Only authored blueprints can be matched to a certificate; the street
        # project is recorded as visible but never becomes a completion candidate.
        self.assertEqual(detail["projects_observed"], [house])
        # Every parsed project is signed, not only the slot holder: a completion may
        # be any of them (review D123Civ6 note a), so a waiting project's progress
        # also keeps the day from counting as a stall.
        self.assertEqual(qa.stall_signature(
            {"projects": [house, lamp], "project_ui": [], "buildings_now": []})[0],
            (house, lamp))

    def test_civilization_sprint_wait_can_outlast_one_mcp_request(self):
        def sprint(server, log, ticks, timeout):
            self.assertEqual(ticks, 300)
            self.assertEqual(timeout, 7200.0)
            raise RuntimeError("completion event awaited")

        self.observe(sprint)

    def test_furnace_output_collected_before_the_menu_does_not_abort_the_run(self):
        # Seed-24 run o31rlnkb: the console read saw one ingot, a resident took
        # it before the player's menu opened and the whole gate aborted on
        # furnace_iron_output_visible. The held re-read now finds it gone.
        reads = iter([{"minecraft:iron_ingot": 1}, {"minecraft:raw_iron": 1}])
        player = {"player": {"x": 1.0, "y": 70.0, "z": 1.0}}

        def wait(predicate, label, **kwargs):
            if label == "furnace_iron_output_visible":
                raise TimeoutError("civilization-seed-qa timed out waiting for furnace_iron_output_visible")
            return player

        def sprint(server, log, ticks, timeout):
            raise RuntimeError("completion event awaited")

        detail = self.observe(
            sprint, workstations=[{"x": 134, "y": 78, "z": -217, "block_id": "minecraft:furnace"}],
            read_container=lambda pos: next(reads), wait=wait)
        self.assertNotIn("iron_smelted", detail["milestones"])
        furnace = detail["snapshots"][0]["workstation_blocks"][0]
        self.assertEqual(furnace["iron_menu_skipped"], "output collected before the simulation hold")

    def test_local_building_waits_for_its_chunks_before_the_scan(self):
        # HANDOFF-47 run 3 (20261125T220300-12zuxr8_): a scan of a house 13
        # blocks from the bell failed with "client chunk is not loaded" 122 s
        # into the run. Locality is the observer's distance, not whether the
        # client has streamed the chunk: a local scan asks for its chunks too.
        building = {"natural": False, "min": [144, 79, -224], "max": [150, 84, -218]}
        probe = Mock()
        probe.remaining.side_effect = lambda limit=None: min(120.0, limit or 120.0)
        probe.call.return_value = {"blocks": [], "truncated": False}
        probe.observe.return_value = {"player": {"x": 131.5, "y": 78.0, "z": -217.5}}
        with patch.object(qa.village, "_listed_buildings", return_value=[building]):
            evidence = qa.building_blocks(probe, [], Mock())
        calls = [call.args[0] for call in probe.call.call_args_list]
        self.assertEqual(calls[0], "minecraft_wait_for_loaded_block",
                         "the local scan must wait for its chunks, not scan blind")
        self.assertIn("minecraft_scan_blocks", calls)
        self.assertFalse(evidence[0]["observer_teleported"])

    def test_remote_building_loads_each_chunk_and_returns_observer(self):
        building = {"natural": False, "min": [234, 72, -213], "max": [240, 76, -207]}
        probe = Mock()
        probe.remaining.side_effect = lambda limit=None: min(120.0, limit or 120.0)
        probe.call.return_value = {"blocks": [{"block_id": "minecraft:red_bed"}], "truncated": False}
        server = Mock()
        moves: list[str] = []
        # The tp lands the observer outside the footprint (lo[0] - 4, review M1:
        # a creative observer inside the bounds tramples crops), so the mock
        # reports that stand after the first console move.
        probe.observe.side_effect = lambda: {"player":
            {"x": 230.0, "y": 79.0, "z": -210.0} if len(moves) == 1
            else {"x": 131.5, "y": 78.0, "z": -217.5}}
        with patch.object(qa.village, "_listed_buildings", return_value=[building]), \
             patch.object(qa.item_merge, "_console", side_effect=lambda _s, command: moves.append(command)) as console:
            evidence = qa.building_blocks(probe, [], server)
        self.assertEqual(evidence[0]["blocks"][0]["block_id"], "minecraft:red_bed")
        self.assertTrue(evidence[0]["observer_teleported"])
        self.assertTrue(evidence[0]["observer_moved"])
        # A remote menu holds the observer, so the scan closes it first, loads each
        # chunk of the 2x2 chunk footprint, scans, and returns the observer through
        # the console tp with a bounded horizontal wait (move_observer).
        self.assertEqual([call.args[0] for call in probe.call.call_args_list], [
            "minecraft_close_screen",
            "minecraft_wait_for_loaded_block", "minecraft_wait_for_loaded_block",
            "minecraft_wait_for_loaded_block", "minecraft_wait_for_loaded_block",
            "minecraft_scan_blocks", "minecraft_wait_for_loaded_block",
            "minecraft_close_screen"])
        self.assertEqual(console.call_count, 2)
        landed_x = float(moves[0].split()[2])
        self.assertLess(landed_x, 234, "the observer must land outside the scanned bounds")
        self.assertGreaterEqual(landed_x, 228.0)

    def test_iron_capture_opens_furnace_and_requires_visible_output(self):
        probe = Mock()
        empty = {"container": {"container_id": 4,
                               "menu_class": "net.minecraft.world.inventory.FurnaceMenu", "slots": []}}
        full = {"container": {"container_id": 4,
                              "menu_class": "net.minecraft.world.inventory.FurnaceMenu",
                              "slots": [{"slot": 2, "item_id": "minecraft:iron_ingot", "count": 2}]}}
        events = []

        def wait(predicate, label, **kwargs):
            self.assertFalse(predicate(empty))
            self.assertTrue(predicate(full))
            events.append("menu")
            return full

        def read(pos):
            events.append(("read", pos))
            return {"minecraft:iron_ingot": 2}

        probe.wait.side_effect = wait
        with patch.object(qa.native_client, "capture", return_value={"retained_path": "iron.png"}) as capture, \
             patch.object(qa.item_merge, "_console", side_effect=lambda _, command: events.append(command)):
            result = qa.capture_iron_menu(probe, "server", read, "artifact", (1, 2, 3), 600)
        self.assertEqual(result["container"], full["container"])
        capture.assert_called_once_with(probe.client, "artifact", "civilization-iron-000600")
        self.assertEqual([call.args[0] for call in probe.call.call_args_list],
                         ["minecraft_close_screen", "minecraft_set_hotbar_slot",
                          "minecraft_use_item_on", "minecraft_close_screen"])
        self.assertEqual(events, ["tick freeze", ("read", (1, 2, 3)), "menu", "tick unfreeze"])

    def test_iron_collected_by_a_resident_before_the_hold_is_retried_not_fatal(self):
        # Seed-24 run o31rlnkb: the console read saw one ingot, a resident
        # took it before the player's menu opened, and the run aborted.
        probe = Mock()
        probe.wait.side_effect = TimeoutError("timed out waiting for furnace_iron_output_visible")
        events = []
        with patch.object(qa.item_merge, "_console", side_effect=lambda _, command: events.append(command)):
            result = qa.capture_iron_menu(probe, "server", lambda pos: {"minecraft:raw_iron": 1},
                                          "artifact", (1, 2, 3), 300)
        self.assertIsNone(result)
        probe.call.assert_not_called()
        self.assertEqual(events, ["tick freeze", "tick unfreeze"])

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
                if census == 3:
                    raise RuntimeError("observation begins")
                return {"entities": [], "visible_count": 0, "truncated": False, "next_cursor": None}
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


class ApproachRetryTests(unittest.TestCase):
    """20261124T035536: a walking villager left the client's received set mid-approach
    and the client refused with a non-retryable error, aborting the whole gate."""

    def test_a_baby_that_leaves_the_received_set_mid_approach_is_approached_again(self):
        baby = {"entity_id": 7, "entity_uuid": "b7", "entity_type": "minecraft:villager",
                "received_baby": True, "x": 130.5, "y": 78.0, "z": -218.5}
        mover = {**baby, "x": 131.5}
        calls = {"approach": 0}

        def call(name, arguments):
            if name == "minecraft_approach_entity":
                calls["approach"] += 1
                if calls["approach"] == 1:
                    raise RuntimeError('minecraft_approach_entity failed: {"code": "entity identity '
                                       'left client-visible state while approaching", "retryable": false}')
                return {"in_reach": True}
            if name == "minecraft_list_entities":
                return {"entities": [mover], "visible_count": 1, "truncated": False,
                        "next_cursor": None}
            if name == "minecraft_wait_ticks":
                return {}
            return {}

        probe = Mock()
        probe.call.side_effect = call
        probe.observe.return_value = {"player": {"x": 130.0, "y": 78.0, "z": -215.0}}
        hit = {"type": "entity", "entity_uuid": "b7"}
        with patch.object(qa, "_aim", return_value=hit):
            observed, teleported = qa._aim_at_baby(probe, baby, None)
        self.assertEqual(calls["approach"], 2, "the lost identity is approached again")
        self.assertEqual(observed, hit)
        self.assertFalse(teleported)


    def test_a_not_client_visible_refusal_is_approached_again_not_fatal(self):
        # Run 20261125T040502: the same class of failure arrived in another wording,
        # "entity is not client-visible", and aborted the gate before verdicts.
        baby = {"entity_id": 7, "entity_uuid": "b7", "entity_type": "minecraft:villager",
                "received_baby": True, "x": 130.5, "y": 78.0, "z": -218.5}
        calls = {"approach": 0}

        def call(name, arguments):
            if name == "minecraft_approach_entity":
                calls["approach"] += 1
                if calls["approach"] == 1:
                    raise RuntimeError('minecraft_approach_entity failed: {"code": "entity is not '
                                       'client-visible", "retryable": false, "stage": "automation"}')
                return {"in_reach": True}
            if name == "minecraft_list_entities":
                return {"entities": [baby], "visible_count": 1, "truncated": False,
                        "next_cursor": None}
            return {}

        probe = Mock()
        probe.call.side_effect = call
        probe.observe.return_value = {"player": {"x": 130.0, "y": 78.0, "z": -215.0}}
        hit = {"type": "entity", "entity_uuid": "b7"}
        with patch.object(qa, "_aim", return_value=hit):
            observed, teleported = qa._aim_at_baby(probe, baby, None)
        self.assertEqual(calls["approach"], 2, "the not-visible refusal is retried, not fatal")
        self.assertEqual(observed, hit)
        self.assertFalse(teleported)

    def test_a_baby_inside_a_house_is_observed_from_beside_it(self):
        # 20261124T205323: the observer was teleported onto the baby's own cell
        # inside its house and the raycast hit cobblestone at 120,71,-274, which
        # aborted the whole gate before any other milestone was observed.
        baby = {"entity_id": 7, "entity_uuid": "b7", "entity_type": "minecraft:villager",
                "received_baby": True, "x": 120.5, "y": 71.0, "z": -274.5}
        approaches = []
        teleports = []

        def call(name, arguments):
            if name == "minecraft_approach_entity":
                approaches.append(1)
                return {"in_reach": len(approaches) > 1}
            if name == "minecraft_list_entities":
                return {"entities": [baby], "visible_count": 1, "truncated": False,
                        "next_cursor": None}
            return {}

        probe = Mock()
        probe.call.side_effect = call
        probe.observe.return_value = {"player": {"x": 120.0, "y": 71.0, "z": -272.0}}
        hit = {"type": "entity", "entity_uuid": "b7"}
        with patch.object(qa, "_aim", return_value=hit):
            observed, teleported = qa._aim_at_baby(
                probe, baby, lambda x, y, z: teleports.append((x, y, z)))
        self.assertTrue(teleported)
        self.assertEqual(observed, hit)
        self.assertTrue(teleports, "the unreachable baby still needs the observer beside it")
        for x, _y, z in teleports:
            self.assertGreaterEqual(
                abs(x - baby["x"]) + abs(z - baby["z"]), 1.0,
                f"the observer must stand beside the baby, never on its own cell: {teleports}")


class GatedCensusTests(unittest.TestCase):
    """Seed 24 (2026-11-24): the 128-block all-entity census never stabilised over a
    village that harvests into the world, so the gate refused before its milestones."""

    def test_items_that_churn_between_pages_no_longer_refuse_the_census(self):
        villager = {"entity_id": 1, "entity_uuid": "v1", "entity_type": "minecraft:villager",
                    "x": 130.5, "y": 78.0, "z": -216.5}
        sheep = {"entity_id": 2, "entity_uuid": "s1", "entity_type": "minecraft:sheep",
                 "x": 140.5, "y": 70.0, "z": -220.5}
        pages = 0

        def call(name, arguments):
            nonlocal pages
            if name != "minecraft_list_entities":
                return {}
            kind = arguments.get("entity_type")
            if kind is None:
                # Unfiltered: an item dropped between the two pages moves the count.
                pages += 1
                first = pages % 2 == 1
                row = {"entity_id": 900 + pages, "entity_uuid": f"i{pages}",
                       "entity_type": "minecraft:item", "x": 131.5, "y": 77.0, "z": -216.5}
                return {"entities": [row] if first else [row, {**row, "entity_id": 950}],
                        "visible_count": 100 if first else 101,
                        "truncated": first, "next_cursor": "1" if first else None}
            rows = [villager] if kind == "minecraft:villager" else [sheep]
            return {"entities": rows, "visible_count": len(rows),
                    "truncated": False, "next_cursor": None}

        probe = Mock()
        probe.call.side_effect = call
        with self.assertRaisesRegex(RuntimeError, "membership changed"):
            qa.entity_census(probe)
        census = qa.gated_census(probe)
        self.assertEqual([row["entity_type"] for row in census["entities"]],
                         ["minecraft:villager", "minecraft:sheep"])
        self.assertFalse(census["truncated"])
        self.assertEqual(census["census_types"], ["minecraft:villager", "minecraft:sheep"])


class ObserverMoveTests(unittest.TestCase):
    """20261124T170423: the observer's console teleport landed 1.5 blocks off the
    requested column and 6 below the requested height, the unbounded wait for that
    landing spent the last 1 h 45 min of the scenario, and the milestones the world
    completed meanwhile (house L1 at server tick 99 816) were never observed."""

    def _probe(self, states, *, clock=None):
        probe = Mock()
        probe.remaining.side_effect = lambda *, limit=120.0: min(1800.0, limit)
        probe.waits = []
        sequence = list(states)

        def observe():
            current = sequence.pop(0) if len(sequence) > 1 else sequence[0]
            return {"player": dict(current)}

        def wait(predicate, label, *, timeout_seconds=None):
            probe.waits.append((label, timeout_seconds))
            return {"player": dict(sequence[0])}

        def call(name, arguments=None):
            if clock is not None:
                clock["t"] += 1.0
            return {"blocks": [], "truncated": False}

        probe.observe.side_effect = observe
        probe.wait.side_effect = wait
        probe.call.side_effect = call
        return probe

    def test_an_observer_that_never_lands_cannot_consume_the_scenario(self):
        clock = {"t": 0.0}
        # Requested point of the farm at [78,68,-320]..[84,72,-311] is (81, 75, -315.5);
        # the server leaves the observer on the field at (82.5, 69, -317).
        probe = self._probe([{"x": 0.0, "y": 64.0, "z": 0.0},
                             {"x": 82.5, "y": 69.0, "z": -317.0}], clock=clock)
        lines = ["Buildings: #7 farm level 1 [78, 68, -320]-[84, 72, -311]"]
        with patch.object(qa, "time", SimpleNamespace(monotonic=lambda: clock["t"])):
            evidence = qa.building_blocks(probe, lines, Mock())
        self.assertEqual(evidence[0].get("observer_moved"), False,
                         "a landing the server clamps is a recorded miss, not a hang")
        self.assertEqual(probe.wait.call_count, 0,
                         f"the observer move must never use an unbounded probe.wait: {probe.waits}")
        self.assertLessEqual(probe.call.call_count, 2 * int(qa.OBSERVER_MOVE_TIMEOUT) + 8,
                             "both observer moves are bounded by their own timeout")

    def test_a_landing_below_the_requested_height_is_still_an_arrival(self):
        clock = {"t": 0.0}
        probe = self._probe([{"x": 82.5, "y": 69.0, "z": -316.0}], clock=clock)
        with patch.object(qa, "time", SimpleNamespace(monotonic=lambda: clock["t"])):
            self.assertTrue(qa.move_observer(probe, {"x": 82.0, "y": 75.0, "z": -316.0}))
        self.assertEqual(probe.call.call_count, 0)

    def test_a_remote_building_is_observed_after_the_observer_moves_there(self):
        events = []
        probe = Mock()
        probe.remaining.side_effect = lambda *, limit=120.0: min(600.0, limit)
        probe.observe.return_value = {"player": {"x": 131.0, "y": 78.0, "z": -217.0}}

        stand = [{"x": 77, "y": 66, "z": -316, "collision_empty": False, "is_air": False},
                 {"x": 77, "y": 67, "z": -316, "collision_empty": True, "is_air": True, "fluid": {"empty": True}},
                 {"x": 77, "y": 68, "z": -316, "collision_empty": True, "is_air": True, "fluid": {"empty": True}}]

        def call(name, arguments=None):
            events.append(name)
            if name == "minecraft_scan_blocks":
                return {"blocks": stand, "truncated": False}
            return {}

        def teleport(x, y, z):
            events.append(("tp", round(x, 1), round(y, 1), round(z, 1)))

        probe.call.side_effect = call
        building = {"min": [78, 68, -320], "max": [84, 72, -311], "kind": "farm"}
        seen = {"type": "block", "x": 81, "y": 68, "z": -316}
        with patch.object(qa, "_aim", return_value=seen), \
             patch.object(qa.native_client, "capture", return_value={}):
            qa.capture_target(probe, Path("/tmp"), "civilization-building", (81.0, 68.0, -316.0),
                              building=building, solid=[], teleport=teleport)
        teleports = [index for index, event in enumerate(events) if isinstance(event, tuple)]
        scans = [index for index, event in enumerate(events) if event == "minecraft_scan_blocks"]
        self.assertTrue(teleports and scans and min(teleports) < min(scans),
                        f"the observer must move to the site before its faces are scanned: {events}")

    def test_an_observer_that_never_lands_stops_at_its_own_timeout(self):
        clock = {"t": 0.0}
        probe = self._probe([{"x": 0.0, "y": 64.0, "z": 0.0}], clock=clock)
        with patch.object(qa, "time", SimpleNamespace(monotonic=lambda: clock["t"])):
            self.assertFalse(qa.move_observer(probe, {"x": 100.0, "y": 70.0, "z": 100.0}))
        self.assertLessEqual(probe.call.call_count, int(qa.OBSERVER_MOVE_TIMEOUT) + 1)



class IronEvidenceTests(unittest.TestCase):
    """CIV-B2 (1) review: which observation proves the town smelted iron."""

    def test_a_town_furnace_counter_proves_a_smelt(self) -> None:
        furnace = {"container_totals": {}, "recipes_used": {"minecraft:iron_ingot_from_smelting_raw_iron": 2}}
        self.assertEqual(qa.iron_evidence(furnace, baseline=False), "furnace_recipes")

    def test_a_stored_ingot_is_the_stronger_evidence(self) -> None:
        furnace = {"container_totals": {"minecraft:iron_ingot": 1},
                   "recipes_used": {"minecraft:iron_ingot_from_smelting_raw_iron": 1}}
        self.assertEqual(qa.iron_evidence(furnace, baseline=False), "output_slot")

    def test_a_baseline_furnace_with_a_prewritten_counter_proves_nothing(self) -> None:
        # A furnace that stood before the fixture can carry a counter an earlier
        # world or a reviewer fixture wrote, so only the output slot would count.
        furnace = {"container_totals": {}, "recipes_used": {"minecraft:iron_ingot_from_smelting_raw_iron": 7}}
        self.assertIsNone(qa.iron_evidence(furnace, baseline=True))
        self.assertIsNone(qa.iron_evidence({"container_totals": {"minecraft:iron_ingot": 1}}, baseline=True))

    def test_empty_zero_and_blasting_only_counters_are_no_evidence(self) -> None:
        for furnace in ({}, {"container_totals": {}, "recipes_used": {}},
                        {"recipes_used": {"minecraft:iron_ingot_from_smelting_raw_iron": 0}},
                        {"recipes_used": {"minecraft:iron_ingot_from_blasting_raw_iron": 3}}):
            with self.subTest(furnace=furnace):
                self.assertIsNone(qa.iron_evidence(furnace, baseline=False))

    def test_a_fixture_command_that_writes_block_entity_nbt_fails_closed(self) -> None:
        qa.assert_no_evidence_writes(["console: gamemode creative VillageAuto",
                                      "console: time set day", "console: weather clear 24000"])
        for command in ("console: data merge block 132 78 -222 {RecipesUsed: {}}",
                        "console: setblock 132 78 -222 minecraft:furnace{RecipesUsed: {\"minecraft:iron_ingot_from_smelting_raw_iron\": 9}}",
                        "console: data modify block 132 78 -222 recipes_used set value 9"):
            with self.subTest(command=command):
                with self.assertRaisesRegex(RuntimeError, "NBT"):
                    qa.assert_no_evidence_writes([command])


class BabyCaptureRetryTests(unittest.TestCase):
    """Review wave2-10: run 20261124T232341 aborted after 947 s when the listed
    newborn left the client's view before its capture."""

    def test_a_baby_that_left_the_view_is_a_miss_not_an_abort(self) -> None:
        baby = {"entity_uuid": "new", "x": 160.5, "y": 75.0, "z": -280.5}
        with patch.object(qa, "capture_target",
                          side_effect=qa._BabyLeftView("baby left the client-visible set")):
            observation, miss = qa.capture_baby(Mock(), "artifact", baby, 600, Mock())
        self.assertIsNone(observation, "a vanished baby yields no observation")
        self.assertIn("left the client-visible set", miss)
        self.assertEqual(qa.BABY_CAPTURE_ATTEMPTS, 12)

    def test_the_next_snapshot_captures_the_same_baby(self) -> None:
        baby = {"entity_uuid": "new", "x": 160.5, "y": 75.0, "z": -280.5}
        shot = {"retained_path": "baby.png"}
        with patch.object(qa, "capture_target", side_effect=[qa._BabyLeftView("gone"), shot]):
            first, miss = qa.capture_baby(Mock(), "artifact", baby, 600, Mock())
            second, second_miss = qa.capture_baby(Mock(), "artifact", baby, 900, Mock())
        self.assertIsNone(first)
        self.assertIsNotNone(miss)
        self.assertEqual(second["screenshot"], shot)
        self.assertEqual(second["sprint_ticks"], 900)
        self.assertIsNone(second_miss)

    def test_an_occluded_baby_is_a_miss_not_an_abort(self) -> None:
        # Run 20261125T005852: the observer stood on the roof and the aim hit
        # dirt; _aim_at_baby now reports that as a miss for the next snapshot.
        baby = {"entity_uuid": "new", "entity_id": 17, "x": 114.5, "y": 70.0, "z": -278.5}
        probe = Mock()
        probe.call.return_value = {"in_reach": False}
        with patch.object(qa, "entity_census", return_value={"entities": [baby]}), \
             patch.object(qa, "_aim", return_value={"type": "block", "block_id": "minecraft:dirt"}):
            with self.assertRaises(qa._BabyLeftView):
                qa._aim_at_baby(probe, baby, lambda *_: None)
        self.assertEqual(qa.BABY_AIM_ATTEMPTS, 10)

    def test_a_hard_capture_error_still_aborts(self) -> None:
        # The old refusal: a RuntimeError other than _BabyLeftView is a real
        # gate failure and must still propagate.
        baby = {"entity_uuid": "new", "x": 160.5, "y": 75.0, "z": -280.5}
        with patch.object(qa, "capture_target",
                          side_effect=RuntimeError("baby no longer visible for milestone capture")):
            with self.assertRaisesRegex(RuntimeError, "baby no longer visible"):
                qa.capture_baby(Mock(), "artifact", baby, 600, Mock())

if __name__ == "__main__":
    unittest.main()


class RemoteHouseDoorTests(unittest.TestCase):
    """HARNESS-CIV-GATE-HOUSE-DOOR-UNLOADED: final-pin 20261126T092432 and candidate 20261126T190711."""

    DOOR = {"block_id": "minecraft:oak_door", "x": 96, "y": 64, "z": -297,
            "properties": {"half": "lower", "facing": "north", "open": "true"}}
    DOOR_XYZ = (96, 64, -297)

    def _blocks(self):
        """The house envelope plus one standable exterior column beside the door."""
        blocks = [{"x": 93, "y": 63, "z": -297, "block_id": "minecraft:cobblestone",
                   "collision_empty": False, "fluid": {"empty": True}},
                  {"x": 99, "y": 67, "z": -291, "block_id": "minecraft:cobblestone",
                   "collision_empty": False, "fluid": {"empty": True}}]
        for y, block, empty in ((63, "minecraft:cobblestone", False),
                                (64, "minecraft:air", True), (65, "minecraft:air", True)):
            blocks.append({"x": 96, "y": y, "z": -298, "block_id": block,
                           "collision_empty": empty, "fluid": {"empty": True}})
        blocks.append(self.DOOR)
        return blocks

    def _probe(self, player, walk):
        """`walk(name, arguments)` decides each navigation; returns the probe."""
        probe = Mock()
        probe.observe.side_effect = lambda: {"player": dict(player)}

        def call(name, arguments):
            if name == "minecraft_navigate_to_block":
                rejected = walk(arguments)
                if rejected:
                    raise RuntimeError(rejected)
                if (arguments["x"], arguments["y"], arguments["z"]) != self.DOOR_XYZ:
                    player.update(x=float(arguments["x"]), y=float(arguments["y"]),
                                  z=float(arguments["z"]))
                return {"status": "ok"}
            if name == "minecraft_wait_for_loaded_block":
                return {"block_id": "minecraft:cobblestone"}
            if name == "minecraft_read_block":
                return self.DOOR
            return {}
        probe.call.side_effect = call
        return probe

    def test_a_remote_door_loads_its_chunk_then_walks_straight_to_the_capture(self):
        player = {"x": 132.2, "y": 77.0, "z": -215.5}
        probe = self._probe(player, lambda arguments: None)
        target = (96.5, 64.5, -297.5)
        with patch.object(qa, "capture_target", return_value={"retained_path": "house.png"}) as capture:
            shot = qa.capture_house(probe, "artifact", "house", target, self._blocks())
        self.assertEqual(shot["retained_path"], "house.png")
        self.assertFalse(shot["observer_teleported"])
        loaded = [call.args[1] for call in probe.call.call_args_list
                  if call.args[0] == "minecraft_wait_for_loaded_block"]
        self.assertEqual((loaded[0]["x"], loaded[0]["z"]), (96, -297))
        walks = [call.args[1] for call in probe.call.call_args_list
                 if call.args[0] == "minecraft_navigate_to_block"]
        self.assertEqual((walks[0]["x"], walks[0]["z"]), (96, -297))
        self.assertEqual((walks[-1]["x"], walks[-1]["z"]), (132, -216))
        self.assertEqual(capture.call_args.args, (probe, "artifact", "house", target))

    def test_a_refused_straight_walk_takes_a_lateral_hop_then_the_door(self):
        player = {"x": 132.2, "y": 77.0, "z": -215.5}
        refused = {"ahead": 0}

        def walk(arguments):
            if (arguments["x"], arguments["y"], arguments["z"]) == self.DOOR_XYZ:
                # remote: the door's chunk is not held until the hops get close
                if math.dist((player["x"], player["z"]), (96, -297)) > 64:
                    return ('minecraft_navigate_to_block failed: {"code": "target chunk is not '
                            'client-loaded: 96, 64, -297", "retryable": false}')
                return None
            # the straight ahead candidate is unreachable; a lateral one is not
            if arguments["z"] == -253 and arguments["x"] == 115 and refused["ahead"] == 0:
                refused["ahead"] += 1
                return ('minecraft_navigate_to_block failed: {"code": "target is unreachable '
                        'from the current loaded client position: 115, 76, -253", "retryable": false}')
            return None
        probe = self._probe(player, walk)
        with patch.object(qa, "capture_target", return_value={"retained_path": "house.png"}) as capture:
            shot = qa.capture_house(probe, "artifact", "house", (96.5, 64.5, -297.5), self._blocks())
        self.assertFalse(shot["observer_teleported"])
        walks = [call.args[1] for call in probe.call.call_args_list
                 if call.args[0] == "minecraft_navigate_to_block"]
        hops = [walk for walk in walks[:-1]          # the last walk is the return to outside
                if (walk["x"], walk["y"], walk["z"]) != self.DOOR_XYZ]
        self.assertGreaterEqual(len(hops), 2)         # ahead refused, a lateral one walked
        start = (132, -216)
        for previous, following in zip([start] + [(hop["x"], hop["z"]) for hop in hops],
                                       [(hop["x"], hop["z"]) for hop in hops]):
            self.assertLessEqual(math.dist(previous, following), qa.DOOR_HOP_BLOCKS + qa.DOOR_SIDESTEP_BLOCKS)
        capture.assert_called_once()

    def test_a_door_no_hop_can_reach_is_observed_from_the_console_teleport(self):
        player = {"x": 132.2, "y": 77.0, "z": -215.5}
        refusal = ('minecraft_navigate_to_block failed: {"code": "target is unreachable from '
                   'the current loaded client position", "retryable": false}')

        def walk(arguments):
            return ('minecraft_navigate_to_block failed: {"code": "target chunk is not '
                    'client-loaded", "retryable": false}'
                    if (arguments["x"], arguments["y"], arguments["z"]) == self.DOOR_XYZ else refusal)
        probe = self._probe(player, walk)
        teleport = Mock()
        with patch.object(qa, "DOOR_HOP_LIMIT", 2), \
                patch.object(qa, "capture_target", return_value={"retained_path": "house.png"}):
            shot = qa.capture_house(probe, "artifact", "house", (96.5, 64.5, -297.5),
                                    self._blocks(), teleport=teleport)
        self.assertTrue(shot["observer_teleported"])
        # door facing north: the player stands on its south side, one step out
        self.assertEqual(teleport.call_args_list[0].args, (96.5, 64.0, -295.0))
        self.assertEqual(teleport.call_args_list[-1].args, (132.2, 77.0, -215.5))

    def test_a_door_no_hop_can_reach_without_a_teleport_fails_with_a_named_error(self):
        player = {"x": 132.2, "y": 77.0, "z": -215.5}
        refusal = ('minecraft_navigate_to_block failed: {"code": "target is unreachable from '
                   'the current loaded client position", "retryable": false}')

        def walk(arguments):
            return ('minecraft_navigate_to_block failed: {"code": "target chunk is not '
                    'client-loaded", "retryable": false}'
                    if (arguments["x"], arguments["y"], arguments["z"]) == self.DOOR_XYZ else refusal)
        probe = self._probe(player, walk)
        with patch.object(qa, "DOOR_HOP_LIMIT", 2), patch.object(qa, "capture_target") as capture:
            with self.assertRaisesRegex(RuntimeError, "hop 1 was refused on every candidate"):
                qa.capture_house(probe, "artifact", "house", (96.5, 64.5, -297.5), self._blocks())
        capture.assert_not_called()


class PlannerReadRecoveryTests(unittest.TestCase):
    """HARNESS-CIV-GATE-PLANNER-READ-BLOCKED-BY-STATUS-DIALOG and -STORAGE-READ-TIMEOUT."""

    DIALOG = "net.minecraft.client.gui.screens.dialog.MultiButtonDialogScreen"
    CLEAR = {"screen": {"class": "none", "open": False}}

    def _probe(self, screens):
        probe = Mock()
        probe.observe.side_effect = lambda: screens.pop(0) if screens else dict(self.CLEAR)
        return probe

    def _status(self, probe, stock):
        with patch.object(village, "_project_status", return_value={"ui_nodes": []}), \
                patch.object(village, "_project_progress", return_value=[]), \
                patch.object(village, "_recent_chat", return_value=[]), \
                patch.object(village, "_send", return_value=[]), \
                patch.object(village, "_settlement_stock", side_effect=stock):
            return village._planner_status(probe, {}, (98, 67, -155), lambda pos: {})

    def test_a_status_dialog_over_the_view_is_closed_and_the_read_retried(self):
        probe = self._probe([{"screen": {"class": self.DIALOG, "open": True}}, dict(self.CLEAR)])
        status = self._status(probe, [
            TimeoutError("storage read of block 98,67,-155 got no reply in 15.0s"),
            {"minecraft:cobblestone": 50},
        ])
        self.assertNotIn("storage_error", status)
        self.assertEqual(status["storage_now"], {"minecraft:cobblestone": 50})
        self.assertIn(call("minecraft_close_screen", {}), probe.call.call_args_list)

    def test_a_dialog_that_never_closes_names_itself_in_the_storage_error(self):
        dialog = {"screen": {"class": self.DIALOG, "open": True}}
        probe = Mock()
        probe.observe.side_effect = lambda: dict(dialog)
        with patch.object(village, "_settlement_stock", side_effect=TimeoutError("no reply")) as stock:
            with patch.object(village, "_project_status", return_value={"ui_nodes": []}), \
                    patch.object(village, "_project_progress", return_value=[]), \
                    patch.object(village, "_recent_chat", return_value=[]), \
                    patch.object(village, "_send", return_value=[]):
                status = village._planner_status(probe, {}, (98, 67, -155), lambda pos: {})
        self.assertEqual(stock.call_count, 2)
        self.assertIn("screen stayed open", status["storage_error"])
        self.assertIn(self.DIALOG, status["storage_error"])

    def test_a_console_reply_embedded_in_a_perf_line_is_accepted(self):
        payload = ('98, 67, -155 has the following block data: {Items: [{Slot: 0b, count: 50, '
                   'id: "minecraft:cobblestone"}], id: "minecraft:barrel", x: 98, y: 67, z: -155}')
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "server.log"
            log.write_bytes(b"")

            class _Stdin:
                def write(self, data):
                    # The perf line has no trailing newline yet: the reply lands mid-line
                    # exactly as run 20261126T101004 logged it (`... post=0.0398, 67, -155 has`).
                    with log.open("ab") as stream:
                        stream.write(b"perf: tick outer ms: tick=141292 pre=0.02 run=1.91 post=0.03")
                        stream.write(payload.encode())

                def flush(self):
                    pass

            server = SimpleNamespace(stdin=_Stdin())
            totals = village._read_storage_container(server, log, (98, 67, -155), 2.0)
        self.assertEqual(totals, {"minecraft:cobblestone": 50})


class RemoteStorageScanTests(unittest.TestCase):
    """The settlement's own Storage can stand beyond the observer's loaded radius."""

    LINE = "#3 storage level 1 [28, 66, -170]-[32, 70, -168]"

    def _probe(self, outcomes):
        probe = Mock()
        probe.remaining.return_value = 300.0
        scans = iter(outcomes)

        def call(name, arguments):
            if name == "minecraft_scan_blocks":
                outcome = next(scans)
                if isinstance(outcome, Exception):
                    raise outcome
                return outcome
            if name == "minecraft_wait_for_loaded_block":
                return {"block_id": "minecraft:cobblestone"}
            return {}
        probe.call.side_effect = call
        return probe

    def test_a_remote_storage_scan_waits_for_one_loaded_cell_then_reads(self):
        refusal = RuntimeError('minecraft_scan_blocks failed: {"code": "client chunk is not '
                               'loaded at 28, 68, -168", "retryable": false, "stage": "automation"}')
        chest = {"x": 30, "y": 66, "z": -169, "block_id": "minecraft:chest"}
        probe = self._probe([refusal, {"blocks": [chest], "truncated": False}])
        stock = village._settlement_stock(probe, {}, (30, 66, -169), [self.LINE],
                                         lambda pos: {"minecraft:birch_log": 128})
        self.assertEqual(stock, {"minecraft:birch_log": 128})
        waits = [call.args[1] for call in probe.call.call_args_list
                 if call.args[0] == "minecraft_wait_for_loaded_block"]
        self.assertEqual(len(waits), 1)
        self.assertEqual((waits[0]["x"], waits[0]["z"]), (28, -170))

    def test_a_storage_that_stays_unloaded_names_its_bounds(self):
        refusal = RuntimeError('minecraft_scan_blocks failed: {"code": "client chunk is not '
                               'loaded at 28, 68, -168", "retryable": false}')
        probe = self._probe([refusal, refusal])
        with self.assertRaisesRegex(RuntimeError, r"owned storage at \(28, 66, -170\)"):
            village._settlement_stock(probe, {}, (30, 66, -169), [self.LINE],
                                      lambda pos: {"minecraft:birch_log": 128})

    def test_a_non_chunk_refusal_is_not_retried(self):
        refusal = RuntimeError("minecraft_scan_blocks failed: server refused the read")
        probe = self._probe([refusal])
        with self.assertRaisesRegex(RuntimeError, "server refused the read"):
            village._settlement_stock(probe, {}, (30, 66, -169), [self.LINE],
                                      lambda pos: {"minecraft:birch_log": 128})
