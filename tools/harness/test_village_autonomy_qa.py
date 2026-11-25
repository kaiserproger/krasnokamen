"""Causal client observations and fail-closed autonomous-village receipts."""
from __future__ import annotations

import io
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
from unittest.mock import Mock, patch

from .backends import village_autonomy_qa as qa
from .backends import probe_steps


def setUpModule() -> None:
    # .analysis/ is gitignored, so a fresh checkout or worktree lacks it.
    (qa.runtime.REPO_ROOT / ".analysis").mkdir(exist_ok=True)


def construction_screen(*projects: str, orders: tuple[str, ...] = ()) -> dict:
    body = [f"Active projects: {len(projects)}/8", f"Pending orders: {len(orders)}/8",
            f"Head: {orders[0].split(' · ')[0] if orders else 'none'}"]
    body.extend(f"{project}\nBuilder: builder-{index}" for index, project in enumerate(projects, 1))
    body.extend(f"Pending #{index}: {order}" for index, order in enumerate(orders, 1))
    return {"ui_nodes": [{"id": f"dialog/body/{index}", "label": text}
                         for index, text in enumerate(body)]}


def status_view(*texts: str) -> dict:
    """The client's Status dialog at GUI scale 3 (solaris-client ui/dialog.rs, laptop 22:28 geometry):
    body text from y=99, 27 px per line, 30 px apart; Close at y=639 with the viewport ending
    at y=621. Body nodes starting below the viewport are not observed."""
    nodes, y = [{"id": "dialog/action/3", "label": "Close", "role": "button",
                 "bounds": [414.0, 639.0, 450.0, 60.0]}], 99.0
    for index, text in enumerate(texts):
        height = 27.0 * (text.count("\n") + 1)
        if y < 621.0:
            nodes.append({"id": f"dialog/body/{index}", "label": text, "role": "text",
                          "bounds": [339.0, y, 600.0, height]})
        y += height + 30.0
    return {"ui_nodes": nodes}


class VillageAutonomyQaTests(unittest.TestCase):
    def test_walk_uses_path_planner_for_the_final_loaded_leg(self) -> None:
        probe = Mock()
        initial = {"player": {"x": 128.5, "y": 70.0, "z": 0.5}}
        arrived = {"player": {"x": 120.5, "y": 70.0, "z": 7.5}}
        probe.observe.return_value = initial

        def act(tool, arguments):
            if tool == "minecraft_navigate_to_block":
                probe.observe.return_value = arrived
            return {}

        probe.call.side_effect = act
        with patch.object(qa.time, "monotonic", side_effect=[0.0, 0.0, 601.0]):
            state = qa._walk_to(probe, {}, (120.0, 70.0, 7.0), "approach")
        self.assertEqual(state, arrived)
        probe.call.assert_any_call("minecraft_navigate_to_block", {
            "x": 120, "y": 70, "z": 7, "timeout_seconds": 110.0,
        })
        self.assertFalse(any(call.args[0] == "minecraft_press_inputs"
                             for call in probe.call.call_args_list))

    def test_build_completion_accepts_actual_building_list_level_format(self) -> None:
        probe = Mock()
        probe.observe.return_value = {
            "recent_chat": ["Buildings: #123 storage level 1 [12, 70, 20]-[18, 75, 26]."]
        }
        plaza = {"id": "spanish_plaza", "kind": "storage"}
        storage = {"id": "storage", "kind": "storage"}
        with patch.object(qa, "_authored_blueprints", return_value=[plaza, storage]), patch.object(
            qa, "_observe_blueprint", return_value=True,
        ) as observe, patch.object(qa.time, "monotonic", side_effect=[0.0, 0.0, 100.0, 100.0, 2.0]):
            completed = qa._wait_building_level(
                probe, {}, "Storage", 1, 1.0, (1, 2, 3),
                SimpleNamespace(windowed=False), Path("unused"), "storage",
            )
        self.assertTrue(completed, "CLI reports 'storage level 1', not 'Storage L1'")
        self.assertIs(observe.call_args.args[3], storage)

    def test_chest_waits_for_server_menu_and_excludes_player_inventory(self) -> None:
        probe = Mock()
        probe.observe.return_value = {
            "container": {"container_id": 0, "menu_class": "net.minecraft.world.inventory.InventoryMenu",
                          "slot_count": 46, "slots": [{"slot": 9, "item_id": "minecraft:coal", "count": 64}]}
        }
        stock = {"slot": 0, "item_id": "minecraft:oak_log", "count": 2}
        probe.wait.return_value = {
            "container": {"container_id": 1, "menu_class": "net.minecraft.world.inventory.ChestMenu",
                          "slot_count": 63, "slots": [stock, {"slot": 27, "item_id": "minecraft:coal", "count": 64}]}
        }
        self.assertEqual(qa._open_chest(probe, (1, 2, 3)), {0: stock})
        probe.wait.assert_called_once()
        predicate = probe.wait.call_args.args[0]
        self.assertFalse(predicate(probe.observe.return_value))
        self.assertTrue(predicate(probe.wait.return_value))
        self.assertEqual(probe.wait.call_args.kwargs["timeout_seconds"], 15.0)

    def test_container_wait_expires_even_when_state_keeps_changing(self) -> None:
        clock = Mock(return_value=0.0)
        with patch.object(probe_steps.time, "monotonic", clock):
            probe = qa.ProbeSteps(Mock(), Path("unused"), 3600.0, "bounded-menu")
            probe.observe = Mock(return_value={"state_version": 1})

            def changed(*args):
                clock.return_value += 1.0
                return {}

            probe.call = Mock(side_effect=changed)
            with self.assertRaisesRegex(TimeoutError, "missing_menu"):
                probe.wait(lambda state: False, "missing_menu", timeout_seconds=15.0)
        self.assertFalse(probe.report["assertions"]["missing_menu"])
        self.assertEqual(probe.call.call_count, 15)
        self.assertTrue(all(0 < call.args[1]["timeout_seconds"] <= 15.0
                            for call in probe.call.call_args_list))

    def test_stock_falls_back_to_server_read_when_the_chest_menu_will_not_open(self) -> None:
        # Run 20261125T034259: a farm project in flight kept the founding chest shut
        # and the whole planner/storage snapshot failed. The same physical block is
        # readable from the server, so the snapshot must survive on that read.
        probe = Mock()
        probe.call.side_effect = lambda tool, arguments: (
            {"blocks": [{"id": "minecraft:chest", "x": 130, "y": 76, "z": -218,
                         "properties": {}}], "truncated": False}
            if tool == "minecraft_scan_blocks" else {}
        )
        probe.wait.side_effect = TimeoutError("founding_chest_opened")
        detail: dict = {}
        lines = ["#4294967300 storage level 1 [130, 76, -218]-[132, 79, -216]"]
        totals = qa._settlement_stock(
            probe, detail, (130, 76, -218), lines,
            lambda pos: {"minecraft:bread": 3},
        )
        self.assertEqual(totals, {"minecraft:bread": 3})
        self.assertIn("menu_fallback", detail["storage_observation"]["containers"][0])
        self.assertEqual(probe.wait.call_count, 3)

    def test_cut_status_dialog_is_accepted_only_as_a_proven_clipped_view(self) -> None:
        # Laptop 2026-11-18 22:28, pin 38c3f1cdf358: the rendered body ends inside the third project.
        road = [f"Road L1 · Prepared\n0/9\n115 68 {z}\nBuilder: b{z}" for z in (4, 7, 10, 13)]
        summary = "Active projects: {}/8\nPending orders: {}/8\nHead: {}"
        for active, pending, shown_projects in ((4, 1, 3), (3, 1, 3), (3, 2, 3)):
            with self.subTest(active=active, pending=pending):
                orders = [f"Pending #{i}: spanish_plaza L1 · busy" for i in range(1, pending + 1)]
                state = status_view(summary.format(active, pending, "spanish_plaza L1"), *road[:active], *orders)
                with self.assertRaisesRegex(RuntimeError, "count"):
                    qa._pending_dialog(state)
                seen = qa._pending_dialog(state, clipped_ok=True)
                self.assertEqual((seen["clipped"], seen["pending_count"], seen["head"], len(seen["projects"]),
                                  seen["orders"]), (True, pending, "spanish_plaza L1", shown_projects, []))
        # A visible prefix of the pending lines is kept, numbered from #1.
        orders = [f"Pending #{i}: spanish_house L1 · busy" for i in range(1, 5)]
        seen = qa._pending_dialog(status_view(summary.format(2, 4, "spanish_house L1"), *road[:2], *orders),
                                  clipped_ok=True)
        self.assertEqual(([o["index"] for o in seen["orders"]], seen["pending_count"]), ([1, 2, 3], 4))
        # Lines missing from a body that ends well above the action row are not a cut tail.
        with self.assertRaisesRegex(RuntimeError, "pending order list"):
            qa._pending_dialog(status_view(summary.format(2, 1, "spanish_plaza L1"), *road[:2]), clipped_ok=True)
        with self.assertRaisesRegex(RuntimeError, "active project list"):
            qa._pending_dialog(status_view(summary.format(3, 0, "none"), *road[:2]), clipped_ok=True)
        # A gap in the visible pending numbers is refused even when cut.
        with self.assertRaisesRegex(RuntimeError, "pending order list"):
            qa._pending_dialog(status_view(summary.format(2, 4, "spanish_house L1"), *road[:2],
                                           *orders[1:]), clipped_ok=True)

    def test_spanish_construction_dialog_passes_every_parser(self) -> None:
        # Laptop 2026-11-18 22:35, pin 38c3f1cdf358: culture blueprints have no display
        # name in settlements/ui.rs blueprint_label, so the dialog shows the raw id.
        projects = ("spanish_house L1 · Building\n3/40\n120 70 7", "spanish_church L1 · Prepared\n0/90\n140 70 7",
                    "Gate L1 · Building\n2/12\n100 70 7", "Road L1 · Prepared\n0/9\n115 68 4")
        orders = ("spanish_plaza L1 · waiting for forest evidence", "spanish_fortress L1 · busy")
        state = construction_screen(*projects, orders=orders)
        seen = qa._pending_dialog(state)
        self.assertEqual([(p["blueprint"], p["level"], p["origin"]) for p in seen["projects"]], [
            ("spanish_house", 1, [120, 70, 7]), ("spanish_church", 1, [140, 70, 7]),
            ("gate", 1, [100, 70, 7]), ("road", 1, [115, 68, 4])])
        self.assertEqual([(o["index"], o["blueprint"], o["reason"]) for o in seen["orders"]], [
            (1, "spanish_plaza", "waiting for forest evidence"), (2, "spanish_fortress", "busy")])
        self.assertFalse(qa._head_waits(seen))
        clipped = status_view(*(n["label"] for n in state["ui_nodes"]))
        cut = qa._pending_dialog(clipped, clipped_ok=True)
        self.assertEqual((cut["clipped"], cut["pending_count"], cut["head"], len(cut["projects"]), cut["orders"]),
                         (True, 2, "spanish_plaza L1", 3, []))
        sites = {"ui_nodes": [{"id": "dialog/body/0", "label":
                 "Active site: spanish_house L1 · 120 70 7\nActive site: spanish_church L1 · 140 70 7"}]}
        self.assertTrue(qa._map_has_sites(sites, seen["projects"][:2]))
        listed = qa._listed_buildings(["#7 spanish_plaza level 1 [110, 68, 0]-[124, 75, 14]"])
        self.assertEqual((listed[0]["kind"], listed[0]["level"]), ("spanish_plaza", 1))
        with self.assertRaisesRegex(RuntimeError, "'Spanish house'"):
            qa._project_progress(construction_screen("Spanish house L1 · Building\n1/9\n1 2 3"))

    def test_planner_diagnostic_reads_status_instead_of_stale_chat(self) -> None:
        observed = construction_screen("Mason's workshop L1 · Building\n5/100\n-12 70 24")
        observed["recent_chat"] = ["Buildings: stale list"]
        with patch.object(qa, "_project_status", return_value=observed) as project, patch.object(
            qa, "_send", return_value={"recent_chat": ["Buildings: current list"]},
        ), patch.object(qa, "_settlement_stock", return_value={"minecraft:coal": 4}):
            status = qa._planner_status(Mock(), {}, (1, 2, 3), Mock())
        project.assert_called_once()
        self.assertEqual(status["projects"], [{
            "blueprint": "mason", "level": 1, "phase": "Building", "done": 5,
            "total": 100, "origin": [-12, 70, 24], "builder": "builder-1",
        }])
        self.assertEqual(status["project_ui"], observed["ui_nodes"])

    def test_failed_second_player_check_still_closes_owned_clients(self) -> None:
        with TemporaryDirectory(prefix="autonomy-lifecycle-", dir=qa.runtime.REPO_ROOT / ".analysis") as folder:
            artifact = Path(folder)
            binary = artifact / "server"
            binary.write_bytes(b"unit-test binary identity")
            ctx = SimpleNamespace(
                client="native", extra_args=[], platform=None, windowed=True,
                timeout_seconds=10, artifact_dir=artifact, client_evidence={},
                commands=[], cwds=[], exits=[], logs=[], parts=1,
            )
            server = Mock(pid=101, returncode=0)
            primary = Mock(pid=102, returncode=0)
            peer = Mock(pid=103, returncode=0)
            primary_log, peer_log = Mock(), Mock()
            primary_mcp, peer_mcp = Mock(token=""), Mock(token="")
            primary_mcp.call_tool.return_value = {"in_play": True}

            def start_primary(*args, **kwargs):
                ctx.client_evidence["command_index"] = 0
                ctx.commands.append(["unit-test native client"])
                ctx.exits.append(None)
                return primary, primary_log, primary_mcp

            def start_server(*args, **kwargs):
                config = (artifact / "village-autonomy-qa.toml").read_text()
                self.assertIn("spawn_protection = 0", config,
                              "the peer operator must not lock the non-op out of the founding chest")
                return server

            def fail_after_peer(*args):
                ctx.exits.append(None)
                args[-1]((peer, peer_log, peer_mcp), 1)
                raise RuntimeError("second-player check failed")

            with (
                patch.dict(qa.os.environ, {"KRASNOKAMEN_SERVER_BINARY": str(binary)}),
                patch.object(qa.subprocess, "Popen", side_effect=start_server),
                patch.object(qa.runtime, "reserve_port", return_value=25566),
                patch.object(qa.runtime, "wait_port"),
                patch.object(qa.runtime, "start_xvfb", return_value=(Mock(), ":100", Mock())),
                patch.object(qa.runtime, "stop_process") as stop,
                patch.object(qa.native_client, "resolve_binary", return_value=Path("unit-test-client")),
                patch.object(qa.native_client, "start", side_effect=start_primary),
                patch.object(qa.native_client, "capture"),
                patch.object(qa, "_scenario", side_effect=fail_after_peer),
                self.assertRaisesRegex(RuntimeError, "second-player check failed"),
            ):
                qa.run(ctx)
            self.assertIn(peer, [call.args[0] for call in stop.call_args_list])
            self.assertIn(primary, [call.args[0] for call in stop.call_args_list])
            peer_mcp.close.assert_called_once()
            peer_log.close.assert_called_once()
            primary_log.close.assert_called_once()
            self.assertEqual(ctx.exits, [0, 0])


    def test_civilization_fixture_raises_only_the_simulation_distance(self) -> None:
        source = (qa.runtime.REPO_ROOT / "playable.toml").read_text()
        config = qa.server_config_text(source, port=25599, world_dir=".analysis/qa/world",
                                       plugins_dir=".analysis/qa/plugins", seed=24, civilization=True)
        self.assertIn("simulation_distance = 6", config)
        self.assertNotIn("simulation_distance = 4", config)
        self.assertIn("view_distance = 4", config)
        self.assertIn("seed = 24", config)
        plain = qa.server_config_text(source, port=25599, world_dir=".analysis/qa/world",
                                      plugins_dir=".analysis/qa/plugins", seed=100, civilization=False)
        self.assertIn("simulation_distance = 4", plain)
        self.assertIn("view_distance = 4", plain)

    def test_dialog_claim_is_confirmed_by_owner_not_by_chat_or_closed_screen(self) -> None:
        # The server answers a dialog Claim click by re-presenting the same
        # dialog (note inside the dialog body), so neither a "Claimed " chat
        # line nor a closed screen ever appears; ownership is the evidence.
        probe = Mock()
        probe.call.return_value = {"status": "ok"}
        owned = {"recent_chat": [
            "village_7 | owner=2f1c anchor=[0, 70, 0] radius=100 members=8 revision=2",
            "village_7 | owner=autonomous anchor=[0, 70, 0] radius=100 members=8 revision=1",
        ]}
        detail: dict = {}
        with patch.object(qa, "_send", return_value=owned) as send:
            claimed = qa._claim_via_dialog(probe, detail, "village_7", "Settlement village_7")
        self.assertTrue(claimed)
        send.assert_called_once_with(probe, "settlement info village_7")
        probe.call.assert_any_call("minecraft_click_screen_button", {
            "expected_screen_class": qa.DIALOG_CLASS,
            "expected_title": "Settlement village_7",
            "button_label": "Claim settlement",
        })
        # No unbounded state-change wait may follow the click.
        self.assertFalse(probe.wait.called)

    def test_project_status_is_read_from_the_status_dialog_behind_the_construction_page(self) -> None:
        # Projects menu (chest) -> Status slot -> Construction page (chest) -> clock slot -> status dialog.
        menu = {"screen": {"class": qa.CONTAINER_CLASS, "title": "Projects"}, "container": {"slot_count": 90}}
        page = {"screen": {"class": qa.CONTAINER_CLASS, "title": "Construction"}, "container": {"slot_count": 90}}
        status = construction_screen("Storage L1 · Building\n3/25\n1 70 2")
        status["screen"] = {"class": qa.DIALOG_CLASS, "title": "Status"}
        probe = Mock()
        probe.call.return_value = {"status": "ok"}
        probe.remaining.return_value = 60.0
        probe.wait.side_effect = [menu, page, status]
        with patch.object(qa, "_send", return_value={}) as send:
            observed = qa._project_status(probe)
        send.assert_called_once_with(probe, "settlement menu projects")
        slots = [call.args[1]["slot"] for call in probe.call.call_args_list
                 if call.args[0] == "minecraft_click_container_slot"]
        self.assertEqual(slots, [qa.PROJECTS_STATUS_SLOT, qa.CONSTRUCTION_STATUS_SLOT])
        progress = qa._project_progress(observed)[0]
        self.assertEqual(
            (progress["blueprint"], progress["level"], progress["done"], progress["total"], progress["origin"]),
            ("storage", 1, 3, 25, [1, 70, 2]),
        )
        probe.call.assert_any_call("minecraft_close_screen", {})


class UpgradeSpeciesObservationTests(unittest.TestCase):
    """Run 20261111T161835-nemkmfug: Storage L1 in birch, its L2 upgrade in acacia."""

    RECORD = {"id": 4294967304, "kind": "storage", "level": 2, "min": [121, 69, -2], "max": [129, 72, 2]}

    @staticmethod
    def upgraded_storage():
        """Client cells of the authored Storage L2: cells the upgrade left
        unchanged keep the L1 project's birch, cells it placed are acacia."""
        blueprint = next(b for b in qa._authored_blueprints() if b["id"] == "storage")
        lower = blueprint["levels"][0]["layers"]
        upper = blueprint["levels"][1]["layers"]
        cells = []
        for y, layer in enumerate(upper):
            for z, row in enumerate(layer):
                for x, symbol in enumerate(row):
                    if symbol == ".":
                        continue
                    kept = y < len(lower) and z < len(lower[y]) and x < len(lower[y][z]) and lower[y][z][x] == symbol
                    block, _, properties = blueprint["palette"][symbol].partition("[")
                    if block.startswith("oak_"):
                        block = ("birch" if kept else "acacia") + block.removeprefix("oak")
                    cells.append({
                        "x": 121 + x, "y": 69 + y, "z": -2 + z, "block_id": f"minecraft:{block}",
                        "properties": dict(part.split("=") for part in properties.rstrip("]").split(","))
                        if properties else {},
                    })
        return blueprint, cells

    def test_upgrade_keeps_the_earlier_level_species_in_cells_it_did_not_place(self):
        blueprint, cells = self.upgraded_storage()
        probe = Mock()
        probe.call.return_value = {"blocks": cells}
        detail = {}
        self.assertTrue(qa._observe_blueprint(probe, detail, dict(self.RECORD), blueprint, "upgraded"))
        result = detail["client_blocks"]["upgraded"]
        self.assertEqual((result["mismatch_count"], result["matching_rotations"]), (0, [0]))
        self.assertEqual(result["wood_species"], "L1 birch, L2 acacia")

    def test_each_level_project_still_needs_one_species(self):
        for kept, placed in (("birch", "acacia"), ("acacia", "birch")):
            with self.subTest(changed=kept):
                blueprint, cells = self.upgraded_storage()
                # The last cell of one level's project in the other project's wood.
                cell = [cell for cell in cells if cell["block_id"].startswith(f"minecraft:{kept}_")][-1]
                cell["block_id"] = cell["block_id"].replace(kept, placed)
                probe = Mock()
                probe.call.return_value = {"blocks": cells}
                detail = {}
                self.assertFalse(qa._observe_blueprint(probe, detail, dict(self.RECORD), blueprint, "mixed"))
                self.assertEqual(detail["client_blocks"]["mixed"]["mismatch_count"], 1)


class OwnerOrderScenarioTests(unittest.TestCase):
    def test_owner_projects_observes_current_status_dialog(self) -> None:
        probe, detail = Mock(), {}
        state = construction_screen(orders=("Farm L1 · busy",))
        with patch.object(qa, "_project_status", return_value=state) as status, \
             patch.object(qa.native_client, "capture"):
            observed = qa._owner_projects(probe, detail, "farm", Path("/receipt"))
        status.assert_called_once()
        self.assertEqual(observed["orders"][0]["blueprint"], "farm")
        self.assertEqual(detail["farm"], observed)


class SettlementStockObservationTests(unittest.TestCase):
    def test_sums_storage_without_navigating_or_opening_its_containers(self) -> None:
        probe = Mock()
        buildings = [
            "Buildings: #1 storage level 1 [10, 64, 0]-[13, 66, 3], "
            "#2 farmer level 1 [20, 64, 0]-[25, 68, 5].",
        ]
        probe.call.return_value = {"blocks": [
            {"x": 10, "y": 64, "z": 1, "block_id": "minecraft:chest"},
            {"x": 12, "y": 64, "z": 1, "block_id": "minecraft:barrel"},
            {"x": 99, "y": 64, "z": 1, "block_id": "minecraft:chest"},
        ]}
        contents = {
            (10, 64, 1): {"minecraft:cobblestone": 64},
            (12, 64, 1): {"minecraft:birch_log": 16},
        }
        reader = Mock(side_effect=contents.__getitem__)
        detail = {}
        with patch.object(qa, "_read", return_value={"block_id": "minecraft:chest"}), patch.object(
            qa, "_open_chest", return_value={0: {"item_id": "minecraft:cobblestone", "count": 2}},
        ) as opened:
            totals = qa._settlement_stock(probe, detail, (1, 64, 1), buildings, reader)
        self.assertEqual(totals, {"minecraft:cobblestone": 66, "minecraft:birch_log": 16})
        opened.assert_called_once_with(probe, (1, 64, 1))
        self.assertEqual({call.args[0] for call in reader.call_args_list}, set(contents))
        self.assertTrue(detail["storage_observation"]["complete"])
        self.assertEqual(len(detail["storage_observation"]["containers"]), 3)
        probe.call.assert_called_once_with("minecraft_scan_blocks", {
            "min_x": 10, "min_y": 64, "min_z": 0,
            "max_x": 13, "max_y": 66, "max_z": 3, "offset": 0,
        })

    def test_double_chest_and_overlapping_founding_position_are_counted_once(self) -> None:
        probe = Mock()
        probe.call.return_value = {"blocks": [
            {"x": 10, "y": 64, "z": 1, "block_id": "minecraft:chest",
             "properties": {"type": "left", "facing": "north"}},
            {"x": 11, "y": 64, "z": 1, "block_id": "minecraft:chest",
             "properties": {"type": "right", "facing": "north"}},
        ]}
        detail = {}
        reader = Mock()
        with patch.object(qa, "_open_chest", return_value={
            0: {"item_id": "minecraft:coal", "count": 4},
            27: {"item_id": "minecraft:birch_log", "count": 16},
        }) as opened:
            totals = qa._settlement_stock(probe, detail, (10, 64, 1), [
                "Buildings: #1 storage level 1 [10, 64, 0]-[12, 66, 3].",
                "Buildings: #1 storage level 1 [10, 64, 0]-[12, 66, 3].",
            ], reader)
        self.assertEqual(totals, {"minecraft:coal": 4, "minecraft:birch_log": 16})
        opened.assert_called_once_with(probe, (10, 64, 1))
        reader.assert_not_called()
        self.assertEqual(detail["storage_observation"]["containers"][0]["positions"],
                         [[10, 64, 1], [11, 64, 1]])

    def test_storage_read_failure_records_reason_without_partial_total(self) -> None:
        probe = Mock()
        probe.call.return_value = {"blocks": [
            {"x": 12, "y": 64, "z": 1, "block_id": "minecraft:barrel"},
        ]}
        detail = {}
        with patch.object(qa, "_read", return_value={"block_id": "minecraft:chest"}), patch.object(
            qa, "_open_chest", return_value={},
        ), self.assertRaisesRegex(TimeoutError, "storage read"):
            qa._settlement_stock(probe, detail, (1, 64, 1), [
                "Buildings: #1 storage level 1 [10, 64, 0]-[13, 66, 3].",
            ], Mock(side_effect=TimeoutError("storage read timed out")))
        observation = detail["storage_observation"]
        self.assertFalse(observation["complete"])
        self.assertNotIn("totals", observation)
        self.assertIn("storage read timed out", observation["containers"][-1]["error"])

    def test_planner_receipt_uses_the_aggregate_storage_observation(self) -> None:
        detail = {}
        probe, reader = Mock(), Mock()
        buildings = ["Buildings: #1 storage level 1 [10, 64, 0]-[13, 66, 3]."]
        with patch.object(qa, "_send", side_effect=[
            {"recent_chat": ["Projects"]}, {"recent_chat": buildings},
        ]), patch.object(qa, "_settlement_stock", return_value={
            "minecraft:cobblestone": 190, "minecraft:acacia_log": 37,
        }) as stock:
            status = qa._planner_status(probe, detail, (1, 64, 1), reader)
        stock.assert_called_once_with(probe, detail, (1, 64, 1), buildings, reader)
        self.assertEqual(status["storage_now"]["minecraft:cobblestone"], 190)


class ServerStorageReadTests(unittest.TestCase):
    def test_console_timeout_and_refusal_fail_closed_with_bounded_wait(self) -> None:
        with TemporaryDirectory() as folder:
            log = Path(folder) / "server.log"
            log.touch()
            server = SimpleNamespace(stdin=io.BytesIO())
            with patch.object(qa.runtime, "wait_for_path_condition",
                              side_effect=TimeoutError("storage read timed out")) as wait:
                with self.assertRaisesRegex(TimeoutError, "storage read"):
                    qa._read_storage_container(server, log, (12, 64, 1), 15.0)
            self.assertEqual(wait.call_args.args[1], 15.0)

            def refused(path, timeout, condition):
                log.write_text("That position is not loaded\n")
                reached, value = condition()
                self.assertTrue(reached)
                return value

            with patch.object(qa.runtime, "wait_for_path_condition", side_effect=refused):
                with self.assertRaisesRegex(RuntimeError, "That position is not loaded"):
                    qa._read_storage_container(server, log, (12, 64, 1), 15.0)


class IronSmeltCounterTests(unittest.TestCase):
    """CIV-B2 (1): the furnace's persistent RecipesUsed counter proves a smelt.

    Seed-24 gate 20261124T212153: the tower furnace showed
    RecipesUsed: {"minecraft:iron_ingot_from_smelting_raw_iron": 2} while every
    sample found the output slot empty and shears in storage, so the ingot was
    collected between samples and the slot-only evidence never saw it.
    """

    def test_a_furnace_recipe_counter_proves_a_smelt(self) -> None:
        line = ('132, 78, -222 has the following block data: {CookTime: 0, Items: [], '
                'RecipesUsed: {"minecraft:iron_ingot_from_smelting_raw_iron": 2}, '
                'id: "minecraft:furnace"}')
        counts = qa._recipe_counts(line)
        self.assertEqual(counts, {qa.IRON_SMELT_RECIPE: 2})
        self.assertTrue(qa.iron_smelted_by_recipes(counts))

    def test_an_empty_or_foreign_counter_is_no_evidence(self) -> None:
        for recipes in (None, {}, {qa.IRON_SMELT_RECIPE: 0},
                        {"minecraft:iron_ingot_from_blasting_raw_iron": 2}):
            with self.subTest(recipes=recipes):
                self.assertFalse(qa.iron_smelted_by_recipes(recipes))
        self.assertEqual(qa._recipe_counts(
            '1, 64, 1 has the following block data: {Items: [], id: "minecraft:furnace"}'), {})

    def test_recipe_read_reports_console_refusal_and_expiry(self) -> None:
        with TemporaryDirectory() as directory:
            log = Path(directory) / "server.log"
            log.write_text("")
            server = SimpleNamespace(stdin=io.BytesIO())
            with patch.object(qa.runtime, "wait_for_path_condition",
                              side_effect=TimeoutError("recipe read timed out")):
                with self.assertRaisesRegex(TimeoutError, "recipe read"):
                    qa.read_recipes(server, log, (12, 64, 1), 15.0)

            def refused(path, timeout, condition):
                log.write_text("The target block is not a block entity\n")
                reached, value = condition()
                self.assertTrue(reached)
                return value

            with patch.object(qa.runtime, "wait_for_path_condition", side_effect=refused):
                with self.assertRaisesRegex(RuntimeError, "not a block entity"):
                    qa.read_recipes(server, log, (12, 64, 1), 15.0)


if __name__ == "__main__":
    unittest.main()
