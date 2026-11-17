"""Offline parts of the battle-crash-recovery gate: export reading, WAL history and the census verdict."""
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
    body = b"".join(json.dumps(line).encode() + b"\n" for line in lines)
    crc = zlib.crc32(body) ^ (1 if corrupt else 0)
    path = directory / f"batch-{low:020d}-{high:020d}.bin"
    path.write_bytes(struct.pack("<IIQQII", 0x534F4C4A, 1, low, high, len(body), crc) + body)
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
    def test_every_player_custody_place_and_nested_container_is_a_stack(self):
        damaged = {**SPEAR, "components": {**SPEAR["components"], "minecraft:damage": 17}}
        shulker = {"id": "minecraft:shulker_box", "count": 1,
                   "components": {"minecraft:container": [{"slot": 0, "item": SHIELD}]}}
        player = {"Inventory": [{**damaged, "Slot": 0}, {**shulker, "Slot": 9}],
                  "equipment": {"offhand": SHIELD, "head": {"id": "minecraft:iron_helmet"}},
                  "krasnokamen:Carried": {"id": "minecraft:iron_spear"},
                  "krasnokamen:CraftingGrid": [{}, SPEAR, {}, {}],
                  "EnderItems": [{**SPEAR, "Slot": 3}], "Pos": [0.0, 1.0, 2.0], "id": "not-an-item"}
        found = {(path, item["id"]) for path, item in br.stacks_in(player)}
        self.assertEqual(found, {
            ("/Inventory/0", "minecraft:iron_spear"), ("/Inventory/9", "minecraft:shulker_box"),
            ("/Inventory/9/components/minecraft:container/0/item", "minecraft:shield"),
            ("/equipment/offhand", "minecraft:shield"), ("/equipment/head", "minecraft:iron_helmet"),
            ("/krasnokamen:Carried", "minecraft:iron_spear"), ("/krasnokamen:CraftingGrid/1", "minecraft:iron_spear"),
            ("/EnderItems/3", "minecraft:iron_spear")})
        record = br._stack_record("player:x", "player", None, "/Inventory/0", damaged)
        self.assertEqual(record["damage"], 17)
        self.assertEqual(record["key"], br.kit_key(SPEAR), "wear is not identity")
        self.assertNotEqual(br.kit_key({"id": "minecraft:iron_spear", "count": 1}), br.kit_key(SPEAR))

    def test_nbt_uuid_matches_vanilla_int_array(self):
        self.assertEqual(br.nbt_uuid([0, 0, 0, 1]), "00000000-0000-0000-0000-000000000001")
        self.assertEqual(br.nbt_uuid([-1, -1, -1, -1]), "ffffffff-ffff-ffff-ffff-ffffffffffff")


class WalHistoryTests(unittest.TestCase):
    def test_complete_batches_only_and_conflicts_refuse(self):
        with tempfile.TemporaryDirectory() as directory:
            first, second = Path(directory, "a"), Path(directory, "b")
            first.mkdir()
            second.mkdir()
            spawn = {"Entity": {"id": 7, "uuid": "u-1", "tick": 10, "item": {"item_id": 1}, "alive": True, "age": 0}}
            wrapped = {"Dimension": {"dimension": "minecraft:the_nether", "entries": [{"RemoveEntity": {"id": 9, "tick": 11}}]}}
            batch(first, 10, 10, [spawn, {"EntityMotion": {"id": 1}}])
            batch(first, 11, 11, [wrapped, {"ItemMotion": {"id": 7, "tick": 11, "age": 5}}])
            batch(first, 12, 12, [{"RemoveEntity": {"id": 7, "tick": 12}}], corrupt=True)
            batch(second, 10, 10, [spawn, {"EntityMotion": {"id": 1}}])
            batches = br.read_batches([first, second])
            self.assertEqual(sorted(batches), [(10, 10), (11, 11)])
            result = br.wal_history(batches)
            self.assertEqual(result["spawns"][0]["uuid"], "u-1")
            self.assertEqual(result["removals"], [{"dimension": "minecraft:the_nether", "id": 9, "tick": 11}])
            self.assertEqual(result["motions"], {"minecraft:overworld/7": [(11, 5)]})
            batch(second, 11, 11, [{"RemoveEntity": {"id": 8, "tick": 11}}])
            with self.assertRaises(br.Incomplete):
                br.read_batches([first, second])

    def test_coverage_gaps(self):
        self.assertEqual(br.coverage_gaps([(1, 5), (6, 6), (9, 12)], 0, 12), [[7, 8]])
        self.assertEqual(br.coverage_gaps([(1, 5)], 2, 4), [])
        self.assertEqual(br.coverage_gaps([(1, 5)], 2, 7), [[6, 7]])


class ReconcileTests(unittest.TestCase):
    def setUp(self):
        self.issued = census("issued", 100, [stack(f"player:{HOLDER}", "player", SPEAR, "/Inventory/0"),
                                             stack(f"player:{HOLDER}", "player", SHIELD, "/equipment/offhand")])
        self.kit = br.issued_kit(self.issued, HOLDER)
        self.dropped = census("dropped", 110, [stack(f"player:{HOLDER}", "player", SHIELD),
                                               stack(f"entity:{DROPPED}", "item_entity", SPEAR, "/Item", age=8)])
        self.spawn = {"dimension": "minecraft:overworld", "id": 42, "uuid": DROPPED.split("/")[1], "tick": 102, "age": 0}
        self.history = history(100, 200, spawns=[self.spawn])

    def test_kept_drop_passes_and_soldier_equipment_is_not_custody(self):
        soldier = "minecraft:overworld/bbbbbbbb-0000-0000-0000-000000000002"
        later = census("crash", 120, self.dropped["stacks"] + [stack(f"entity:{soldier}", "entity", SPEAR)])
        later["stacks"][1] = {**later["stacks"][1], "age": 18}
        result = br.reconcile([self.issued, self.dropped, later], self.kit, {soldier.split("/")[1]}, [self.history])
        self.assertEqual(result["verdict"], "pass", result)
        self.assertEqual(result["phases"][-1]["soldier_equipment"][br.kit_key(SPEAR)], 1)

    def test_rearm_with_the_dropped_piece_on_the_ground_fails(self):
        rearmed = census("crash-rejoin", 130, self.dropped["stacks"] + [stack(f"player:{HOLDER}", "player", SPEAR)])
        result = br.reconcile([self.issued, self.dropped, rearmed], self.kit, set(), [self.history])
        self.assertEqual(result["verdict"], "fail")
        self.assertEqual(result["violations"][0]["units"], 2)

    def test_lost_piece_needs_an_ordered_durable_removal(self):
        gone = census("death", 130, [stack(f"player:{HOLDER}", "player", SHIELD)])
        unexplained = br.reconcile([self.issued, self.dropped, gone], self.kit, set(), [self.history])
        self.assertEqual(unexplained["verdict"], "incomplete")
        removed = history(100, 200, spawns=[self.spawn], removals=[{"dimension": "minecraft:overworld", "id": 42, "tick": 125}])
        explained = br.reconcile([self.issued, self.dropped, gone], self.kit, set(), [removed])
        self.assertEqual(explained["verdict"], "pass", explained)
        self.assertEqual(explained["removals"][0]["tick"], 125)
        early = history(100, 200, spawns=[self.spawn], removals=[{"dimension": "minecraft:overworld", "id": 42, "tick": 105}])
        self.assertEqual(br.reconcile([self.issued, self.dropped, gone], self.kit, set(), [early])["verdict"], "incomplete")

    def test_common_restore_loss_in_both_exports_still_fails_issued_kit_reconciliation(self):
        dropped = census("dropped", 110, [stack(f"player:{HOLDER}", "player", SHIELD)])
        crash = census("crash", 120, dropped["stacks"])
        result = br.reconcile([self.issued, dropped, crash], self.kit, set(), [self.history])
        self.assertTrue(all(phase["reopen"]["agree"] for phase in result["phases"]))
        self.assertEqual(result["verdict"], "incomplete")
        self.assertTrue(any(row.get("key") == br.kit_key(SPEAR)
                            and "1 unit(s) left custody with 0 ordered removal(s)" in row["reason"]
                            for row in result["incomplete"]), result)

    def test_identity_reuse_gaps_and_clocks_are_incomplete(self):
        gone = census("death", 130, [stack(f"player:{HOLDER}", "player", SHIELD)])
        reused = history(100, 200, spawns=[self.spawn, {**self.spawn, "uuid": "other", "tick": 120}],
                         removals=[{"dimension": "minecraft:overworld", "id": 42, "tick": 125}])
        self.assertEqual(br.reconcile([self.issued, self.dropped, gone], self.kit, set(), [reused])["verdict"], "incomplete")
        gap = {**self.history, "spans": [s for s in self.history["spans"] if s != (115, 115)]}
        kept = census("crash", 120, self.dropped["stacks"])
        self.assertIn("gaps", json.dumps(br.reconcile([self.issued, self.dropped, kept], self.kit, set(), [gap])["incomplete"]))
        younger = census("crash", 120, [self.dropped["stacks"][0], {**self.dropped["stacks"][1], "age": 2}])
        self.assertEqual(br.reconcile([self.issued, self.dropped, younger], self.kit, set(), [self.history])["verdict"], "incomplete")
        backwards = census("crash", 105, self.dropped["stacks"])
        self.assertEqual(br.reconcile([self.issued, self.dropped, backwards], self.kit, set(), [self.history])["verdict"], "incomplete")
        retyped = census("crash", 120, [self.dropped["stacks"][0]], entities={DROPPED: {"type": "minecraft:arrow", "age": None}})
        self.assertIn("identity reuse", json.dumps(br.reconcile([self.issued, self.dropped, retyped], self.kit, set(),
                                                                [self.history])["incomplete"]))

    def test_new_epoch_reads_its_own_history_from_the_recovered_tick(self):
        kept = census("crash", 120, self.dropped["stacks"])
        rejoin = census("crash-rejoin", 140, self.dropped["stacks"], epoch=1)
        rejoin["epoch_start"] = 120
        recovered = history(121, 140)
        self.assertEqual(br.reconcile([self.issued, self.dropped, kept, rejoin], self.kit, set(),
                                      [self.history, recovered])["verdict"], "pass")
        late = history(125, 140)
        self.assertEqual(br.reconcile([self.issued, self.dropped, kept, rejoin], self.kit, set(),
                                      [self.history, late])["verdict"], "incomplete")

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

    def test_offline_and_production_recovery_exports_must_agree_on_every_kit_item(self):
        soldier = "minecraft:overworld/bbbbbbbb-0000-0000-0000-000000000002"
        player, ground = self.dropped["stacks"]

        def verdict(reopened, tick=130, offline=None):
            kept = census("crash", 120, offline or [player, ground])
            kept["reopen"] = {"replay_tick": tick, "stacks": reopened}
            return br.reconcile([self.issued, self.dropped, kept], self.kit, {soldier.split("/")[1]}, [self.history])

        aged = {**ground, "age": ground["age"] + 10}
        agreed = verdict([player, aged, stack(f"entity:{soldier}", "entity", SPEAR)])
        self.assertEqual(agreed["verdict"], "pass", agreed)
        self.assertEqual(agreed["phases"][-1]["reopen"]["bound_ticks"], 10)
        # The image carries 10 ticks of unrepaid simulation debt (age 8 at tick 120, born at 102); the
        # reopened server may repay it, up to the birth bound 130 - 102 = 28.
        self.assertEqual(verdict([player, {**ground, "age": 28}])["verdict"], "pass")
        damaged = {**SHIELD, "components": {**SHIELD["components"], "minecraft:damage": 3}}
        for label, reopened in (
                ("older than its birth allows", [player, {**ground, "age": 29}]),
                ("younger", [player, {**ground, "age": ground["age"] - 1}]),
                ("other slot", [{**player, "place": "/Inventory/1"}, aged]),
                ("other wear", [stack(f"player:{HOLDER}", "player", damaged), aged]),
                ("other dimension", [player, {**aged, "dimension": "minecraft:the_nether"}]),
                ("missing piece", [player]),
                ("extra piece", [player, aged, stack(f"player:{HOLDER}", "player", SPEAR, "/Inventory/4")])):
            result = verdict(reopened)
            self.assertEqual(result["verdict"], "incomplete", label)
            self.assertEqual(result["incomplete"][0]["reason"], br.DISAGREE, label)
        self.assertEqual(verdict([player, ground], tick=119)["verdict"], "incomplete", "reopen ran backwards")
        self.assertEqual(verdict([player, ground], offline=[player, {**ground, "age": 19}])["verdict"], "incomplete",
                         "the offline image itself older than its birth allows")
        unborn = census("crash", 120, [player, ground])
        self.assertEqual(br.reconcile([self.issued, self.dropped, unborn], self.kit, set(),
                                      [history(100, 200, spawns=[{**self.spawn, "age": None}])])["verdict"],
                         "incomplete", "no durable birth record bounds the age")
        duplicated = [player, ground, stack(f"player:{HOLDER}", "player", SPEAR, "/Inventory/4")]
        self.assertEqual(verdict(duplicated, offline=duplicated)["verdict"], "fail", "both exports hold two")
        self.assertEqual(verdict([player, ground], offline=duplicated)["verdict"], "incomplete",
                         "a duplicate only one export shows is not a verified red")
        missing = census("crash", 120, [player, ground])
        del missing["reopen"]
        self.assertEqual(br.reconcile([self.issued, self.dropped, missing], self.kit, set(), [self.history])["verdict"],
                         "incomplete")

    def test_recovery_tick_evidence_needs_the_crash_tail_and_the_next_tick(self):
        with tempfile.TemporaryDirectory() as directory:
            archive, crash, recovery = (Path(directory, name) for name in ("archive", "crash", "recovery"))
            for path in (archive, crash, recovery):
                path.mkdir()
            for tick in range(1, 11):
                batch(archive if tick <= 5 else crash, tick, tick, [{"Tick": tick}])
            for tick in range(6, 13):
                batch(recovery, tick, tick, [{"Tick": tick}])
            pre, rec = br.read_batches([archive, crash]), br.read_batches([recovery])
            image = {"checkpoint_tick": 5, "replay_tick": 10}
            evidence = br.recovery_tick_evidence(pre, rec, image)
            self.assertTrue(evidence["match"], evidence)
            self.assertEqual((evidence["reopened_tail"], evidence["recovered_tick"]), ([6, 10], 10))
            self.assertFalse(br.recovery_tick_evidence(pre, rec, {**image, "replay_tick": 9})["match"])
            self.assertFalse(br.recovery_tick_evidence(pre, rec, {**image, "checkpoint_tick": 4})["match"])
            (recovery / "batch-00000000000000000011-00000000000000000011.bin").unlink()
            self.assertFalse(br.recovery_tick_evidence(pre, br.read_batches([recovery]), image)["match"], "first new tick 12")
            batch(recovery, 10, 10, [{"Tick": "rewritten"}])
            self.assertEqual(br.recovery_tick_evidence(pre, br.read_batches([recovery]), image)["differing"], [[10, 10]])


class InputsAndOutputsTests(unittest.TestCase):
    def test_census_config_points_at_the_derived_world(self):
        text = '[data]\nworld_dir = "/run/world"\nvanilla_data_dir = "/c"\nbackup_dir = "/run/backups"\n'
        out = br.census_config(text, Path("/tmp/derived world"))
        self.assertIn('world_dir = "/tmp/derived world"', out)
        self.assertNotIn("backup_dir", out)
        with self.assertRaises(ValueError):
            br.census_config("[data]\n", Path("/x"))

    def test_reopen_config_uses_fresh_ports_and_a_private_plugin_copy(self):
        text = ('[network]\nport = 1\n[dashboard]\nport = 2\n[plugins]\ndirectory = "/run/plugins"\n'
                '[data]\nworld_dir = "/run/world"\nbackup_dir = "/run/backups"\n')
        out, source = br.reopen_config(text, Path("/tmp/reopen/world"), Path("/tmp/reopen/plugins"))
        self.assertEqual(source, Path("/run/plugins"))
        self.assertIn('directory = "/tmp/reopen/plugins"', out)
        self.assertIn('world_dir = "/tmp/reopen/world"', out)
        self.assertNotIn("backup_dir", out)
        ports = [line for line in out.splitlines() if line.startswith("port =")]
        self.assertEqual(len(ports), 2)
        self.assertNotIn("port = 1", ports)
        self.assertNotIn("port = 2", ports)

    def test_census_refuses_a_compact_that_changes_more_than_the_redb_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stopped = root / "stopped"
            (stopped / "native").mkdir(parents=True)
            (stopped / "journal").mkdir()
            (stopped / "native" / "state.redb").write_bytes(b"redb")
            (stopped / "journal" / "batch-1-1.bin").write_bytes(b"wal")
            toml = root / "server.toml"
            toml.write_text(f'[data]\nworld_dir = "{stopped}"\n')
            ctx = profiles.ProfileContext(root)

            def compact(argv, ctx, log):
                world = Path(argv[argv.index("--config") + 1]).parent / "world"
                (world / "native" / "state.redb").write_bytes(b"repaired")
                (world / "journal" / "batch-1-1.bin").write_bytes(b"rewritten")

            with mock.patch.object(br, "reopen_image", return_value={}), \
                    mock.patch.object(br, "_run_command", side_effect=compact):
                with self.assertRaisesRegex(br.Incomplete, "journal/batch-1-1.bin"):
                    br.take_census(ctx, Path("/bin/true"), toml, root / "census" / "crash", phase="crash", epoch=0,
                                   stopped=stopped, hashes=br.file_hashes(stopped))
            recorded = json.loads((root / "census" / "crash" / "compact.json").read_text())
            self.assertEqual(recorded["changed"], ["journal/batch-1-1.bin", "native/state.redb"])
            self.assertEqual(recorded["state_redb_before"], br.file_hashes(stopped)["native/state.redb"])

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

    def test_password_scan_reports_paths_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "clean.log").write_text("login [redacted]\n")
            (root / "sub").mkdir()
            (root / "sub/leak.bin").write_bytes(b"\x00secret-pw\x00")
            leaks = br.password_leaks(root, ["secret-pw"])
            self.assertEqual(leaks, [str(root / "sub/leak.bin")])
            self.assertNotIn("secret-pw", json.dumps(leaks))

    def test_online_census_groups_replies_by_marker(self):
        class Fake:
            def __init__(self):
                self.out = []

            def mark(self):
                return 0

            def send(self, command):
                if command.startswith("clear @a minecraft:iron_spear"):
                    self.out += ["Found 1 matching item(s) on player PovRed", "No items were found on player CamSpec"]
                elif command.startswith("clear"):
                    self.out += ["No items were found on player PovRed"]
                else:
                    self.out.append("[Server] " + command.split(" ", 1)[1])

            def wait(self, pattern, mark=0, timeout=0, fail=None):
                return None

            def text_since(self, mark):
                return "\n".join(self.out)

        counts = br.online_census(Fake(), ["minecraft:iron_spear", "minecraft:shield"], "p")
        self.assertEqual(counts, {"minecraft:iron_spear": {"PovRed": 1, "CamSpec": 0},
                                  "minecraft:shield": {"PovRed": 0}})

    def test_named_scenarios_only(self):
        ctx = profiles.ProfileContext(Path("/nonexistent"), scenario_id=None)
        with self.assertRaises(ValueError):
            br.run(ctx)
        with self.assertRaises(ValueError):
            br.run(profiles.ProfileContext(Path("/nonexistent"), scenario_id="battle-kit-census", extra_args=["--x"]))


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
