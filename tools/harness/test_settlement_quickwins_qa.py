"""Pure checks of the settlement quick-wins acceptance: screen parsing and the text verdicts."""
from __future__ import annotations

import unittest

from tools.harness.backends import settlement_quickwins_qa as gate

GOOD_GUIDE = """Поселения — быстрый старт:
1. Встаньте в центре и введите /settlement create Имя: в сундуке рядом хранится склад.
2. Откройте колокол → «Управление» → «Строительство» и закажите здание: строители возьмут материалы со склада.
3. Не хватает материалов? Положите их на склад: Shift+клик по своему стаку.
4. Построили сами? «Постройки» → «Сертифицировать»: встаньте на пол в два угла.
5. Назначьте жителей на работу: «Жители и работа».
6. «Следующий шаг» в колоколе подскажет, что делать дальше."""


class ViewTests(unittest.TestCase):
    def test_dialog_view_reads_body_and_buttons(self):
        state = {"screen": {"class": "net.minecraft.client.gui.screens.dialog.MultiButtonDialogScreen",
                            "open": True, "title": "Поселение qw"},
                 "ui_nodes": [
                     {"id": "dialog/title", "role": "text", "label": "Поселение qw", "enabled": True},
                     {"id": "dialog/body/0", "role": "text", "label": "владелец", "enabled": True},
                     {"id": "dialog/action/0", "role": "button", "label": "Управление", "enabled": True},
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
        self.assertFalse(gate.guide_verdict(GOOD_GUIDE.replace("склад", "хранилище"))[0])  # task order needs склад

    def test_guide_rejects_wrong_order_and_length(self):
        shuffled = "\n".join(reversed(GOOD_GUIDE.splitlines()))
        self.assertFalse(gate.guide_verdict(shuffled)[0])
        self.assertFalse(gate.guide_verdict(GOOD_GUIDE + "\n" + "7. Ещё шаг.\n8. И ещё.")[0])

    def test_greeting(self):
        self.assertTrue(gate.greeting_verdict(["Поселения: /settlement guide"])[0])
        self.assertFalse(gate.greeting_verdict(["Settlements / Поселения: /settlement guide"])[0])
        self.assertFalse(gate.greeting_verdict([])[0])
        self.assertFalse(gate.greeting_verdict(["Поселения: /settlement guide", "Поселения: /settlement guide"])[0])

    def test_shortage(self):
        text = "Заказ принят: Дом. Не хватает: 24 доски, 8 булыжника. Положите на склад."
        self.assertTrue(gate.shortage_verdict(text, ["Склад", "Статус"])[0])
        self.assertFalse(gate.shortage_verdict(text, ["Статус"])[0])  # no link to storage
        self.assertFalse(gate.shortage_verdict("Queued Дом: not enough: minecraft:white_wool x3", ["Склад"])[0])
        self.assertFalse(gate.shortage_verdict("Заказ принят: Дом.", ["Склад"])[0])  # names no material

    def test_refusal(self):
        good = "Не удалось сертифицировать Дом. Не хватает: кровати (0 из 1) — поставьте кровать; крыша."
        self.assertTrue(gate.refusal_verdict(good)[0])
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
