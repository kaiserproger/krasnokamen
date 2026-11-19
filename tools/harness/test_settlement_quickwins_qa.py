"""Pure checks of the settlement quick-wins acceptance: screen parsing and the text verdicts."""
from __future__ import annotations

import unittest

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


if __name__ == "__main__":
    unittest.main()
