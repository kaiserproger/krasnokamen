"""Unit tests for the join-lag gate: its options, verdicts and row-facing registration."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from .backends import join_lag
from .profiles import PROFILES, ProfileContext


def context(*extra: str, client: str = "native", timeout: float | None = None) -> ProfileContext:
    return ProfileContext(artifact_dir=Path("/tmp/unused"), extra_args=list(extra),
                          client=client, timeout_seconds=timeout)


class ArgsTest(unittest.TestCase):
    def test_defaults_are_the_gate_options(self) -> None:
        timeout, tick_rate, flood_clients, flood_steps, scenario = join_lag.validate_args(context())
        self.assertEqual((timeout, tick_rate, flood_clients, flood_steps, scenario),
                         (join_lag.TIMEOUT, 20, 3, 60, "all"))

    def test_options_select_a_row_and_the_flood_load(self) -> None:
        timeout, tick_rate, flood_clients, flood_steps, scenario = join_lag.validate_args(
            context("--scenario", "border-reveal", "--tick-rate", "3", "--flood-clients", "2",
                    "--flood-steps", "8", timeout=600.0))
        self.assertEqual((timeout, tick_rate, flood_clients, flood_steps, scenario),
                         (600.0, 3, 2, 8, "border-reveal"))

    def test_a_java_client_is_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, "requires --client native"):
            join_lag.validate_args(context(client="java"))

    def test_out_of_range_options_are_refused(self) -> None:
        for argv in (("--tick-rate", "0"), ("--tick-rate", "21"),
                     ("--flood-steps", "3"), ("--flood-steps", "129"),
                     ("--flood-clients", "0"), ("--flood-clients", "5"),
                     ("--scenario", "other")):
            with self.subTest(argv=argv):
                with self.assertRaises((ValueError, SystemExit)):
                    join_lag.validate_args(context(*argv))

    def test_the_timeout_bounds_are_enforced(self) -> None:
        for timeout in (0.0, 2400.5):
            with self.subTest(timeout=timeout):
                with self.assertRaisesRegex(ValueError, "timeout-seconds"):
                    join_lag.validate_args(context(timeout=timeout))


class JoinVerdictTest(unittest.TestCase):
    def test_a_join_inside_the_deadline_behind_skips_passes(self) -> None:
        ok, why = join_lag.join_verdict(2.5, 4, 9, None)
        self.assertTrue(ok)
        self.assertIn("2.5 s", why)

    def test_a_join_past_the_deadline_fails(self) -> None:
        ok, why = join_lag.join_verdict(10.5, 4, 9, None)
        self.assertFalse(ok)
        self.assertIn("deadline", why)

    def test_a_clean_pump_is_inconclusive_not_false(self) -> None:
        ok, why = join_lag.join_verdict(2.5, 4, 4, None)
        self.assertIsNone(ok)
        self.assertIn("did not occur", why)

    def test_a_missing_join_fails(self) -> None:
        ok, why = join_lag.join_verdict(None, 4, 9, None)
        self.assertFalse(ok)
        self.assertIn("never entered play", why)

    def test_a_core_halt_fails_even_when_the_join_landed(self) -> None:
        ok, why = join_lag.join_verdict(2.5, 4, 9, "ERROR edge pump not draining")
        self.assertFalse(ok)
        self.assertIn("halted", why)


class HaltVerdictTest(unittest.TestCase):
    def test_a_ticking_core_behind_skips_passes(self) -> None:
        ok, _ = join_lag.halt_verdict(10, 30, 100, 160, None)
        self.assertTrue(ok)

    def test_a_halted_core_fails(self) -> None:
        ok, _ = join_lag.halt_verdict(10, 30, 100, 160, "ERROR edge pump not draining")
        self.assertFalse(ok)

    def test_a_stuck_simulation_tick_fails(self) -> None:
        ok, why = join_lag.halt_verdict(10, 30, 100, 100, None)
        self.assertFalse(ok)
        self.assertIn("did not advance", why)

    def test_no_skips_is_inconclusive_not_false(self) -> None:
        ok, why = join_lag.halt_verdict(10, 10, 100, 160, None)
        self.assertIsNone(ok)
        self.assertIn("did not occur", why)


class BorderVerdictTest(unittest.TestCase):
    def test_a_reveal_after_a_skip_passes(self) -> None:
        ok, _ = join_lag.border_verdict(join_lag.DIAMOND, 1, 3)
        self.assertTrue(ok)

    def test_a_stale_mask_fails(self) -> None:
        ok, why = join_lag.border_verdict(join_lag.STONE, 1, 3)
        self.assertFalse(ok)
        self.assertIn("stone", why)

    def test_a_reveal_without_a_skip_is_inconclusive_not_false(self) -> None:
        ok, why = join_lag.border_verdict(join_lag.DIAMOND, 3, 3)
        self.assertIsNone(ok)
        self.assertIn("did not occur", why)

    def test_an_unread_block_fails(self) -> None:
        ok, _ = join_lag.border_verdict(None, 1, 3)
        self.assertFalse(ok)


class DispositionTest(unittest.TestCase):
    """A false verdict reads as the row's symptom in the ledger; an unreached condition must not."""

    def test_a_clean_run_passes(self) -> None:
        self.assertEqual(join_lag.disposition({"a": True, "b": True}), ("passed", []))

    def test_a_real_symptom_fails_and_is_named(self) -> None:
        self.assertEqual(join_lag.disposition({"a": True, "b": False}), ("failed", ["b"]))

    def test_an_unreached_condition_is_inconclusive_never_false(self) -> None:
        outcome, names = join_lag.disposition({"a": None, "b": True})
        self.assertEqual(outcome, "inconclusive")
        self.assertEqual(names, ["a"])
        self.assertEqual(join_lag.disposition({"a": None}), ("inconclusive", ["a"]))

    def test_a_symptom_outranks_an_unreached_condition(self) -> None:
        self.assertEqual(join_lag.disposition({"a": None, "b": False}), ("failed", ["b"]))

    def test_every_verdict_function_returns_none_without_a_skip(self) -> None:
        self.assertIsNone(join_lag.join_verdict(1.0, 5, 5, None)[0])
        self.assertIsNone(join_lag.halt_verdict(5, 5, 10, 20, None)[0])
        self.assertIsNone(join_lag.border_verdict(join_lag.DIAMOND, 5, 5)[0])
        self.assertIsNone(join_lag.border_verdict(join_lag.STONE, 5, 5)[0])


class LayoutTest(unittest.TestCase):
    def test_the_ore_sits_at_the_chunk_edge_with_its_side_cell_beyond(self) -> None:
        for x in (100, 127, 128, 200):
            with self.subTest(x=x):
                plan = join_lag.border_layout(x, 64, 0)
                ore_x, ore_y, ore_z = plan["ore"]
                side_x, side_y, side_z = plan["side"]
                self.assertEqual(ore_x % 16, 15)
                self.assertEqual(ore_x // 16, x // 16)
                self.assertEqual((side_x, side_y, side_z), (ore_x + 1, ore_y, ore_z))
                self.assertEqual(side_x // 16, ore_x // 16 + 1)
                self.assertLess(ore_y, 64)

    def test_the_fill_box_holds_the_ore_and_its_side_cell(self) -> None:
        plan = join_lag.border_layout(100, 64, 0)
        x0, y0, z0, x1, y1, z1 = plan["fill"]
        for x, y, z in (plan["ore"], plan["side"]):
            self.assertTrue(x0 <= x <= x1 and y0 <= y <= y1 and z0 <= z <= z1)


class HaltLineTest(unittest.TestCase):
    def test_a_halt_line_is_found(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text("INFO tick\nERROR edge pump not draining; core halted\n")
            self.assertIn("edge pump not draining", join_lag.halt_line(log) or "")

    def test_a_healthy_log_has_none(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text("INFO tick\nINFO joined\n")
            self.assertIsNone(join_lag.halt_line(log))

    def test_a_missing_log_has_none(self) -> None:
        self.assertIsNone(join_lag.halt_line(Path("/tmp/definitely-missing-join-lag.log")))


class JoinTimingTest(unittest.TestCase):
    LOG = (
        "2026-11-20T01:18:15.209868Z  INFO mc_edge::server::connection: "
        "login admitted peer=127.0.0.1:53492 connection=2 name=JoinLagWitness protocol=777 release=\"26.3\"\n"
        "2026-11-20T01:18:17.827594Z  INFO mc_edge::server::connection: "
        "login admitted peer=127.0.0.1:53510 connection=4 name=JoinLagJoiner protocol=777 release=\"26.3\"\n"
        "2026-11-20T01:18:18.000000Z DEBUG mc_edge::server::pump: connection = 2 joined=1 \"join landed\"\n"
        "2026-11-20T01:18:30.827594Z DEBUG mc_edge::server::pump: connection = 4 joined=1 \"join landed\"\n"
    )

    def test_the_join_is_measured_from_admission_to_landing(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(self.LOG)
            self.assertAlmostEqual(join_lag.join_timing(log, "JoinLagJoiner"), 13.0, places=3)
            self.assertAlmostEqual(join_lag.join_timing(log, "JoinLagWitness"), 2.79, places=2)

    def test_a_kicked_join_has_no_timing(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(self.LOG.replace(' connection = 4 joined=1 "join landed"', ""))
            self.assertIsNone(join_lag.join_timing(log, "JoinLagJoiner"))

    def test_an_unknown_client_has_no_timing(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(self.LOG)
            self.assertIsNone(join_lag.join_timing(log, "Nobody"))


class RegistrationTest(unittest.TestCase):
    def test_the_profile_is_a_windowed_real_client_gate(self) -> None:
        from . import regressions

        self.assertEqual(PROFILES["join-lag-gate"]["scope"], "real-client")
        self.assertIn("join-lag-gate", regressions._WINDOWED_PROFILES)

    def test_every_verdict_is_a_registered_row_ref(self) -> None:
        from . import regressions

        for verdict in join_lag.CRITERIA:
            with self.subTest(verdict=verdict):
                self.assertTrue(regressions.is_graphical_ref(f"harness:join-lag-gate:{verdict}"))
                self.assertIn(verdict, regressions._BOOLEAN_PROFILE_VERDICTS["join-lag-gate"])

    def test_a_foreign_verdict_is_not_a_ref(self) -> None:
        from . import regressions

        self.assertFalse(regressions.is_graphical_ref("harness:join-lag-gate:not_a_verdict"))
        self.assertFalse(regressions.is_graphical_ref("client:join-lag-gate:join_behind_slow_pump"))


if __name__ == "__main__":
    unittest.main()
