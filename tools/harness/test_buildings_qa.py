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


class FreshCertificateTest(unittest.TestCase):
    def test_old_started_reply_is_excluded_from_fresh_refusal(self):
        class ChatProbe(FakeProbe):
            def call(self, name, arguments=None):
                if name == "minecraft_send_chat":
                    message = arguments["message"]
                    self.chat.insert(0, "No blueprint upgrade available for this building."
                                     if arguments["command"] else f"<BuildQA> {message}")
                return {}

            def wait(self, predicate, label, **kwargs):
                state = self.observe()
                assert predicate(state), label
                return state

        probe = ChatProbe(["Started gate construction."])
        state = buildings_qa._send_fresh(probe, "settlement build upgrade 7")
        self.assertEqual(state["recent_chat"], ["No blueprint upgrade available for this building."])

    def test_packed_fresh_listing_keeps_kind_id_level_and_origin_together(self):
        state = {"recent_chat": ["Buildings: #10 storage level 1 [1, 2, 3]-[4, 5, 6], "
                                 "#11 gate level 2 [-8, 9, 10]-[0, 12, 18]."]}
        with mock.patch.object(buildings_qa, "_send_fresh", return_value=state):
            self.assertEqual(buildings_qa._building_records(FakeProbe(), "gate"),
                             {11: (2, (-8, 9, 10))})

    def test_existing_or_wrong_id_cannot_prove_this_run(self):
        probe = FakeProbe()
        sub = {"baseline_building_ids": [11], "building_id": 12}
        records = {11: (2, (1, 2, 3)), 13: (2, (4, 5, 6))}
        with mock.patch.object(buildings_qa, "_building_records", return_value=records):
            self.assertIsNone(buildings_qa._current_certificate(probe, sub, "buildings_gate", "gate", 2))

    def test_new_building_is_selected_over_a_baseline_building(self):
        probe = FakeProbe()
        sub = {"baseline_building_ids": [11]}
        records = {11: (1, (1, 2, 3)), 13: (1, (4, 5, 6))}
        with mock.patch.object(buildings_qa, "_building_records", return_value=records):
            self.assertEqual(buildings_qa._current_certificate(probe, sub, "buildings_gate", "gate", 1),
                             (13, (4, 5, 6)))
        self.assertEqual(sub["building_id"], 13)

    def test_overtaken_level_is_an_explicit_failure_not_a_higher_level_pass(self):
        probe = FakeProbe()
        sub = {"baseline_building_ids": [], "building_id": 12}
        with mock.patch.object(buildings_qa, "_building_records", return_value={12: (3, (1, 2, 3))}):
            with self.assertRaisesRegex(RuntimeError, "buildings_gate_L2_not_overtaken"):
                buildings_qa._current_certificate(probe, sub, "buildings_gate", "gate", 2)
        self.assertIn("reached L3 before L2 verification", sub["L2_not_verified"])

    def test_unfundable_fence_must_be_queued_not_started(self):
        probe = FakeProbe()
        with mock.patch.object(buildings_qa, "_send_fresh",
                               return_value={"recent_chat": ["Started cleric construction."]}):
            with self.assertRaisesRegex(RuntimeError, "buildings_gate_L2_planner_fenced"):
                buildings_qa._queue_planner_fence(probe, {}, "buildings_gate", "gate", 2)

    def test_air_from_a_next_level_clear_is_not_a_current_level_match(self):
        data = {"gate": {"levels": [{}, {}]}}
        with mock.patch.object(buildings_qa, "_load_blueprints", return_value=data), \
                mock.patch.object(buildings_qa, "_fit_orientation", return_value=0), \
                mock.patch.object(buildings_qa, "oriented_cells", return_value={(0, 0, 0): "minecraft:stone"}), \
                mock.patch.object(buildings_qa, "_read", return_value={"block_id": "minecraft:air"}), \
                mock.patch.object(buildings_qa, "_send", return_value={"recent_chat": []}):
            with self.assertRaisesRegex(RuntimeError, "buildings_gate_L2_blocks"):
                buildings_qa._verify_level(FakeProbe(), {}, HEADLESS, None, "gate", 2,
                                           (0, 0, 0), "buildings_gate", "town", (0, 0, 0))

    def test_wall_torch_does_not_satisfy_a_cobblestone_wall(self):
        data = {"gate": {"levels": [{}, {}]}}
        with mock.patch.object(buildings_qa, "_load_blueprints", return_value=data), \
                mock.patch.object(buildings_qa, "_fit_orientation", return_value=0), \
                mock.patch.object(buildings_qa, "oriented_cells",
                                  return_value={(0, 0, 0): "minecraft:cobblestone_wall"}), \
                mock.patch.object(buildings_qa, "_read", return_value={"block_id": "minecraft:wall_torch"}), \
                mock.patch.object(buildings_qa, "_send", return_value={"recent_chat": []}):
            with self.assertRaisesRegex(RuntimeError, "buildings_gate_L2_blocks"):
                buildings_qa._verify_level(FakeProbe(), {}, HEADLESS, None, "gate", 2,
                                           (0, 0, 0), "buildings_gate", "town", (0, 0, 0))


class SequentialFlowTest(unittest.TestCase):
    """`_scenario_sequential` for one blueprint with every game-facing helper replaced."""

    def run_flow(self, bp, top, order_admitted, cert_after_order, windowed=False, menu_evidence=(1, 2, 3)):
        probe = FakeProbe()
        detail: dict = {}
        certs = {}
        events = []
        stages = []

        def current(_probe, sub, tag, _bp, level):
            if level not in certs:
                return None
            sub["building_id"] = 7
            return 7, certs[level]

        def order(_probe, sub, tag, _bp, level, cmd, use_dialog, ctx, dialog_fn=None,
                  upgrade_bid=None):
            events.append(("order", level, use_dialog))
            stages.append(("order", level))
            if use_dialog and level in menu_evidence:
                sub[f"L{level}_dialog_{'order' if level == 1 else 'upgrade'}"] = "confirmed"
            if cert_after_order:
                certs[level] = (level, 70, level)
            return order_admitted

        def wait_cert(_probe, sub, tag, _bp, level, ctx, artifact):
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
            _wait_certified_origin_here=wait_cert,
            _verify_level=lambda _p, _s, _c, _a, _b, level, *_args: stages.append(("verify", level)),
        )
        with mock.patch.multiple(buildings_qa, **patches), \
                mock.patch.object(buildings_qa, "_capture_level",
                                  side_effect=lambda _p, _s, _a, _t, _b, level, _o:
                                  stages.append(("capture", level))) as capture:
            buildings_qa._scenario_sequential(probe, detail, ctx, None, (bp,), "", None, None)
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

    def test_windowed_house_orders_each_level_through_the_menus(self):
        _, _, events = self.run_flow("house", 3, order_admitted=True,
                                     cert_after_order=False, windowed=True)
        self.assertEqual(events, [("order", 1, True), ("order", 2, True), ("order", 3, True)])

    def test_windowed_house_rejects_missing_order_or_upgrade_confirmation(self):
        for evidence, failure in (((2, 3), "buildings_house_L1_order_menu"),
                                  ((1,), "buildings_house_upgrade_menu")):
            with self.subTest(evidence=evidence), self.assertRaisesRegex(RuntimeError, failure):
                self.run_flow("house", 3, order_admitted=True, cert_after_order=False,
                              windowed=True, menu_evidence=evidence)

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

    def test_missing_blueprint_level_is_an_explicit_failed_assertion(self):
        data = buildings_qa._load_blueprints()
        truncated = dict(data)
        truncated["gate"] = dict(data["gate"], levels=data["gate"]["levels"][:2])
        probe = FakeProbe()
        with mock.patch.object(buildings_qa, "_load_blueprints", return_value=truncated), \
                mock.patch(f"{__name__}.FakeProbe", return_value=probe):
            with self.assertRaisesRegex(RuntimeError, "buildings_gate_L3_level_exists"):
                self.run_flow("gate", 3, order_admitted=True, cert_after_order=False)
        self.assertIs(probe.report["assertions"]["buildings_gate_L3_level_exists"], False)

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

    def test_missing_blueprint_is_an_explicit_failed_assertion(self):
        data = dict(buildings_qa._load_blueprints())
        del data["gate"]
        with mock.patch.object(buildings_qa, "_load_blueprints", return_value=data):
            with self.assertRaisesRegex(RuntimeError, "buildings_gate_L1_level_exists"):
                self.run_flow("gate", 3, order_admitted=True, cert_after_order=False)


if __name__ == "__main__":
    unittest.main()
