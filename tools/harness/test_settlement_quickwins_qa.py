"""Pure checks of the settlement quick-wins acceptance: screen parsing and the text verdicts."""
from __future__ import annotations

import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness.backends import settlement_quickwins_qa as gate

GOOD_GUIDE = """Поселения — быстрый старт:
1. Основать поселение: /settlement create Имя; управление — /settlement.
2. Сначала склад: наполните пустой сундук у центра, стаки переносите Shift+клик.
3. Строительство: выберите чертёж и закажите, недостающее принесите на склад.
4. Построили сами? Сертифицируйте: встаньте на пол в два угла.
5. Жители и работа: назначьте жителей на рабочие блоки.
6. «Следующий шаг» в колоколе подскажет, что делать дальше."""


class ViewTests(unittest.TestCase):
    def test_dialog_view_reads_body_and_buttons(self):
        state = {"screen": {"class": "net.minecraft.client.gui.screens.dialog.MultiButtonDialogScreen",
                            "open": True, "title": "Поселение qw"},
                 "ui_nodes": [
                     {"id": "dialog/title", "role": "text", "label": "Поселение qw", "enabled": True},
                     {"id": "dialog/body/0", "role": "text", "label": "владелец", "enabled": True},
                     {"id": "dialog/action/0", "role": "button", "label": "Управление", "enabled": True,
                      "bounds": [100.0, 600.0, 450.0, 60.0]},
                     {"id": "dialog/action/2", "role": "button", "label": "Ниже окна", "enabled": True,
                      "bounds": [100.0, 700.0, 450.0, 60.0]},
                     {"id": "dialog/action/1", "role": "button", "label": "Закрыть", "enabled": False}]}
        shown = gate.view(state)
        self.assertEqual((shown["kind"], shown["title"], shown["body"], shown["buttons"], shown["ready"]),
                         ("dialog", "Поселение qw", ["владелец"], ["Управление"], True))

    def test_menu_view_needs_the_synced_chest(self):
        state = {"screen": {"class": gate.CONTAINER, "open": True, "title": "Поселение"},
                 "container": {"slot_count": 90, "slots": [
                     {"slot": 10, "name": "Карта", "item_id": "minecraft:filled_map"},
                     {"slot": 70, "name": "inventory", "item_id": "minecraft:stone"}]}}
        shown = gate.view(state)
        self.assertEqual((shown["kind"], shown["slots"], shown["ready"]), ("menu", {10: "Карта"}, True))
        state["container"]["slots"] = []
        self.assertFalse(gate.view(state)["ready"])

    def test_no_screen_is_not_ready(self):
        self.assertEqual(gate.view({"screen": {"open": False}})["kind"], "none")
        self.assertFalse(gate.view({})["ready"])


class VerdictTests(unittest.TestCase):
    def test_guide(self):
        ok, facts = gate.guide_verdict(GOOD_GUIDE)
        self.assertTrue(ok, facts)
        old = "Settlements / Поселения — quick start:\n" + "\n".join(f"{i}. composter=Farmer" for i in range(1, 10))
        self.assertFalse(gate.guide_verdict(old)[0])
        self.assertFalse(gate.guide_verdict(GOOD_GUIDE.replace("Сначала склад", "Сначала сундук"))[0])  # task order needs склад

    def test_guide_rejects_wrong_order_and_length(self):
        shuffled = "\n".join(reversed(GOOD_GUIDE.splitlines()))
        self.assertFalse(gate.guide_verdict(shuffled)[0])
        self.assertFalse(gate.guide_verdict(GOOD_GUIDE + "\n" + "7. Ещё шаг.\n8. И ещё.")[0])

    def test_greeting(self):
        self.assertTrue(gate.greeting_verdict(["Поселения: /settlement guide"])[0])
        self.assertFalse(gate.greeting_verdict(["Settlements / Поселения: /settlement guide"])[0])
        self.assertFalse(gate.greeting_verdict([])[0])
        self.assertFalse(gate.greeting_verdict(["Поселения: /settlement guide", "Поселения: /settlement guide"])[0])

    ESTIMATE = ("Булыжник ×43 · на складе 1\nДубовые доски ×106 · на складе 0\nДубовое бревно ×36 · на складе 0\n"
                "Факел ×3 · на складе 0\nСундук ×1 · на складе 0\nВерстак ×1 · на складе 0\n"
                "Печь ×1 · на складе 0\nДубовый забор ×9 · на складе 0\nДубовые ступеньки ×44 · на складе 0\n"
                "Дубовая нажимная пластина ×1 · на складе 0\nДубовая дверь ×1 · на складе 0\nБелая кровать ×1 · на складе 0")

    def test_expected_shortage_from_estimate(self):
        expected = gate.expected_shortage(self.ESTIMATE)
        self.assertEqual(len(expected), 12)
        self.assertEqual(expected["Булыжник"], 42)

    def test_shortage(self):
        expected = gate.expected_shortage(self.ESTIMATE)
        good = ("Заказ принят: Дом.\nВ очереди #1: Дом ур. 1 · не хватает материалов\n"
                "Булыжник ×42, Дубовые доски ×106, Дубовое бревно ×36 и ещё 9 видов")
        ok, facts = gate.shortage_verdict(good, ["Склад", "Статус", "Назад"], expected)
        self.assertTrue(ok, facts)
        self.assertFalse(gate.shortage_verdict(good, ["Статус"], expected)[0])  # no link to storage

    def test_old_outcome_text_fails(self):
        """The false pass: «#1», «ур. 1» are digits but not materials; the item list is what counts."""
        expected = gate.expected_shortage(self.ESTIMATE)
        old = "Заказ принят: Дом. В очереди #1: Дом ур. 1 · не хватает материалов Положите недостающие материалы на склад."
        ok, facts = gate.shortage_verdict(old, ["Склад", "Статус", "Назад"], expected)
        self.assertFalse(ok)
        self.assertFalse(facts["items_exact"])
        self.assertFalse(gate.shortage_verdict("Queued Дом: not enough: minecraft:cobblestone x43", ["Склад"], expected)[0])

    def test_shortage_rules(self):
        expected = gate.expected_shortage(self.ESTIMATE)
        head = "Заказ принят: Дом. Не хватает: "
        links = ["Склад"]
        self.assertFalse(gate.shortage_verdict(head + "Булыжник ×43, Дубовые доски ×106 и ещё 10 видов", links, expected)[0])  # wrong count
        self.assertFalse(gate.shortage_verdict(head + "Булыжник ×42, Дубовые доски ×106", links, expected)[0])  # no «и ещё»
        self.assertFalse(gate.shortage_verdict(head + "Булыжник ×42, Дубовые доски ×106 и ещё 3 видов", links, expected)[0])  # wrong N
        four = head + "Булыжник ×42, Дубовые доски ×106, Дубовое бревно ×36, Факел ×3 и ещё 8 видов"
        self.assertFalse(gate.shortage_verdict(four, links, expected)[0])  # more than three
        self.assertFalse(gate.shortage_verdict(head + "Гравий ×42 и ещё 11 видов", links, expected)[0])  # not a missing item
        small = gate.expected_shortage("Булыжник ×2 · на складе 0")
        self.assertTrue(gate.shortage_verdict(head + "Булыжник ×2", links, small)[0])  # all listed: no «и ещё»

    def test_refusal(self):
        self.assertTrue(gate.refusal_verdict("Не хватает: кровати 0/1, крыша 0/1")[0])  # compact visible text
        self.assertTrue(gate.refusal_verdict("Не удалось сертифицировать Дом. Не хватает: кровати 0/1 — поставьте, крыша 0/1.")[0])
        self.assertFalse(gate.refusal_verdict("Не хватает: кровати 0/1")[0])  # roof cut off (the 79-char truncation)
        self.assertFalse(gate.refusal_verdict("Не хватает: кровати, крыша")[0])  # no counts
        bad = "Не удалось сертифицировать Дом. Не хватает: beds 0/1, roof 0/1."
        self.assertFalse(gate.refusal_verdict(bad)[0])
        self.assertEqual(gate.refusal_verdict(bad)[1]["jargon"], ["beds", "roof"])


class RegistryTests(unittest.TestCase):
    def test_every_criterion_cites_its_source(self):
        for cid, (what, source) in gate.CRITERIA.items():
            self.assertTrue(what and source, cid)

    def test_blocks_cover_every_criterion_once(self):
        from tools.harness.backends import settlement_quickwins_steps as steps
        owned = [cid for _, ids, _ in steps.BLOCKS for cid in ids]
        self.assertEqual(sorted(owned), sorted(gate.CRITERIA))


class C1RaceTests(unittest.TestCase):
    """`--scenario quickwins-c1-race`: the standing check of JAVA-CLIENT-MISSES-FOUNDING-STARTER-CHEST-BLOCK."""

    def test_options_default_to_four_clocks_and_100_ticks(self):
        self.assertEqual(gate.c1_race_options(gate.C1_RACE_SCENARIO, None, None), gate.C1Race(4, 100))
        self.assertEqual(gate.c1_race_options(gate.C1_RACE_SCENARIO, 2, 40), gate.C1Race(2, 40))
        self.assertEqual(gate.c1_race_options(gate.C1_RACE_SCENARIO, 6, 1200), gate.C1Race(6, 1200))
        self.assertEqual(gate.c1_race_options(gate.C1_RACE_SCENARIO, 1, 1), gate.C1Race(1, 1))

    def test_options_outside_their_ranges_are_refused(self):
        for clocks, ticks in ((0, None), (7, None), (-1, None), (None, 0), (None, 1201), (None, -5)):
            with self.assertRaisesRegex(ValueError, "must be", msg=(clocks, ticks)):
                gate.c1_race_options(gate.C1_RACE_SCENARIO, clocks, ticks)

    def test_options_belong_to_the_race_scenario_only(self):
        for scenario in ("civil", "quickwins", "maplabel"):
            self.assertIsNone(gate.c1_race_options(scenario, None, None))
            for clocks, ticks in ((4, None), (None, 100)):
                with self.assertRaisesRegex(ValueError, "belong to --scenario quickwins-c1-race"):
                    gate.c1_race_options(scenario, clocks, ticks)

    def test_clocks_stay_in_the_anchor_chunk_away_from_the_chest_sites(self):
        # The layout of the recorded control and fix runs (receipts/ChestDeliveryFix): anchor (128,201,0).
        self.assertEqual(gate.c1_race_clocks([128, 201, 0], 4),
                         [([140, 201, z], [141, 201, z]) for z in (2, 4, 6, 8)])
        for anchor in ([128, 201, 0], [-3, 70, 13], [7, 64, 7], [8, 64, 8], [-16, 64, -1], [15, 90, 15]):
            pairs = gate.c1_race_clocks(anchor, 6)
            cells = [cell for pair in pairs for cell in pair]
            self.assertEqual(len({tuple(cell) for cell in cells}), 12, anchor)
            for east, west in pairs:
                self.assertEqual(west, [east[0] + 1, east[1], east[2]], anchor)
            for cell in cells:
                self.assertEqual((cell[0] // 16, cell[2] // 16), (anchor[0] // 16, anchor[2] // 16), (anchor, cell))
                self.assertEqual(cell[1], anchor[1])
                self.assertGreaterEqual(abs(cell[0] - anchor[0]), 5, (anchor, cell))  # never a chest site

    def test_settlement_map_qa_validates_the_race_arguments_before_any_client_starts(self):
        from types import SimpleNamespace
        from tools.harness.backends import settlement_map_qa
        for extra, message in ((["--", "--founding-clocks", "4"], "belong to"),
                               (["--", "--scenario", "quickwins", "--found-after-ticks", "10"], "belong to"),
                               (["--", "--scenario", "quickwins-c1-race", "--founding-clocks", "9"], "must be"),
                               (["--", "--scenario", "quickwins-c1-race", "--found-after-ticks", "0"], "must be"),
                               (["--", "--scenario", "quickwins-c1-race", "--survey"], "--survey")):
            ctx = SimpleNamespace(client="java", extra_args=extra, client_evidence={}, parts=1, logs=[])
            with patch.object(gate.scene_shot, "run", side_effect=AssertionError("a client started")):
                with self.assertRaisesRegex(ValueError, message, msg=extra):
                    settlement_map_qa.run(ctx)

    def test_the_ledger_ref_runs_the_race_on_vanilla_java(self):
        from tools.harness import regressions
        from tools.harness.backends import settlement_map_qa
        command = regressions._profile_command("settlement-map-qa", gate.C1_RACE_VERDICT)
        self.assertEqual(command[-8:], ["--client", "java", "--windowed", "--timeout-seconds", "3600",
                                        "--", "--scenario", gate.C1_RACE_SCENARIO])
        self.assertEqual(regressions._BOOLEAN_PROFILE_VERDICTS["settlement-map-qa"],
                         {*settlement_map_qa.VERDICTS, *settlement_map_qa.SCENARIO_VERDICTS,
                          gate.C1_RACE_VERDICT})

    def race_run(self, block=None, setup=None):
        """Run `--scenario quickwins-c1-race` with the client, the GUI and the walk's fixtures replaced."""
        from types import SimpleNamespace
        from tools.harness.backends import settlement_quickwins_steps as steps

        def check(name, ok):
            probe.report["assertions"][name] = bool(ok)
            if not ok:
                raise RuntimeError(f"scene-shot assertion failed: {name}")
        probe = SimpleNamespace(report={"assertions": {}, "fixture_commands": []}, check=check,
                                client=SimpleNamespace(call_tool=lambda name, arguments=None: {}))
        ctx = SimpleNamespace(client="java", client_evidence={}, logs=[])
        args = SimpleNamespace(scenario=gate.C1_RACE_SCENARIO, c1_race=gate.C1Race(3, 50), survey=False,
                               language="en_us", gui_scale=3)
        seen = []

        def fake_setup(walk, race):
            seen.append(race)
            if setup:
                setup(walk)

        def scene(ctx_, scenario, script):
            return script(probe, Path("/nonexistent"), [], "java")
        with patch.object(gate.scene_shot, "run", scene), patch.object(steps, "setup", fake_setup), \
                patch.object(steps, "block_create", block), \
                patch("tools.harness.backends.settlement_map_qa.Gui"):
            try:
                gate.run(ctx, args)
                error = None
            except Exception as caught:  # noqa: BLE001 - the test inspects what the run raised
                error = caught
        self.assertEqual(seen, [gate.C1Race(3, 50)])
        return ctx.client_evidence, error

    def test_a_seen_chest_publishes_true_and_passes(self):
        def found(walk):
            walk.judge("C1", True, chest=[129, 201, 0], anchor=[128, 201, 0])
        evidence, error = self.race_run(found)
        self.assertIsNone(error)
        self.assertEqual(evidence["verdicts"], {gate.C1_RACE_VERDICT: True})
        self.assertEqual(evidence["quickwins"]["criteria"], {"C1": True})

    def test_a_chest_seen_only_after_reentry_publishes_false_and_fails(self):
        def resent(walk):
            walk.results["C1"] = {"ok": False, "visible_only_after_reenter": [129, 201, 0]}
        evidence, error = self.race_run(resent)
        self.assertRegex(str(error), f"assertion failed: QW {gate.C1_RACE_VERDICT}")
        self.assertEqual(evidence["verdicts"], {gate.C1_RACE_VERDICT: False})

    def test_a_chest_never_seen_or_a_run_cut_short_is_no_verdict(self):
        def never(walk):
            walk.judge("C1", False, chest=None, anchor=[128, 201, 0])
        evidence, error = self.race_run(never)
        self.assertRegex(str(error), "assertion failed: QW C1")
        self.assertEqual(evidence["verdicts"], {gate.C1_RACE_VERDICT: None})

        def lost(walk):
            raise gate.StepFailed("the pad did not load")
        evidence, error = self.race_run(block=lambda walk: None, setup=lost)
        self.assertIsInstance(error, gate.StepFailed)
        self.assertEqual(evidence["verdicts"], {gate.C1_RACE_VERDICT: None})


if __name__ == "__main__":
    unittest.main()
