"""Evidence scanning must not turn generated/natural geometry into stage proof."""
import copy
import unittest
from unittest.mock import Mock, patch
from tempfile import TemporaryDirectory
from pathlib import Path

from .backends import civilization_seed_qa as qa


class CivilizationEvidenceTests(unittest.TestCase):
    def test_entity_census_retains_all_pages_including_a_distant_newborn(self):
        entities = [{"entity_id": index, "entity_uuid": str(index),
                     "entity_type": "minecraft:villager", "received_baby": index == 599}
                    for index in range(600)]
        probe = Mock()
        def page(name, arguments):
            self.assertEqual(name, "minecraft_list_entities")
            self.assertEqual((arguments["radius"], arguments["limit"]), (128, 16))
            start = int(arguments["cursor"]) + 1 if arguments["cursor"] else 0
            rows = entities[start:start + 16]
            more = start + len(rows) < len(entities)
            return {"entities": rows, "visible_count": len(entities),
                    "truncated": more, "next_cursor": str(rows[-1]["entity_id"]) if more else None}
        probe.call.side_effect = page
        census = qa.entity_census(probe)
        self.assertEqual(census["entities"], entities)
        self.assertFalse(census["truncated"])
        self.assertEqual(qa.newborn(census, {str(index) for index in range(599)}, True)["entity_id"], 599)

    def test_entity_census_never_accepts_an_incomplete_or_repeated_page(self):
        for pages in (
            [{"entities": [], "visible_count": 1, "truncated": False, "next_cursor": None}],
            [{"entities": [], "visible_count": 1, "truncated": True, "next_cursor": None}],
            [{"entities": [{"entity_id": 1}], "visible_count": 2, "truncated": True, "next_cursor": "1"},
             {"entities": [{"entity_id": 1}], "visible_count": 2, "truncated": False, "next_cursor": None}],
            [{"entities": [{"entity_id": 1}], "visible_count": 3, "truncated": True, "next_cursor": "1"},
             {"entities": [{"entity_id": 2}], "visible_count": 2, "truncated": False, "next_cursor": None}],
        ):
            with self.subTest(pages=pages):
                probe = Mock()
                probe.call.side_effect = pages
                with self.assertRaises(RuntimeError):
                    qa.entity_census(probe)

    def test_ecology_reuses_the_complete_census(self):
        probe = Mock()
        census = {"entities": [{"entity_type": "minecraft:sheep"}, {"entity_type": "minecraft:villager"}]}
        ecology = qa.village._probe_ecology(probe, {}, [], entities=census)
        self.assertEqual((ecology["sheep"], ecology["villagers"]), (1, 1))
        probe.call.assert_not_called()

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
        baby = {"entity_id": 17, "entity_uuid": "new", "entity_type": "minecraft:villager",
                "received_baby": True, "x": 0, "y": 65.12, "z": 4}
        probe.call.side_effect = lambda name, args=None: {
            "entities": [baby], "visible_count": 1, "truncated": False, "next_cursor": None
        } if name == "minecraft_list_entities" else {}
        probe.observe.return_value = {"player": {"x": 0, "y": 64, "z": 0},
                                      "target": {"type": "entity", "entity_uuid": "new"}}
        with patch.object(qa.native_client, "capture", return_value={"retained_path": "baby.png"}):
            result = qa.capture_target(probe, "artifact", "baby", (0, 65.62, 4), entity=baby)
        self.assertEqual(result["observed_target"]["entity_uuid"], "new")
        probe.call.assert_any_call("minecraft_approach_entity", {"entity_id": 17, "timeout_seconds": 120.0})
        probe.call.assert_any_call("minecraft_list_entities", {
            "radius": 128, "limit": 16, "cursor": "", "entity_type": "minecraft:villager"})
        probe.observe.return_value["target"]["entity_uuid"] = "other"
        with patch.object(qa.native_client, "capture") as capture:
            with self.assertRaisesRegex(RuntimeError, "occluded or out of reach"):
                qa.capture_target(probe, "artifact", "baby", (0, 65.62, 4), entity=baby)
        capture.assert_not_called()

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
        self.assertEqual([c.args for c in teleport.call_args_list],
                         [(160.5, 75.0, -280.5), (131.0, 77.0, -215.0)])

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
            return {"buildings_now": [], "project": road, "project_ui": [
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
            return {"buildings_now": [], "project": None, "project_ui": [
                {"id": "dialog/body/0", "role": "text",
                 "label": f"Farm:\nDirt {next(dirt)}/30 — awaiting production"}]}

        self.observe(lambda *_: "", "bounded observation limit", planner=planner)

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
        self.assertEqual(evidence, [{"building": building, "blocks": blocks[:3], "truncated": True, "solid": []}])
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
                if census == 2:
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


if __name__ == "__main__":
    unittest.main()
