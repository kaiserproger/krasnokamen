import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from tools.harness import profiles
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


class ProjectMenuTest(unittest.TestCase):
    def test_order_uses_container_icon_then_real_confirmation(self):
        probe, sub = mock.Mock(), {}
        probe.wait.return_value = {"screen": {"title": "Projects"}}
        with mock.patch.object(buildings_qa, "_send") as send, \
                mock.patch.object(buildings_qa, "_menu_click_named") as click, \
                mock.patch.object(buildings_qa, "_confirm_project") as confirm:
            buildings_qa._order_via_dialog(probe, sub, "buildings_house", "house", 1, "town")
        send.assert_called_once_with(probe, "settlement menu projects")
        click.assert_called_once_with(probe, "Projects", "House L1")
        confirm.assert_called_once_with(probe, "Order: House")

    def test_upgrade_selects_exact_building_and_confirms_its_level(self):
        probe, sub = mock.Mock(), {}
        probe.wait.return_value = {"screen": {"title": "Projects"}, "container": {"slots": [
            {"slot": 52, "name": "Order construction L2"}]}}
        with mock.patch.object(buildings_qa, "_send"), \
                mock.patch.object(buildings_qa, "_menu_click_named") as click, \
                mock.patch.object(buildings_qa, "_confirm_project") as confirm:
            buildings_qa._upgrade_via_dialog(probe, sub, "buildings_house", "house", (1 << 32) | 7, 2, "town")
        click.assert_called_once_with(probe, "Buildings", "#7 House L1")
        confirm.assert_called_once_with(probe, "Order construction L2 #7")

    def test_queued_icon_is_not_reported_as_an_active_project(self):
        class MenuProbe:
            def call(self, *args):
                return {}

            def remaining(self):
                return 60

            def wait(self, predicate, label, **kwargs):
                if label == "projects_menu_open":
                    state = {"screen": {"class": buildings_qa.CONTAINER_CLASS, "title": "Projects"},
                             "container": {"slots": [{"slot": 52, "name": "Status"}]}}
                else:
                    state = {"screen": {"class": buildings_qa.CONTAINER_CLASS, "title": "Construction"},
                             "container": {"slots": [
                                 {"slot": 4, "name": "Cleric's workshop L1"},
                                 {"slot": 10, "name": "Queued — site not assigned\nPending: Cleric L1"}]}}
                assert predicate(state)
                return state

        with mock.patch.object(buildings_qa, "_close_any_screen"):
            self.assertEqual(buildings_qa._project_view(MenuProbe()), ("Cleric's workshop", 1, False))
            self.assertIsNone(buildings_qa._project_status(MenuProbe()))


class ResidentsMenuTest(unittest.TestCase):
    def test_resident_lore_carries_profession_and_specific_workplace(self):
        class Probe:
            def __init__(self):
                self.calls = []

            def call(self, name, args=None):
                self.calls.append((name, args))
                if name == "minecraft_read_item_components":
                    return {"components": {"minecraft:lore": [
                        {"text": "Farmer"}, {"text": "Works at #7 Farmer workshop L1"}]}}
                return {}

            def observe(self):
                return {}

            def wait(self, predicate, label, **kwargs):
                state = {"screen": {"class": buildings_qa.CONTAINER_CLASS,
                                    "title": "Residents / households"},
                         "container": {"slots": [{"slot": 9, "item_id": "minecraft:player_head"}]}}
                assert predicate(state)
                return state

        probe = Probe()
        lines = buildings_qa._residents_text(probe)
        self.assertEqual(lines, ["Farmer works at #7 Farmer workshop L1"])
        self.assertEqual(probe.calls[-1][0], "minecraft_close_screen")

    def test_census_continues_to_the_next_page(self):
        class Probe:
            page = 0

            def remaining(self):
                return 60

            def observe(self):
                return {}

            def call(self, name, args=None):
                if name == "minecraft_click_container_slot":
                    self.page += 1
                if name == "minecraft_read_item_components":
                    return {"components": {"minecraft:lore": [{"text": f"resident {self.page}"}]}}
                return {}

            def wait(self, predicate, label, **kwargs):
                slots = [{"slot": 9, "item_id": "minecraft:player_head", "name": str(self.page)}]
                if self.page == 0:
                    slots.append({"slot": 50, "name": "Next page"})
                state = {"screen": {"class": buildings_qa.CONTAINER_CLASS,
                                    "title": "Residents / households"},
                         "container": {"slots": slots}}
                assert predicate(state)
                return state

        self.assertEqual(buildings_qa._residents_text(Probe()), ["resident 0", "resident 1"])

    def test_assignment_does_not_hide_a_menu_failure(self):
        with mock.patch.object(buildings_qa, "_residents_text", side_effect=RuntimeError("menu failed")):
            with self.assertRaisesRegex(RuntimeError, "menu failed"):
                buildings_qa._await_assignment(mock.Mock(), 7, "Farmer workshop", "Farmer")


class FakeProbe:
    """Enough of ProbeSteps for the order and admission helpers; check raises like the real one."""

    def __init__(self, chat=()):
        self.chat = list(chat)
        self.report = {"id": "buildings-qa", "assertions": {}}
        self.waits = 0
        self.sent = 0
        self.deadline = 1e12
        self.client = mock.sentinel.client

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


class WorkerSettlementTest(unittest.TestCase):
    def test_miner_uses_tracked_building_and_own_settlement_menu(self):
        probe = FakeProbe()
        sub = {"site": [128, 70, 0], "building_id": (2 << 32) | 2}
        with mock.patch.object(buildings_qa, "_at_settlement_anchor") as anchor, \
                mock.patch.object(buildings_qa, "_building_seq", side_effect=AssertionError("stale lookup")), \
                mock.patch.object(buildings_qa, "_await_assignment", side_effect=LookupError) as census:
            with self.assertRaises(LookupError):
                buildings_qa._function_mine(probe, sub, "buildings_mine", 2, (113, 69, -13), {})
        anchor.assert_called_once_with(probe, sub)
        census.assert_called_once_with(probe, 2, "Mine", "Miner")


class CameraCaptureTest(unittest.TestCase):
    def test_elevated_support_is_removed_even_if_capture_fails(self):
        for capture_error in (None, RuntimeError("capture failed")):
            with self.subTest(capture_error=capture_error):
                probe, sub = mock.Mock(), {}
                with mock.patch.object(buildings_qa, "_fit_orientation", return_value=0), \
                        mock.patch.object(buildings_qa, "oriented_cells",
                                          return_value={(10, 70, 20): "stone", (12, 72, 24): "stone"}), \
                        mock.patch.object(buildings_qa, "_read", return_value={"block_id": "minecraft:air"}), \
                        mock.patch.object(buildings_qa, "_close_any_screen"), \
                        mock.patch.object(buildings_qa, "_at_settlement_anchor") as anchor, \
                        mock.patch.object(buildings_qa.native_client, "capture",
                                          side_effect=capture_error, return_value={"retained_path": "image.png"}):
                    if capture_error:
                        with self.assertRaisesRegex(RuntimeError, "capture failed"):
                            buildings_qa._capture_level(probe, sub, None, "buildings_gate", "gate", 2, (10, 70, 20))
                    else:
                        buildings_qa._capture_level(probe, sub, None, "buildings_gate", "gate", 2, (10, 70, 20))
                        self.assertEqual(sub["L2_visual_capture"]["retained_path"], "image.png")
                commands = [call.args[0] for call in probe.fixture.call_args_list]
                self.assertIn("setblock 2 79 12 minecraft:barrier", commands)
                self.assertEqual(commands[-1], "setblock 2 79 12 minecraft:air")
                probe.call.assert_any_call("minecraft_wait_ticks", {"ticks": 240})
                anchor.assert_called_once_with(probe, sub)


class OrderEchoRetryTest(unittest.TestCase):
    def run_retry(self, lines, project_active=False, ctx=HEADLESS):
        probe, sub = FakeProbe(), {}

        def send(_probe, command, prefixes=()):
            probe.sent += 1
            return {"recent_chat": list(lines)}

        with mock.patch.object(buildings_qa, "_send_fresh", send), \
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

    def test_refusal_without_target_project_is_not_admitted(self):
        result, probe, _, active = self.run_retry(
            ["No blueprint upgrade available for this building."], project_active=False)
        self.assertFalse(result)
        self.assertEqual(probe.sent, 1)
        active.assert_called_once()

    def test_busy_does_not_prove_an_unrelated_project_is_ours(self):
        result, _, _, active = self.run_retry(["Busy."])
        self.assertFalse(result)
        active.assert_called_once()


class AdmissionTest(unittest.TestCase):
    def test_an_admitted_order_needs_no_cert(self):
        probe = FakeProbe()
        with mock.patch.object(buildings_qa, "_current_certificate") as certificate:
            self.assertIsNone(buildings_qa._admitted_or_caught(
                probe, {}, "buildings_gate", "gate", 3, True))
        certificate.assert_not_called()
        self.assertTrue(probe.report["assertions"]["buildings_gate_L3_project"])

    def test_a_refused_order_is_fine_when_the_planner_already_certified_the_level(self):
        probe = FakeProbe()
        with mock.patch.object(buildings_qa, "_current_certificate", return_value=(7, (1, 2, 3))):
            caught = buildings_qa._admitted_or_caught(
                probe, {}, "buildings_gate", "gate", 3, False)
        self.assertEqual(caught, (1, 2, 3))
        self.assertTrue(probe.report["assertions"]["buildings_gate_L3_project"])

    def test_a_refused_order_without_a_cert_fails_the_project_check(self):
        probe = FakeProbe()
        with mock.patch.object(buildings_qa, "_current_certificate", return_value=None):
            with self.assertRaisesRegex(RuntimeError, "buildings_gate_L3_project"):
                buildings_qa._admitted_or_caught(probe, {}, "buildings_gate", "gate", 3, False)


class ForeignBlockTest(unittest.TestCase):
    def test_the_project_origin_is_the_map_icon_lore(self):
        class Probe(FakeProbe):
            def call(self, name, arguments=None):
                if name == "minecraft_read_item_components":
                    return {"components": {"minecraft:lore": [{"text": "122 69 -5"}]}}
                return super().call(name, arguments)

        page = {"container": {"slots": [{"slot": buildings_qa.PROJECT_MAP_SLOT}]}}
        with mock.patch.object(buildings_qa, "_project_page", return_value=page):
            self.assertEqual(buildings_qa._project_origin(Probe()), (122, 69, -5))
        with mock.patch.object(buildings_qa, "_project_page", return_value={"container": {"slots": []}}):
            self.assertIsNone(buildings_qa._project_origin(Probe()))

    def test_the_corner_posts_follow_the_rotation_of_the_placement_frame(self):
        posts = buildings_qa._corner_posts((100, 69, 200))
        self.assertIn(((100, 70, 200), 0), posts)
        for turns in range(4):
            cells = [pos for pos, t in posts if t == turns]
            self.assertEqual(len(cells), 4, turns)
            self.assertTrue(all(pos[1] == 70 for pos in cells))
        # The 7 x 7 first level does not fill the 11 x 7 frame: another rotation moves its posts.
        self.assertNotEqual({pos for pos, t in posts if t == 0}, {pos for pos, t in posts if t == 1})

    def replaced(self, final):
        probe, sub = FakeProbe(), {"broken_cell": {}}
        with mock.patch.object(buildings_qa, "_read", return_value={"block": {"block_id": final}}):
            buildings_qa._broken_replaced(probe, sub, "buildings_house", (122, 70, -5))
        return sub

    def test_the_authored_block_at_the_certificate_passes(self):
        sub = self.replaced("minecraft:oak_log")
        self.assertEqual(sub["behaviour"]["broken_cell_replaced"], "PASS")

    def test_a_certificate_with_the_cell_still_broken_fails(self):
        with self.assertRaisesRegex(RuntimeError, "buildings_house_broken_cell_replaced"):
            self.replaced("minecraft:air")


class SequentialFlowTest(unittest.TestCase):
    """`_scenario_sequential` for one blueprint with every game-facing helper replaced."""

    def run_flow(self, bp, top, order_admitted, cert_after_order, windowed=False, menu_evidence=(1, 2, 3),
                 order_echo=None):
        probe = FakeProbe()
        detail: dict = {}
        certs = {}
        events = []
        stages = []
        self.calls = []

        def current(_probe, sub, tag, _bp, level):
            if level not in certs:
                return None
            sub["building_id"] = 7
            return 7, certs[level]

        def order(_probe, sub, tag, _bp, level, cmd, use_dialog, ctx, dialog_fn=None,
                  upgrade_bid=None):
            events.append(("order", level, use_dialog))
            stages.append(("order", level))
            if order_echo is not None:
                sub[f"L{level}_order_attempt1"] = order_echo
            if use_dialog and level in menu_evidence:
                sub[f"L{level}_dialog_{'order' if level == 1 else 'upgrade'}"] = "confirmed"
            if cert_after_order:
                certs[level] = (level, 70, level)
            return order_admitted

        def break_corner(_probe, sub, tag, _bp):
            self.calls.append("corner")
            return (4, 71, 0)

        def broken_replaced(_probe, sub, tag, corner):
            self.calls.append(("replaced", corner))

        def wait_cert(_probe, sub, tag, _bp, level, ctx, artifact):
            if level == 1:
                self.calls.append("cert")
            certs[level] = (level, 70, level)
            sub["building_id"] = 7
            sub[f"L{level}_origin"] = list(certs[level])
            return certs[level]

        def fund(_probe, sub, _bp, level, chest):
            stages.append(("fund", level))
            sub.setdefault("behaviour", {})[f"L{level}_funded"] = "PASS"

        ctx = SimpleNamespace(windowed=windowed)
        patches = dict(
            _send=lambda _p, command, settle_ticks=40: {
                "recent_chat": ["anchor=[0, 70, 0]"] if "info" in command else []},
            _echo=lambda _p, label: {"recent_chat": [f"Founded bqa_{bp}"]},
            _hand_storage_at=lambda *a, **k: (5, 70, 5),
            _summon_bind=lambda *a, **k: True,
            _fund_level=fund,
            _order_echo_retry=order,
            _building_records=lambda *a, **k: {},
            _current_certificate=current,
            _at_settlement_anchor=lambda *a, **k: None,
            _queue_planner_fence=lambda _p, _s, _t, _b, level: stages.append(("fence", level)),
            _adversarial_break=lambda *a, **k: False,
            _break_placed_house_cell=lambda *a, **k: None,
            _check_house_repair=lambda *a, **k: None,
            _wait_certified_origin_here=wait_cert,
            _break_corner=break_corner,
            _broken_replaced=broken_replaced,
            _verify_level=lambda _p, _s, _c, _a, _b, level, *_args: stages.append(("verify", level)),
        )
        self.last_probe = probe
        try:
            with mock.patch.multiple(buildings_qa, **patches), \
                    mock.patch.object(buildings_qa, "_capture_level",
                                      side_effect=lambda _p, _s, _a, _t, _b, level, _o:
                                      stages.append(("capture", level))) as capture:
                buildings_qa._scenario_sequential(probe, detail, ctx, None, (bp,), "", None, None)
            self.last_sub = detail["blueprints"][bp]
            if windowed:
                self.assertEqual([call.args[5] for call in capture.call_args_list],
                                 list(range(1, top + 1)))
            expected = [("fence", 1)]
            for level in range(1, top + 1):
                expected.extend([("fund", level), ("order", level), ("fence", level)])
                if windowed:
                    expected.append(("capture", level))
                expected.append(("verify", level))
            self.assertEqual(stages, expected)
        except RuntimeError:
            self.last_sub = detail["blueprints"][bp]
            raise
        return probe, detail["blueprints"][bp], events

    def test_a_passing_run_records_every_level_verdict(self):
        probe, sub, events = self.run_flow("gate", 3, order_admitted=True, cert_after_order=False)
        for name in ("L1_built", "L2_upgrade", "L3_upgrade"):
            self.assertEqual(sub["behaviour"].get(name), "PASS", name)
        self.assertEqual([level for _, level, _ in events], [1, 2, 3])
        self.assertTrue(probe.report["assertions"]["buildings_gate_L3_project"])

    def test_headless_orders_do_not_use_graphical_menus(self):
        for bp, top in (("barracks", 3), ("house", 3)):
            _, _, events = self.run_flow(bp, top, order_admitted=True, cert_after_order=False)
            self.assertEqual({dialog for _, _, dialog in events}, {False})

    def test_the_house_corner_is_broken_before_the_certificate_and_checked_after(self):
        # SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN: the break lands while the
        # project is open and the authored block is read only once the certificate is listed.
        self.run_flow("house", 3, order_admitted=True, cert_after_order=False)
        self.assertEqual(self.calls, ["corner", "cert", ("replaced", (4, 71, 0))])
        self.run_flow("gate", 3, order_admitted=True, cert_after_order=False)
        self.assertEqual(self.calls, ["cert"])

    def test_a_workshop_records_its_level_two_verdict(self):
        _, sub, _ = self.run_flow("armorer", 2, order_admitted=True, cert_after_order=False)
        self.assertEqual(sub["behaviour"].get("L2_upgrade"), "PASS")
        self.assertNotIn("L3_upgrade", sub["behaviour"])

    def test_an_order_the_planner_beat_is_followed_not_failed(self):
        # buildings-qa 2026-11-13, gate L3: the server answered "No blueprint upgrade available"
        # because the planner had already built L3; the failed project check ended the run.
        probe, sub, _ = self.run_flow("gate", 3, order_admitted=False, cert_after_order=True)
        self.assertEqual(sub["behaviour"].get("L3_upgrade"), "PASS")
        self.assertEqual(sub.get("L3_admission"), "same-run certificate")
        self.assertTrue(probe.report["assertions"]["buildings_gate_L3_project"])

    def test_an_order_nobody_admitted_still_fails(self):
        with self.assertRaisesRegex(RuntimeError, "buildings_gate_L1_project"):
            self.run_flow("gate", 3, order_admitted=False, cert_after_order=False)

    def test_an_unknown_blueprint_refusal_fails_l1_built_as_an_ordinary_check(self):
        # ISSUE-1-16 negative control: with the wall blueprint unregistered the server
        # answers "Unknown blueprint."; the gate must record L1_built=FAIL and trip the
        # buildings_wall_L1_built check, never die with an unexpected exception.
        # Red without the fix: the run then aborts at buildings_wall_L1_project instead.
        with self.assertRaisesRegex(RuntimeError, "buildings_wall_L1_built"):
            self.run_flow("wall", 3, order_admitted=False, cert_after_order=False,
                          order_echo="Unknown blueprint.")
        self.assertEqual(self.last_sub["behaviour"].get("L1_built"), "FAIL")
        self.assertIn("Unknown blueprint.", self.last_sub["evidence"].get("L1_built", ""))
        self.assertFalse(self.last_probe.report["assertions"]["buildings_wall_L1_built"])

    def test_missing_level_is_red_only_after_the_server_refuses_the_upgrade(self):
        data = buildings_qa._load_blueprints()
        truncated = dict(data)
        truncated["gate"] = dict(data["gate"], levels=data["gate"]["levels"][:2])
        probe = FakeProbe()
        with mock.patch.object(buildings_qa, "_load_blueprints", return_value=truncated), \
                mock.patch(f"{__name__}.FakeProbe", return_value=probe), \
                mock.patch.object(buildings_qa, "_send_fresh", return_value={
                    "recent_chat": ["No blueprint upgrade available for this building."]}) as send:
            with self.assertRaisesRegex(RuntimeError, "buildings_gate_L3_upgrade_available"):
                self.run_flow("gate", 3, order_admitted=True, cert_after_order=False)
        self.assertEqual(send.call_args.args[1], "settlement build upgrade 7")
        self.assertIs(probe.report["assertions"]["buildings_gate_L3_upgrade_available"], False)

    def test_verifying_previous_level_does_not_index_a_missing_next_level(self):
        data = {"gate": {"levels": [{}, {}]}}
        with mock.patch.object(buildings_qa, "_load_blueprints", return_value=data), \
                mock.patch.object(buildings_qa, "_fit_orientation", return_value=0), \
                mock.patch.object(buildings_qa, "oriented_cells",
                                  return_value={(0, 0, 0): "minecraft:stone"}), \
                mock.patch.object(buildings_qa, "true_origin", return_value=(0, 0, 0)), \
                mock.patch.object(buildings_qa, "cells_true") as next_cells, \
                mock.patch.object(buildings_qa, "_read", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                buildings_qa._verify_level(FakeProbe(), {}, HEADLESS, None, "gate", 2,
                                           (0, 0, 0), "buildings_gate", "town", (0, 0, 0))
        next_cells.assert_not_called()

    def test_missing_blueprint_is_red_only_after_the_server_refuses_the_order(self):
        data = dict(buildings_qa._load_blueprints())
        del data["gate"]
        probe = FakeProbe()
        with mock.patch.object(buildings_qa, "_load_blueprints", return_value=data), \
                mock.patch(f"{__name__}.FakeProbe", return_value=probe), \
                mock.patch.object(buildings_qa, "_send_fresh",
                                  return_value={"recent_chat": ["Unknown blueprint."]}) as send:
            with self.assertRaisesRegex(RuntimeError, "buildings_gate_L1_order_available"):
                self.run_flow("gate", 3, order_admitted=True, cert_after_order=False)
        self.assertEqual(send.call_args.args[1], "settlement build gate")
        self.assertIs(probe.report["assertions"]["buildings_gate_L1_order_available"], False)


class SplitPartsTest(unittest.TestCase):
    """`--parts N` cuts the blueprint list into independent child gate parts."""

    def test_selected_targets_keeps_all_and_accepts_a_comma_group(self):
        self.assertEqual(buildings_qa.selected_targets(None), buildings_qa.BLUEPRINTS)
        self.assertEqual(buildings_qa.selected_targets("ALL"), buildings_qa.BLUEPRINTS)
        self.assertEqual(buildings_qa.selected_targets("house"), ("house",))
        self.assertEqual(buildings_qa.selected_targets("house, storage"), ("house", "storage"))
        self.assertEqual(buildings_qa.selected_targets(buildings_qa.PERIMETER),
                         (buildings_qa.PERIMETER,))

    def test_selected_targets_rejects_unknown_repeats_and_mixed_perimeter(self):
        for value in ("house,nope", "house,house", "ALL,house",
                      f"{buildings_qa.PERIMETER},house", ""):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    buildings_qa.selected_targets(value)

    def test_groups_cover_every_selected_target_exactly_once(self):
        for count in (len(buildings_qa.BLUEPRINTS), 5, 3, 2, 1):
            for parts in (1, 2, 3, 4, 8):
                with self.subTest(count=count, parts=parts):
                    targets = buildings_qa.BLUEPRINTS[:count]
                    groups = buildings_qa.split_groups(targets, parts)
                    self.assertLessEqual(len(groups), max(1, min(parts, count)))
                    self.assertEqual([bp for group in groups for bp in group], list(targets))
                    self.assertTrue(all(groups))
                    self.assertTrue(all(1 <= len(group) for group in groups))

    def block(self, label, blueprints, status="passed"):
        return {"label": label, "status": status, "artifact": f"/a/{label}",
                "log": f"/a/{label}.log", "exit": 0,
                "details": {"buildings_acceptance": {"blueprints": blueprints,
                                                     "behaviour": {f"{label}_ok": "PASS"}}}}

    def house(self, level=1, verdict="PASS"):
        return {"behaviour": {f"L{level}_blockmatch": verdict,
                              f"L{level}_entrance": verdict,
                              f"L{level}_light": verdict}}

    def test_the_merged_matrix_requires_every_blueprint_of_every_part(self):
        blocks = [self.block("part1", {"house": self.house(1)}),
                  self.block("part2", {"storage": self.house(1, "FAIL")})]
        merged = buildings_qa.merge_split_parts(blocks, ("house", "storage"))["scenario"]["buildings_acceptance"]
        self.assertEqual(merged["mode"], "split-independent-parts")
        self.assertEqual(merged["matrix_19"]["house L1"], "PASS")
        self.assertEqual(merged["matrix_19"]["storage L1"], "FAIL")
        self.assertEqual(merged["matrix_19"]["barracks L1"], "NOT-RUN")
        self.assertEqual([part["label"] for part in merged["parts"]], ["part1", "part2"])
        self.assertEqual(merged["behaviour"], {"part1_ok": "PASS", "part2_ok": "PASS"})

    def test_a_blueprint_no_part_reported_fails_the_merge(self):
        blocks = [self.block("part1", {"house": self.house(1)})]
        with self.assertRaisesRegex(RuntimeError, "did not cover: storage"):
            buildings_qa.merge_split_parts(blocks, ("house", "storage"))

    def test_two_parts_claiming_one_blueprint_fail_the_merge(self):
        blocks = [self.block("part1", {"house": self.house(1)}),
                  self.block("part2", {"house": self.house(2)})]
        with self.assertRaisesRegex(RuntimeError, "house reported by parts part1 and part2"):
            buildings_qa.merge_split_parts(blocks, ("house",))

    def test_a_part_child_runs_its_own_selection_instead_of_splitting(self):
        ctx = SimpleNamespace(client="native", extra_args=[], platform=None,
                             timeout_seconds=600.0, parts=4,
                             artifact_dir=Path(tempfile.mkdtemp()))
        with mock.patch.dict("os.environ", {"BUILDINGS_QA_SELECT": "house,storage",
                                            profiles.SPLIT_PART_ENV: "part2",
                                            "KRASNOKAMEN_SERVER_BINARY": ""}), \
                mock.patch.object(buildings_qa, "run_split") as split, \
                mock.patch.object(buildings_qa, "_load_blueprints", return_value={}):
            with self.assertRaisesRegex(RuntimeError, "KRASNOKAMEN_SERVER_BINARY"):
                buildings_qa.run(ctx)
        split.assert_not_called()

    def test_the_parent_cuts_the_selection_into_children_and_merges_them(self):
        ctx = SimpleNamespace(client="native", extra_args=[], platform=None,
                             timeout_seconds=600.0, parts=2,
                             artifact_dir=Path(tempfile.mkdtemp()))
        blocks = [self.block("part1", {"house": self.house(1)}),
                  self.block("part2", {"storage": self.house(1)})]
        with mock.patch.dict("os.environ", {"BUILDINGS_QA_SELECT": "house,storage"}), \
                mock.patch.dict("os.environ", {profiles.SPLIT_PART_ENV: ""}), \
                mock.patch.object(buildings_qa, "run_split", return_value=blocks) as split:
            report = buildings_qa.run(ctx)
        parts = split.call_args.args[2]
        self.assertEqual([(part.label, part.env["BUILDINGS_QA_SELECT"]) for part in parts],
                         [("part1", "house"), ("part2", "storage")])
        self.assertEqual(split.call_args.args[1], "buildings-qa")
        matrix = report["scenario"]["buildings_acceptance"]["matrix_19"]
        self.assertEqual(matrix["house L1"], "PASS")
        self.assertEqual(matrix["storage L1"], "PASS")
        self.assertEqual(matrix["barracks L1"], "NOT-RUN")


if __name__ == "__main__":
    unittest.main()
