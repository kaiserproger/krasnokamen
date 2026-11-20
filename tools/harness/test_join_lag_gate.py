"""Unit tests for the join-lag gate: its options, verdicts and row-facing registration."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from .backends import join_lag
from .profiles import PROFILES, ProfileContext


def context(*extra: str, client: str = "native", timeout: float | None = None) -> ProfileContext:
    return ProfileContext(artifact_dir=Path("/tmp/unused"), extra_args=list(extra),
                          client=client, timeout_seconds=timeout)


class ArgsTest(unittest.TestCase):
    def test_defaults_are_the_gate_options(self) -> None:
        timeout, tick_rate, flood_clients, flood_steps, pump_rate = join_lag.validate_args(context())
        self.assertEqual((timeout, tick_rate, flood_clients, flood_steps, pump_rate),
                         (join_lag.TIMEOUT, 20, 1, 40, 3))

    def test_options_select_the_load_and_the_rate(self) -> None:
        timeout, tick_rate, flood_clients, flood_steps, pump_rate = join_lag.validate_args(
            context("--tick-rate", "10", "--flood-clients", "2", "--flood-steps", "8",
                    "--pump-publish-per-second", "10", timeout=600.0))
        self.assertEqual((timeout, tick_rate, flood_clients, flood_steps, pump_rate),
                         (600.0, 10, 2, 8, 10))

    def test_a_java_client_is_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, "requires --client native"):
            join_lag.validate_args(context(client="java"))

    def test_out_of_range_options_are_refused(self) -> None:
        for argv in (("--tick-rate", "0"), ("--tick-rate", "21"),
                     ("--flood-clients", "-1"), ("--flood-clients", "5"),
                     ("--flood-steps", "3"), ("--flood-steps", "129"),
                     ("--pump-publish-per-second", "0"), ("--pump-publish-per-second", "101"),
                     ("--scenario", "border-reveal")):
            with self.subTest(argv=argv):
                with self.assertRaises((ValueError, SystemExit)):
                    join_lag.validate_args(context(*argv))

    def test_the_timeout_bounds_are_enforced(self) -> None:
        for timeout in (0.0, 2400.5):
            with self.subTest(timeout=timeout):
                with self.assertRaisesRegex(ValueError, "timeout-seconds"):
                    join_lag.validate_args(context(timeout=timeout))


class JoinVerdictTest(unittest.TestCase):
    def test_a_join_inside_the_deadline_behind_a_lagging_pump_passes(self) -> None:
        ok, why = join_lag.join_verdict(2.5, 20, None)
        self.assertTrue(ok)
        self.assertIn("2.5 s", why)

    def test_a_join_past_the_deadline_fails(self) -> None:
        ok, why = join_lag.join_verdict(10.5, 20, None)
        self.assertFalse(ok)
        self.assertIn("deadline", why)

    def test_an_unreached_lag_is_inconclusive_not_false(self) -> None:
        ok, why = join_lag.join_verdict(2.5, join_lag.LAG_TICKS - 1, None)
        self.assertIsNone(ok)
        self.assertIn("did not occur", why)

    def test_a_missing_join_behind_the_lag_fails(self) -> None:
        ok, why = join_lag.join_verdict(None, 20, None)
        self.assertFalse(ok)
        self.assertIn("never landed", why)

    def test_a_core_halt_fails_even_when_the_join_landed(self) -> None:
        ok, why = join_lag.join_verdict(2.5, 20, "ERROR edge pump not draining")
        self.assertFalse(ok)
        self.assertIn("halted", why)


class HaltVerdictTest(unittest.TestCase):
    def test_a_ticking_core_behind_a_lagging_pump_passes(self) -> None:
        ok, _ = join_lag.halt_verdict(20, 100, 160, None)
        self.assertTrue(ok)

    def test_a_halted_core_fails(self) -> None:
        ok, _ = join_lag.halt_verdict(20, 100, 160, "ERROR edge pump not draining")
        self.assertFalse(ok)

    def test_a_stuck_simulation_tick_fails(self) -> None:
        ok, why = join_lag.halt_verdict(20, 100, 100, None)
        self.assertFalse(ok)
        self.assertIn("did not advance", why)

    def test_an_unreached_lag_is_inconclusive_not_false(self) -> None:
        ok, why = join_lag.halt_verdict(join_lag.LAG_TICKS - 1, 100, 160, None)
        self.assertIsNone(ok)
        self.assertIn("did not occur", why)


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

    def test_a_symptom_outranks_an_unreached_condition(self) -> None:
        self.assertEqual(join_lag.disposition({"a": None, "b": False}), ("failed", ["b"]))

    def test_neither_verdict_returns_false_without_the_lag(self) -> None:
        tight = join_lag.LAG_TICKS - 1
        self.assertIsNone(join_lag.join_verdict(30.0, tight, None)[0])
        self.assertIsNone(join_lag.halt_verdict(tight, 100, 160, None)[0])


class PumpLagTest(unittest.TestCase):
    def test_the_lag_is_the_core_tick_minus_the_published_tick(self) -> None:
        stats = {"simulation_tick": 120, "publish": {"source_tick": 100}}
        self.assertEqual(join_lag.pump_lag(stats), 20)

    def test_an_unpublished_or_ahead_pump_is_zero(self) -> None:
        self.assertEqual(join_lag.pump_lag({"simulation_tick": 120, "publish": None}), 0)
        self.assertEqual(join_lag.pump_lag({"simulation_tick": 100, "publish": {"source_tick": 104}}), 0)


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
