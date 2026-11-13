import unittest
from types import SimpleNamespace
from unittest import mock

from tools.harness.backends import buildings_qa


class OrientationTest(unittest.TestCase):
    def observed(self, bp, level, origin, turns):
        return {pos: buildings_qa._state_base(want)
                for pos, want in buildings_qa.oriented_cells(bp, level, origin, turns).items()}

    def test_best_turns_reads_the_orientation_the_world_was_built_in(self):
        origin = (118, 69, -11)
        for bp, level in (("house", 1), ("wall", 1), ("wall", 2), ("wall", 3), ("gate", 3),
                          ("armorer", 2)):
            for turns in range(4):
                with self.subTest(bp=bp, level=level, turns=turns):
                    world = self.observed(bp, level, origin, turns)
                    cells = {t: buildings_qa.oriented_cells(bp, level, origin, t) for t in range(4)}
                    best = buildings_qa.best_turns(cells, world)
                    # A rotation-symmetric blueprint level reads as its lowest equivalent turn count.
                    self.assertEqual(buildings_qa.oriented_cells(bp, level, origin, best),
                                     buildings_qa.oriented_cells(bp, level, origin, turns))

    def test_a_house_with_the_door_on_the_east_side_is_not_rot_zero(self):
        # buildings-qa 2026-11-12: 13 of 90 house cells "mismatched" because the build was rotated.
        origin = (118, 69, -11)
        world = self.observed("house", 1, origin, 3)
        cells = {t: buildings_qa.oriented_cells("house", 1, origin, t) for t in range(4)}
        self.assertEqual(buildings_qa.best_turns(cells, world), 3)

    def test_every_level_of_a_building_rotates_in_one_frame(self):
        # The server rotates every level in the largest level's frame about one placement origin
        # (blueprints.rs fx/fz): the cells an upgrade keeps must keep their world position, so the
        # count of kept cells cannot depend on the rotation. A level-own frame breaks that at
        # quarter turns whenever a level is wider than the previous one.
        true = (100, 70, 200)
        for bp, top in buildings_qa.LEVELS.items():
            for level in range(1, top):
                kept = set()
                for turns in range(4):
                    now = buildings_qa.cells_true(bp, level, true, turns)
                    nxt = buildings_qa.cells_true(bp, level + 1, true, turns)
                    kept.add(sum(1 for pos, block in now.items() if nxt.get(pos) == block))
                with self.subTest(bp=bp, level=level):
                    self.assertEqual(len(kept), 1)
                    self.assertGreater(kept.pop(), 0)

    def test_the_certificate_minimum_is_the_origin_of_every_level(self):
        # A listing reports the certificate minimum; the placement origin is recovered from it.
        true = (100, 70, 200)
        for bp, top in buildings_qa.LEVELS.items():
            for level in range(1, top + 1):
                for turns in range(4):
                    dx, dz = buildings_qa._cert_offset(bp, level, turns)
                    listed = (true[0] + dx, true[1], true[2] + dz)
                    with self.subTest(bp=bp, level=level, turns=turns):
                        self.assertEqual(buildings_qa.true_origin(bp, level, listed, turns), true)

    def test_the_new_kinds_are_registered_everywhere_a_kind_is_listed(self):
        for bp in ("gate", "armorer", "butcher", "cartographer", "cleric", "fisherman",
                   "fletcher", "leatherworker", "librarian", "toolsmith", "weaponsmith"):
            with self.subTest(bp=bp):
                self.assertIn(bp, buildings_qa.BLUEPRINTS)
                self.assertIn(bp, buildings_qa.PARALLEL_SITES)
                self.assertEqual(buildings_qa.LEVELS[bp], 3 if bp == "gate" else 2)
        self.assertEqual({bp: buildings_qa.LEVELS[bp] for bp in ("wall", "watch_tower", "barracks")},
                         {"wall": 3, "watch_tower": 3, "barracks": 3})
        self.assertEqual(len(set(buildings_qa.PARALLEL_SITES.values())),
                         len(buildings_qa.PARALLEL_SITES))


class BaseMaterialsTest(unittest.TestCase):
    LISTING = (
        "== armorer (armorer)\n"
        "  L1: 135 items; base {'blast_furnace': 1, 'coal': 1, 'cobblestone': 61, 'oak_log': 30}; "
        "stations ['blast_furnace']\n"
        "      blast_furnacex1, chestx1\n"
        "  L2: 49 items; base {'blast_furnace': 1, 'cobblestone': 22, 'oak_log': 11}; "
        "stations ['blast_furnace']\n"
        "== gate (gate)\n"
        "  L3: 16 items; base {'coal': 1, 'cobblestone': 14, 'oak_log': 2}\n")

    def test_a_station_kind_suffix_does_not_hide_the_base(self):
        self.assertEqual(buildings_qa._base_materials(self.LISTING, 2),
                         {"blast_furnace": 1, "cobblestone": 22, "oak_log": 11})
        self.assertEqual(buildings_qa._base_materials(self.LISTING, 1)["oak_log"], 30)

    def test_a_plain_kind_still_parses_and_a_missing_level_is_empty(self):
        self.assertEqual(buildings_qa._base_materials(self.LISTING, 3),
                         {"coal": 1, "cobblestone": 14, "oak_log": 2})
        self.assertEqual(buildings_qa._base_materials(self.LISTING, 9), {})


class BarracksFunctionTest(unittest.TestCase):
    """buildings-qa 2026-11-13: the Residents dialog the old check waited for is a container menu
    now; the soldier beds are the `Soldiers: f/n` lore of the Buildings menu icon."""

    class MenuProbe:
        def __init__(self, slots, lore):
            self.slots, self.lore, self.calls = slots, lore, []
            self.report = {"id": "buildings-qa", "assertions": {}}

        def call(self, name, arguments=None):
            self.calls.append((name, arguments))
            if name == "minecraft_read_item_components":
                return {"components": {"minecraft:lore": self.lore[arguments["slot"]]}}
            return {}

        def wait(self, predicate, label, **_):
            state = {"screen": {"class": buildings_qa.CONTAINER_CLASS, "title": "Buildings \u00b7 bqa"},
                     "container": {"slots": self.slots}}
            assert predicate(state)
            return state

        def check(self, name, condition):
            self.report["assertions"][name] = bool(condition)
            if not condition:
                raise RuntimeError(f"assertion failed: {name}")

    def probe(self, beds):
        slots = [{"slot": 0, "name": "#8589934593 Storage L1"},
                 {"slot": 1, "name": "#8589934594 Barracks L2"}]
        lore = {0: [{"text": "Storage"}],
                1: [[{"text": "Soldiers: 0/"}, {"text": str(beds)}], {"text": "[1, 2, 3] .. [4, 5, 6]"}]}
        return self.MenuProbe(slots, lore)

    def test_the_icon_lore_gives_the_soldier_beds(self):
        probe, sub = self.probe(4), {}
        with mock.patch.object(buildings_qa, "_close_any_screen"):
            buildings_qa._function_barracks(probe, sub, "buildings_barracks", 2, (0, 0, 0), {}, "bqa")
        self.assertEqual(sub["behaviour"]["L2_function_barracks"], "PASS")
        self.assertIn("beds=4", sub["evidence"]["L2_function_barracks"])

    def test_a_wrong_bed_count_fails(self):
        probe, sub = self.probe(2), {}
        with mock.patch.object(buildings_qa, "_close_any_screen"):
            with self.assertRaisesRegex(RuntimeError, "buildings_barracks_L2_barracks"):
                buildings_qa._function_barracks(probe, sub, "buildings_barracks", 2, (0, 0, 0), {}, "bqa")

    def test_a_missing_icon_reads_no_beds(self):
        probe = self.probe(4)
        probe.slots = probe.slots[:1]
        with mock.patch.object(buildings_qa, "_close_any_screen"):
            self.assertIsNone(buildings_qa._building_lore(probe, buildings_qa.re.compile(r"#\d+ Barracks L2")))


class FakeProbe:
    """Enough of ProbeSteps for the order and admission helpers; check raises like the real one."""

    def __init__(self, chat=()):
        self.chat = list(chat)
        self.report = {"id": "buildings-qa", "assertions": {}}
        self.waits = 0
        self.sent = 0
        self.deadline = 1e12

    def check(self, name, condition):
        self.report["assertions"][name] = bool(condition)
        if not condition:
            raise RuntimeError(f"buildings-qa assertion failed: {name}")

    def call(self, name, arguments=None):
        if name == "minecraft_wait_ticks":
            self.waits += 1
        return {}

    def fixture(self, command):
        pass

    def observe(self, **_):
        return {"player": {"x": 10.0, "y": 70.0, "z": 10.0}, "recent_chat": list(self.chat)}


HEADLESS = SimpleNamespace(windowed=False)


class OrderEchoRetryTest(unittest.TestCase):
    def run_retry(self, lines, project_active=False, ctx=HEADLESS):
        probe, sub = FakeProbe(), {}

        def send(_probe, command, settle_ticks=40):
            probe.sent += 1
            return {"recent_chat": list(lines)}

        with mock.patch.object(buildings_qa, "_send", send), \
                mock.patch.object(buildings_qa, "_project_active_quiet",
                                  return_value=project_active) as active:
            result = buildings_qa._order_echo_retry(
                probe, sub, "buildings_gate", "gate", 3, "settlement build upgrade 7", False, ctx,
                upgrade_bid=7)
        return result, probe, sub, active

    def test_an_echo_that_started_the_build_stops_the_retries(self):
        result, probe, _, active = self.run_retry(["Started gate construction."])
        self.assertTrue(result)
        self.assertEqual(probe.sent, 1)
        active.assert_not_called()

    def test_no_blueprint_upgrade_while_the_planner_project_runs_is_admitted(self):
        result, probe, sub, active = self.run_retry(
            ["No blueprint upgrade available for this building."], project_active=True)
        self.assertTrue(result)
        self.assertEqual(probe.sent, 1)
        active.assert_called_once()
        self.assertIn("No blueprint upgrade available", sub["L3_order_attempt1"])

    def test_no_blueprint_upgrade_without_a_project_is_retried_then_refused(self):
        result, probe, _, active = self.run_retry(
            ["No blueprint upgrade available for this building."], project_active=False)
        self.assertFalse(result)
        self.assertEqual(probe.sent, 3)
        self.assertEqual(active.call_count, 3)

    def test_a_busy_refusal_still_reads_the_headless_echo(self):
        result, probe, _, active = self.run_retry(["Busy."])
        self.assertTrue(result)
        active.assert_not_called()


class AdmissionTest(unittest.TestCase):
    def test_an_admitted_order_needs_no_cert(self):
        probe = FakeProbe()
        with mock.patch.object(buildings_qa, "_building_bounds") as bounds:
            self.assertIsNone(buildings_qa._admitted_or_caught(
                probe, {}, "buildings_gate", "gate", 3, True))
        bounds.assert_not_called()
        self.assertTrue(probe.report["assertions"]["buildings_gate_L3_project"])

    def test_a_refused_order_is_fine_when_the_planner_already_certified_the_level(self):
        probe = FakeProbe()
        with mock.patch.object(buildings_qa, "_building_bounds", return_value=(1, 2, 3)):
            caught = buildings_qa._admitted_or_caught(
                probe, {}, "buildings_gate", "gate", 3, False)
        self.assertEqual(caught, (1, 2, 3))
        self.assertTrue(probe.report["assertions"]["buildings_gate_L3_project"])

    def test_a_refused_order_without_a_cert_fails_the_project_check(self):
        probe = FakeProbe()
        with mock.patch.object(buildings_qa, "_building_bounds", return_value=None):
            with self.assertRaisesRegex(RuntimeError, "buildings_gate_L3_project"):
                buildings_qa._admitted_or_caught(probe, {}, "buildings_gate", "gate", 3, False)


class SequentialFlowTest(unittest.TestCase):
    """`_scenario_sequential` for one blueprint with every game-facing helper replaced."""

    def run_flow(self, bp, top, order_admitted, cert_after_order):
        probe = FakeProbe()
        detail: dict = {}
        certs = {}
        events = []

        def building_bounds(_probe, _bp, level, exclude=None):
            return certs.get(level)

        def order(_probe, sub, tag, _bp, level, cmd, use_dialog, ctx, dialog_fn=None,
                  upgrade_bid=None):
            events.append(("order", level, use_dialog))
            if cert_after_order:
                certs[level] = (level, 70, level)
            return order_admitted

        def wait_cert(_probe, sub, tag, _bp, level, ctx, artifact):
            certs[level] = (level, 70, level)
            sub[f"L{level}_origin"] = list(certs[level])
            return certs[level]

        def fund(_probe, sub, _bp, level, chest):
            sub.setdefault("behaviour", {})[f"L{level}_funded"] = "PASS"

        ctx = SimpleNamespace(windowed=False)
        patches = dict(
            _send=lambda _p, command, settle_ticks=40: {
                "recent_chat": ["anchor=[0, 70, 0]"] if "info" in command else []},
            _echo=lambda _p, label: {"recent_chat": [f"Founded bqa_{bp}"]},
            _hand_storage_at=lambda *a, **k: (5, 70, 5),
            _summon_bind=lambda *a, **k: True,
            _fund_level=fund,
            _order_echo_retry=order,
            _building_bounds=building_bounds,
            _building_seq=lambda *a, **k: 7,
            _adversarial_break=lambda *a, **k: False,
            _wait_certified_origin_here=wait_cert,
            _verify_level=lambda *a, **k: None,
        )
        with mock.patch.multiple(buildings_qa, **patches):
            buildings_qa._scenario_sequential(probe, detail, ctx, None, (bp,), "", None, None)
        return probe, detail["blueprints"][bp], events

    def test_a_passing_run_records_every_level_verdict(self):
        probe, sub, events = self.run_flow("gate", 3, order_admitted=True, cert_after_order=False)
        for name in ("L1_built", "L2_upgrade", "L3_upgrade"):
            self.assertEqual(sub["behaviour"].get(name), "PASS", name)
        self.assertEqual([level for _, level, _ in events], [1, 2, 3])
        self.assertTrue(probe.report["assertions"]["buildings_gate_L3_project"])

    def test_barracks_and_house_orders_never_wait_for_the_retired_projects_dialog(self):
        # buildings-qa 2026-11-13, barracks: the dialog path waited for a screen class that the
        # container Projects menu never opens, and ran into the 1800 s deadline.
        for bp, top in (("barracks", 3), ("house", 3)):
            _, _, events = self.run_flow(bp, top, order_admitted=True, cert_after_order=False)
            self.assertEqual({dialog for _, _, dialog in events}, {False})

    def test_a_workshop_records_its_level_two_verdict(self):
        _, sub, _ = self.run_flow("armorer", 2, order_admitted=True, cert_after_order=False)
        self.assertEqual(sub["behaviour"].get("L2_upgrade"), "PASS")
        self.assertNotIn("L3_upgrade", sub["behaviour"])

    def test_an_order_the_planner_beat_is_followed_not_failed(self):
        # buildings-qa 2026-11-13, gate L3: the server answered "No blueprint upgrade available"
        # because the planner had already built L3; the failed project check ended the run.
        probe, sub, _ = self.run_flow("gate", 3, order_admitted=False, cert_after_order=True)
        self.assertEqual(sub["behaviour"].get("L3_upgrade"), "PASS")
        self.assertEqual(sub.get("L3_admission"), "planner-admitted")
        self.assertTrue(probe.report["assertions"]["buildings_gate_L3_project"])

    def test_an_order_nobody_admitted_still_fails(self):
        with self.assertRaisesRegex(RuntimeError, "buildings_gate_L1_project"):
            self.run_flow("gate", 3, order_admitted=False, cert_after_order=False)


if __name__ == "__main__":
    unittest.main()
