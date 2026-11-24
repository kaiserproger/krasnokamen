"""Offline regressions and unresolved contracts of the battle-crash-recovery gate."""
from __future__ import annotations

import json
import os
import struct
import tempfile
import unittest
import zlib
from pathlib import Path
from unittest import mock

from tools.harness import profiles
from tools.harness.backends import battle_recovery as br

SPEAR = {"id": "minecraft:iron_spear", "count": 1, "components": {"minecraft:item_model": "krasnokamen_battle:iron_spear"}}
SHIELD = {"id": "minecraft:shield", "count": 1, "components": {"minecraft:item_model": "krasnokamen_battle:red_heater_shield"}}
HOLDER = br.offline_uuid("PovRed")
DROPPED = "minecraft:overworld/aaaaaaaa-0000-0000-0000-000000000001"


def stack(holder, kind, item, place="", age=None):
    return br._stack_record(holder, kind, "minecraft:overworld", place, item, age)


def census(phase, tick, stacks, epoch=0, entities=None):
    found = {s["holder"].split(":", 1)[1]: {"type": "minecraft:item", "age": s["age"]}
             for s in stacks if s["kind"] == "item_entity"}
    found.update(entities or {})
    # The production-recovery export of the same image, by default identical (same list object).
    return {"phase": phase, "replay_tick": tick, "epoch": epoch, "stacks": stacks, "entities": found,
            "reopen": {"replay_tick": tick, "stacks": stacks}}


def history(first, last, spawns=(), removals=()):
    return {"spans": [(tick, tick) for tick in range(first, last + 1)], "spawns": list(spawns),
            "removals": list(removals), "motions": {}}


def batch(directory: Path, low: int, high: int, lines: list[dict], *, corrupt: bool = False) -> Path:
    """One published batch packed with the *writer's* struct (`journal.rs::frame_batch`)."""
    from tools.harness.backends.wal_forge import BATCH_MAGIC, BATCH_VERSION, HEADER

    body = b"".join(json.dumps(line).encode() + b"\n" for line in lines)
    crc = zlib.crc32(body) ^ (1 if corrupt else 0)
    path = directory / f"batch-{low:020d}-{high:020d}.bin"
    header = HEADER.pack(BATCH_MAGIC, BATCH_VERSION, bytes(range(16)), bytes(range(8)),
                         low, high, len(body), crc)
    path.write_bytes(header + body)
    return path


class CleanupFailureTests(unittest.TestCase):
    def test_primary_traceback_survives_broken_control_pipe_and_cleanup_continues(self):
        with tempfile.TemporaryDirectory() as directory:
            session = object.__new__(br.RunnerSession)
            session.ctx = mock.Mock(artifact_dir=Path(directory), exits=[None, None])
            session.runner = mock.Mock(pid=11, returncode=0, poll=mock.Mock(return_value=0))
            session.recovery = mock.Mock(pid=12, returncode=-15, poll=mock.Mock(return_value=-15))
            session.runner_index, session.recovery_index = 0, 1
            session.archive = None
            session.driver_log = None
            session.battle = mock.Mock()
            dead_ctl = mock.Mock(pid=13, poll=mock.Mock(return_value=-9))
            client = mock.Mock(name="client")
            client.name = "PovRed"
            client.process = mock.Mock(pid=14, poll=mock.Mock(return_value=-9))
            client.driver.proc = dead_ctl
            client.stop.side_effect = BrokenPipeError("control stdin")
            session.clients = [client]

            def failing_gate():
                try:
                    raise ValueError("original gate failure")
                finally:
                    session.close()

            with self.assertRaisesRegex(ValueError, "original gate failure"):
                failing_gate()
            diagnostic = json.loads((Path(directory) / "battle-failure-diagnostics.json").read_text())
            self.assertIn("ValueError: original gate failure", diagnostic["primary_traceback"])
            self.assertIn("BrokenPipeError: control stdin", diagnostic["cleanup_errors"][0]["traceback"])
            self.assertEqual(diagnostic["before_cleanup"]["control:PovRed"]["exit"], -9)
            self.assertEqual(diagnostic["after_cleanup"]["recovery_server"]["exit"], -15)
            self.assertEqual(session.battle.stop_child.call_count, 2)

    def test_teardown_failure_without_primary_still_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            session = object.__new__(br.RunnerSession)
            session.ctx = mock.Mock(artifact_dir=Path(directory))
            session.runner = session.recovery = session.archive = session.driver_log = None
            session.runner_index = session.recovery_index = -1
            session.battle = mock.Mock()
            client = mock.Mock()
            client.name, client.process, client.driver = "PovRed", None, None
            client.stop.side_effect = BrokenPipeError("control stdin")
            session.clients = [client]
            with self.assertRaises(BrokenPipeError):
                session.close()
            diagnostic = json.loads((Path(directory) / "battle-failure-diagnostics.json").read_text())
            self.assertIsNone(diagnostic["primary_traceback"])
            self.assertIn("BrokenPipeError", diagnostic["cleanup_errors"][0]["traceback"])


class ExportReadingTests(unittest.TestCase):
    def test_nbt_uuid_matches_vanilla_int_array(self):
        self.assertEqual(br.nbt_uuid([0, 0, 0, 1]), "00000000-0000-0000-0000-000000000001")
        self.assertEqual(br.nbt_uuid([-1, -1, -1, -1]), "ffffffff-ffff-ffff-ffff-ffffffffffff")


class ReconcileTests(unittest.TestCase):
    def setUp(self):
        self.issued = census("issued", 100, [stack(f"player:{HOLDER}", "player", SPEAR, "/Inventory/0"),
                                             stack(f"player:{HOLDER}", "player", SHIELD, "/equipment/offhand")])
        self.kit = br.issued_kit(self.issued, HOLDER)
        self.dropped = census("dropped", 110, [stack(f"player:{HOLDER}", "player", SHIELD),
                                               stack(f"entity:{DROPPED}", "item_entity", SPEAR, "/Item", age=8)])
        self.spawn = {"dimension": "minecraft:overworld", "id": 42, "uuid": DROPPED.split("/")[1], "tick": 102, "age": 0}
        self.history = history(100, 200, spawns=[self.spawn])

    def test_rearm_with_the_dropped_piece_on_the_ground_fails(self):
        rearmed = census("crash-rejoin", 130, self.dropped["stacks"] + [stack(f"player:{HOLDER}", "player", SPEAR)])
        result = br.reconcile([self.issued, self.dropped, rearmed], self.kit, set(), [self.history])
        self.assertEqual(result["verdict"], "fail")
        self.assertEqual(result["violations"][0]["units"], 2)

    def test_common_restore_loss_in_both_exports_still_fails_issued_kit_reconciliation(self):
        dropped = census("dropped", 110, [stack(f"player:{HOLDER}", "player", SHIELD)])
        crash = census("crash", 120, dropped["stacks"])
        result = br.reconcile([self.issued, dropped, crash], self.kit, set(), [self.history])
        self.assertTrue(all(phase["reopen"]["agree"] for phase in result["phases"]))
        self.assertEqual(result["verdict"], "incomplete")
        self.assertTrue(any(row.get("key") == br.kit_key(SPEAR)
                            and "1 unit(s) left custody with 0 ordered removal(s)" in row["reason"]
                            for row in result["incomplete"]), result)

    def test_removal_control_on_a_derived_copy_is_detected(self):
        kept = census("crash", 120, self.dropped["stacks"])
        phases = [self.issued, self.dropped, kept]
        control = br.removal_control(phases, 2, self.kit, set(), [self.history])
        self.assertTrue(control["detected"], control)
        self.assertEqual(control["removed"]["kind"], "item_entity")
        self.assertEqual(len(kept["stacks"]), 2, "the census itself is not altered")
        # A census holding only inventory stacks has no entity whose loss could be detected.
        with self.assertRaises(br.Incomplete):
            br.removal_control([self.issued, census("crash", 120, self.issued["stacks"])], 1, self.kit, set(),
                               [self.history])


class InputsAndOutputsTests(unittest.TestCase):
    def test_inputs_fail_closed_and_are_hashed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pin = root / "pin"
            pin.mkdir()
            (pin / "manifest.json").write_text("{}")
            server = pin / "krasnokamen"
            for executable in (server, root / "client", root / "ctl", root / "recorder"):
                executable.write_bytes(b"#!/bin/true\n")
                executable.chmod(0o755)
            for name in ("resources", "data", "world/native"):
                (root / name).mkdir(parents=True)
            (root / "world/native/state.redb").write_bytes(b"x")
            (root / "data/vanilla").mkdir()
            self.enterContext(mock.patch.object(br, "REPO_ROOT", root))
            env = {"KRASNOKAMEN_SERVER_BINARY": str(server), "KRASNOKAMEN_BATTLE_WORLD": str(root / "world"),
                   "KRASNOKAMEN_BATTLE_WORLD_SEED": "935", "KRASNOKAMEN_BATTLE_HORIZON_CLIENT": str(root / "client"),
                   "KRASNOKAMEN_BATTLE_HORIZON_CTL": str(root / "ctl"),
                   "KRASNOKAMEN_BATTLE_HORIZON_RESOURCES": str(root / "resources"),
                   "KRASNOKAMEN_CLIENT_BINARY": str(root / "recorder"),
                   "KRASNOKAMEN_BATTLE_RECORDER_DATA": str(root / "data")}
            ctx = profiles.ProfileContext(root)
            inputs = br.resolve_inputs(ctx, "battle-kit-census", env)
            self.assertEqual(inputs["server"], server.resolve())
            self.assertEqual(ctx.client_evidence["battle_inputs"]["KRASNOKAMEN_BATTLE_WORLD_SEED"], 935)
            self.assertIn("sha256", ctx.client_evidence["battle_inputs"]["KRASNOKAMEN_BATTLE_HORIZON_CLIENT"])
            for missing in ("KRASNOKAMEN_SERVER_BINARY", "KRASNOKAMEN_BATTLE_HORIZON_CTL", "KRASNOKAMEN_CLIENT_BINARY"):
                with self.subTest(missing=missing), self.assertRaises(ValueError):
                    br.resolve_inputs(profiles.ProfileContext(root), "battle-kit-census",
                                      {k: v for k, v in env.items() if k != missing})
            auth_env = {k: v for k, v in env.items() if k not in br.RUNNER_INPUTS.values()}
            with mock.patch.object(br.runtime, "sibling_root", return_value=root / "plugins"):
                with self.assertRaisesRegex(ValueError, "krasnokamen-auth"):
                    br.resolve_inputs(profiles.ProfileContext(root), "battle-auth-hydration", auth_env)
                (root / "plugins/krasnokamen-auth").mkdir(parents=True)
                for name in ("plugin.toml", "plugin.wasm"):
                    (root / "plugins/krasnokamen-auth" / name).write_bytes(b"p")
                self.assertIn("auth_package", br.resolve_inputs(profiles.ProfileContext(root), "battle-auth-hydration", auth_env))
                (pin / "manifest.json").write_text('{"incremental": true}')
                with self.assertRaisesRegex(ValueError, "--no-incremental"):
                    br.resolve_inputs(profiles.ProfileContext(root), "battle-crash-handoff", env)
                self.assertIn("auth_package", br.resolve_inputs(profiles.ProfileContext(root), "battle-auth-hydration",
                                                                auth_env), "the auth scenario runs no battle runner")
            os.unlink(pin / "manifest.json")
            with self.assertRaisesRegex(ValueError, "manifest"):
                br.resolve_inputs(profiles.ProfileContext(root), "battle-kit-census", env)


class RearmScenarioTests(unittest.TestCase):
    def test_windup_observer_selects_only_fresh_horse_extra(self):
        image = {"battle": {"elapsed_ticks": 100,
                            "formation": {"members": [{"id": 1}, {"id": 2}]},
                            "combat": {"ids": [1, 2, 3], "fighters": [
                                {"action": {"Windup": {"target": 3, "start": 98, "weapon": "Sword"}}},
                                {"action": {"Windup": {"target": 2, "start": 99, "weapon": "Sword"}}},
                                {"action": "Wounded"}]}}}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            batch(root, 1, 1, [{"Entity": {"id": 3, "uuid": "horse-id", "type_id": 67}},
                               {"Battle": {"value": json.dumps(image)}}])
            observer = br.WindupObserver(root, 67)
            observer.update()
            target = br.windup_horse(observer.image, observer.horses)
            self.assertEqual((target["actor"], target["target"], target["uuid"], target["impact"]),
                             (1, 3, "horse-id", 108))
            image["battle"]["elapsed_ticks"] = 105
            self.assertIsNone(br.windup_horse({"tick": 2, "image": image}, observer.horses),
                              "three ticks before impact is too late for the kill")
            image["battle"]["elapsed_ticks"] = 100
            image["battle"]["formation"]["members"].append({"id": 3})
            self.assertIsNone(br.windup_horse({"tick": 2, "image": image}, observer.horses),
                              "a roster member is not a horse extra")

    @staticmethod
    def full_image(elapsed, ids, fighters, live=None):
        return {"version": 1,
                "battle": {"running": True, "elapsed_ticks": elapsed,
                           "formation": {"members": [{"id": 1}, {"id": 2}], "regiments": [], "tick": elapsed},
                           "combat": {"ids": list(ids), "fighters": list(fighters)},
                           "live": live or {"holds": [], "spectators": []}},
                "columns": [[actor, None, None] for actor in ids], "removed": []}

    @staticmethod
    def header(image):
        """The image without its six lists, as core journals a changed header."""
        header = json.loads(json.dumps(image))
        for name in br.BATTLE_LISTS:
            parent, leaf = br._battle_list(header, name)
            parent[leaf] = []
        return header

    def test_a_format_one_archive_fails_loudly_instead_of_stopping_silently(self):
        # A refused frame must name itself: the silent `break` burned a 600 s `published` deadline
        # in the A11 rerun with no diagnosis (review wave2-8 m3). A real v1 frame is 32 bytes; a
        # v1 frame padded past the header must fail on its version, not on its length (m4).
        for size in (32, 56):
            with self.subTest(bytes=size), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                path = root / f"batch-{1:020d}-{1:020d}.bin"
                frame = struct.pack("<IIQQII", 0x534F4C4A, 1, 1, 1, 0, 0)
                path.write_bytes(frame + bytes(max(0, size - len(frame))))
                observer = br.WindupObserver(root, 67)
                with self.assertRaisesRegex(br.Incomplete, r"not a format-2 frame.*version 1"):
                    observer.update()
                self.assertEqual(observer.last_tick, -1, "a refused frame is never folded as published")

    def test_windup_observer_folds_battle_patches_after_the_full_image(self):
        guard = {"action": {"Guard": {"until": 0}}}
        full = self.full_image(100, [1, 2], [guard, guard])
        changed = self.full_image(101, [1, 2, 3], [guard, guard, guard])
        patch = {"header": self.header(changed), "lists": [
            {"list": "ids", "len": 3, "set": [[2, 3]]},
            {"list": "fighters", "len": 3, "set": [
                [0, {"action": {"Windup": {"target": 3, "start": 99, "weapon": "Sword"}}}],
                [2, {"action": "Wounded"}]]},
            {"list": "columns", "len": 3, "set": [[2, [3, None, None]]]}]}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            batch(root, 1, 1, [{"Entity": {"id": 3, "uuid": "horse-id", "type_id": 67}},
                               {"Dimension": {"dimension": "minecraft:overworld",
                                              "entries": [{"Battle": {"value": json.dumps(full)}}]}}])
            batch(root, 2, 2, [{"Dimension": {"dimension": "minecraft:overworld",
                                              "entries": [{"BattlePatch": {"value": patch}}]}}])
            observer = br.WindupObserver(root, 67)
            observer.update()
            self.assertEqual(observer.image["tick"], 2)
            self.assertEqual(observer.image["image"]["battle"]["elapsed_ticks"], 101)
            target = br.windup_horse(observer.image, observer.horses)
            self.assertEqual((target["actor"], target["target"], target["impact"], target["wal_tick"]),
                             (1, 3, 109, 2), "the windup arrives only in the patch")
            history = {(1, 1): root / f"batch-{1:020d}-{1:020d}.bin", (2, 2): root / f"batch-{2:020d}-{2:020d}.bin"}
            self.assertEqual(br.battle_image(history, 1)["image"], full)
            folded = br.battle_image(history, 2)
            self.assertEqual(folded["tick"], 2)
            self.assertEqual(folded["image"]["battle"]["combat"]["ids"], [1, 2, 3])
            self.assertEqual(folded["image"]["columns"], changed["columns"])

    def test_battle_image_reads_header_changes_at_the_locked_tick(self):
        full = self.full_image(100, [1], [{"action": "Wounded"}])
        held = self.full_image(101, [1], [{"action": "Wounded"}],
                               live={"holds": [{"name": "PovRed"}], "spectators": [["CamSpec", "PovRed"]]})
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            history = {(1, 1): batch(root, 1, 1, [{"Battle": {"value": json.dumps(full)}}]),
                       (2, 2): batch(root, 2, 2, [{"BattlePatch": {"value": {"header": self.header(held)}}}]),
                       (3, 3): batch(root, 3, 3, [{"Player": {"uuid": "x"}}])}
            image = br.battle_image(history, 3)
            self.assertEqual(image["tick"], 2)
            self.assertEqual(image["image"]["battle"]["live"], held["battle"]["live"])
            self.assertEqual(image["image"]["battle"]["combat"]["fighters"], [{"action": "Wounded"}])

    def test_battle_patch_refusals_mirror_the_core(self):
        full = self.full_image(100, [1, 2], [{"action": "Wounded"}, {"action": "Wounded"}])
        for base, patch, expected in (
                (None, {}, "without a full image"),
                (full, {"lists": [], "extra": 1}, "malformed battle patch"),
                (full, {"lists": [{"list": "horses", "len": 0, "set": []}]}, "malformed battle patch list"),
                (full, {"lists": [{"list": "ids", "len": 1, "set": [[5, 7]]}]}, "index 5 past length 1"),
                (full, {"lists": [{"list": "ids", "len": 3, "set": []}]}, "left a ids element unset"),
                (full, {"lists": [{"list": "members", "len": 1_000_000, "set": []}]}, "exceeds capacity"),
                (full, {"header": {"battle": {}}}, "battle image lacks members")):
            with self.subTest(expected=expected), self.assertRaisesRegex(br.Incomplete, expected):
                br.apply_battle_patch(json.loads(json.dumps(base)) if base else None, patch)
        shrunk = br.apply_battle_patch(json.loads(json.dumps(full)),
                                       {"lists": [{"list": "ids", "len": 1, "set": []}]})
        self.assertEqual(shrunk["battle"]["combat"]["ids"], [1], "a shorter length drops the tail")

    def test_rearm_scenario_runs_two_acknowledged_commands_before_census(self):
        fifo = mock.Mock()
        fifo.mark.side_effect = [11, 22]
        fifo.wait.return_value.group.return_value = "Battle live rearmed: PovRed side=1"
        session = mock.Mock()
        session.clients = [session.holder, session.viewer]
        backups = []
        report = {"scenario": "battle-rearm-recovery", "assertions": {}}
        with mock.patch.object(br, "_phase_census", return_value={"phase": "precut-repeated-rearm"}) as census:
            br._rearm_twice(fifo, session, ["minecraft:iron_spear"], {}, backups, report)
        self.assertEqual(fifo.send.call_args_list,
                         [mock.call("battle live PovRed red"), mock.call("battle live PovRed red")])
        self.assertEqual([call.args[1] for call in fifo.wait.call_args_list], [11, 22])
        self.assertEqual(backups, [{"phase": "precut-repeated-rearm"}])
        census.assert_called_once()
        session.holder.capture.assert_called_once_with("precut-repeated-rearm")
        self.assertTrue(report["assertions"]["rearmed_twice_before_crash"])
        self.assertIs(br.VERDICTS["battle-rearm-recovery"], br.kit_census)


class ClientKitTests(unittest.TestCase):
    ITEMS = ["minecraft:iron_helmet", "minecraft:iron_spear", "minecraft:shield"]

    def test_a_drop_moves_exactly_one_kit_piece(self):
        issued = {item: 1 for item in self.ITEMS}
        dropped, held = br.kit_after_drop(self.ITEMS, issued, {"minecraft:iron_helmet": 1, "minecraft:shield": 1})
        self.assertEqual((dropped, held), ("minecraft:iron_spear",
                                           {"minecraft:iron_helmet": 1, "minecraft:iron_spear": 0, "minecraft:shield": 1}))
        for after in (issued, {"minecraft:shield": 1}, {**issued, "minecraft:iron_spear": 2}):
            with self.subTest(after=after), self.assertRaises(RuntimeError):
                br.kit_after_drop(self.ITEMS, issued, after)

    def test_roles_after_login_hold_the_kit_minus_the_dropped_piece(self):
        held = {"minecraft:iron_helmet": 1, "minecraft:iron_spear": 0, "minecraft:shield": 1}
        viewer = {"state": {"screen": "playing", "gamemode": "spectator", "camera": {"entity_id": 7}}}

        def holder(counts):
            return {"state": {"screen": "playing", "gamemode": "survival", "player": {"id": 7}},
                    "inventory": {"counts": counts}}

        report = {"scenario": "battle-auth-hydration", "assertions": {}}
        br.roles(report, "after", holder({"minecraft:iron_helmet": 1, "minecraft:shield": 1}), viewer, 7, kit=held)
        self.assertEqual(report["observations"]["after"]["expected_kit"], held)
        for counts in ({"minecraft:iron_helmet": 1, "minecraft:iron_spear": 1, "minecraft:shield": 1},
                       {"minecraft:iron_helmet": 1}):
            with self.subTest(counts=counts), self.assertRaises(AssertionError):
                br.roles(report, "after", holder(counts), viewer, 7, kit=held)

    def test_a_missing_auth_acknowledgement_reports_the_chat_without_the_password(self):
        class Client:
            name = "PovRed"

            def __init__(self):
                self.calls = []

            def last_event(self):
                return 3

            def driver(self, tool, **args):
                self.calls.append(tool)
                if tool == "wait_for":
                    raise RuntimeError("horizon-ctl wait_for failed: timeout")
                if tool == "messages":
                    return {"chat": ["Matching passwords required", "echo s3cret-pw"]}
                return {}

        client = Client()
        with self.assertRaises(RuntimeError) as caught:
            br._authenticate(client, "s3cret-pw", register=True)
        self.assertIn("Matching passwords required", str(caught.exception))
        self.assertIn("/register not acknowledged", str(caught.exception))
        self.assertNotIn("s3cret-pw", str(caught.exception))
        self.assertEqual(client.calls, ["command", "wait_for", "messages"])

    def test_tool_timeouts_reach_horizon_ctl(self):
        sent = []
        ctl = object.__new__(br.HorizonCtl)
        ctl.tools = {"wait_for", "wait_ready", "events"}
        ctl.log = mock.Mock()
        ctl._rpc = lambda method, params, timeout: sent.append((params["arguments"], timeout)) or {"content": []}
        ctl("wait_for", condition="chat", value="x", timeout=60)
        ctl("wait_ready", seconds=400)
        ctl("events")
        self.assertEqual(sent, [({"condition": "chat", "value": "x", "timeout": 60}, 120.0),
                                ({"seconds": 400}, 460.0), ({}, 120.0)])


if __name__ == "__main__":
    unittest.main()
