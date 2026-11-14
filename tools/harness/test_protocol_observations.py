"""Pinned wire vectors for protocol_observations (no live server).

Every encoder below mirrors the exact layouts named in the module docstring
(VarInt stack/codec framing, network-NBT values). The vectors pin the
supported subset; rejection cases pin the fail-closed boundaries.
"""
import struct
import unittest

from . import protocol_observations as obs
from .wire import UnsupportedObservation


def varint(value):
    value &= 0xFFFFFFFF
    out = bytearray()
    while True:
        part = value & 0x7F
        value >>= 7
        if value:
            out.append(part | 0x80)
        else:
            out.append(part)
            return bytes(out)


def nbt_compound(entries):
    out = bytearray()
    for tag, name, payload in entries:
        out.append(tag)
        raw = name.encode()
        out += struct.pack(">H", len(raw)) + raw + payload
    out.append(0)
    return bytes(out)


def text_nbt(text):
    raw = text.encode()
    return b"\x08" + struct.pack(">H", len(raw)) + raw


def nbt_str_payload(text):
    """Bare string payload for compound entries and list elements (no tag)."""
    raw = text.encode()
    return struct.pack(">H", len(raw)) + raw


def stack_bytes(item, count):
    return varint(count) + varint(item) + varint(0) + varint(0)


def with_patch(item, count, added_ids_payloads, removed_ids=()):
    body = varint(count) + varint(item)
    body += varint(len(added_ids_payloads)) + varint(len(removed_ids))
    for cid, payload in added_ids_payloads:
        body += varint(cid) + payload
    for cid in removed_ids:
        body += varint(cid)
    return body


def enchant_payload(*pairs):
    body = varint(len(pairs))
    for enchant_id, level in pairs:
        body += varint(enchant_id) + varint(level)
    return body


class CursorTests(unittest.TestCase):
    def test_reusable_cursor_advances_and_reports_offset(self):
        cursor = obs.Cursor(b"\x01\x02\x03")
        self.assertEqual(cursor.varint(), 1)
        self.assertEqual(cursor.tell(), 1)
        self.assertEqual(cursor.remaining, 2)
        self.assertEqual(cursor.take(2), b"\x02\x03")

    def test_overlong_varint_rejected(self):
        with self.assertRaises(obs.TruncatedObservation):
            obs.Cursor(b"\x80\x80\x80\x80\x80\x01").varint()

    def test_truncated_read_rejected(self):
        with self.assertRaises(obs.TruncatedObservation):
            obs.Cursor(b"\x01").take(2)


class StackTests(unittest.TestCase):
    def test_plain_stack_vector(self):
        stack, offset = obs.decode_stack(stack_bytes(42, 16), 0)
        self.assertEqual(
            stack,
            {"item_id": 42, "count": 16, "components": {}, "removed": []},
        )
        self.assertEqual(offset, len(stack_bytes(42, 16)))

    def test_empty_stack_returns_none(self):
        stack, offset = obs.decode_stack(b"\x00", 0)
        self.assertIsNone(stack)
        self.assertEqual(offset, 1)

    def test_named_shop_item_custom_name(self):
        name_component = nbt_compound([(8, "text", nbt_str_payload("Shop"))])
        body = with_patch(443, 1, [(6, b"\x0a" + name_component)])
        stack, offset = obs.decode_stack(body, 0)
        self.assertEqual(stack["components"]["minecraft:custom_name"], {"text": "Shop"})
        self.assertEqual(offset, len(body))

    def test_damaged_tool_and_max_damage(self):
        body = with_patch(300, 1, [(3, varint(7)), (2, varint(1561))])
        stack, _ = obs.decode_stack(body, 0)
        self.assertEqual(stack["components"]["minecraft:damage"], 7)
        self.assertEqual(stack["components"]["minecraft:max_damage"], 1561)

    def test_negative_damage_clamped_like_rust(self):
        body = with_patch(300, 1, [(3, varint(0xFFFFFFFF))])
        stack, _ = obs.decode_stack(body, 0)
        self.assertEqual(stack["components"]["minecraft:damage"], 0)

    def test_lore_and_enchants_vectors(self):
        line_a = text_nbt("A")
        styled = b"\x0a" + nbt_compound(
            [(8, "text", nbt_str_payload("B")), (1, "bold", b"\x01")]
        )
        lore = varint(2) + line_a + styled
        body = with_patch(436, 1, [(11, lore), (13, enchant_payload((1, 5), (2, 3)))])
        stack, _ = obs.decode_stack(body, 0)
        self.assertEqual(
            stack["components"]["minecraft:lore"], ["A", {"text": "B", "bold": 1}]
        )
        self.assertEqual(
            stack["components"]["minecraft:enchantments"], [[1, 5], [2, 3]]
        )

    def test_potion_contents_vector(self):
        name = "Brew".encode()
        payload = (
            b"\x01" + varint(7) + b"\x01" + struct.pack(">i", 0xFF0000)
            + varint(1) + varint(3) + varint(2) + varint(200)
            + b"\x00\x01\x00\x00" + b"\x01" + varint(len(name)) + name
        )
        body = with_patch(800, 1, [(51, payload)])
        stack, _ = obs.decode_stack(body, 0)
        potion = stack["components"]["minecraft:potion_contents"]
        self.assertEqual(potion["potion"], 7)
        self.assertEqual(potion["color"], 0xFF0000)
        self.assertEqual(potion["effects"][0]["id"], 3)
        self.assertEqual(potion["custom_name"], "Brew")

    def test_removed_component_captured(self):
        body = with_patch(300, 1, [], removed_ids=[3])
        stack, _ = obs.decode_stack(body, 0)
        self.assertEqual(stack["removed"], ["minecraft:damage"])
        self.assertEqual(stack["components"], {})

    def test_unknown_component_names_exact_id(self):
        body = varint(1) + varint(42) + varint(1) + varint(0) + varint(16)
        with self.assertRaisesRegex(UnsupportedObservation, "minecraft:attribute_modifiers"):
            obs.decode_stack(body, 0)

    def test_out_of_table_component_names_numeric_id(self):
        body = varint(1) + varint(42) + varint(1) + varint(0) + varint(200)
        with self.assertRaisesRegex(UnsupportedObservation, "200"):
            obs.decode_stack(body, 0)

    def test_patch_cursor_never_desyncs_on_unknown_id(self):
        body = varint(1) + varint(42) + varint(1) + varint(0) + varint(16) + b"\x00"
        try:
            obs.decode_stack(body, 0)
        except UnsupportedObservation:
            pass
        else:  # pragma: no cover
            self.fail("expected UnsupportedObservation")

    def test_truncated_components_rejected(self):
        with self.assertRaises(obs.TruncatedObservation):
            obs.decode_stack(varint(1) + varint(42) + varint(1), 0)

    def test_oversize_patch_shape_rejected(self):
        body = varint(1) + varint(42) + varint(111) + varint(0)
        with self.assertRaises(UnsupportedObservation):
            obs.decode_stack(body, 0)

    def test_invalid_max_damage_rejected(self):
        body = with_patch(300, 1, [(2, varint(0))])
        with self.assertRaises(UnsupportedObservation):
            obs.decode_stack(body, 0)

    def test_invalid_enchantment_level_rejected(self):
        body = with_patch(300, 1, [(13, enchant_payload((1, 0)))])
        with self.assertRaises(UnsupportedObservation):
            obs.decode_stack(body, 0)

    def test_deeply_nested_nbt_bounded(self):
        nested = text_nbt("x")
        for _ in range(600):
            nested = b"\x0a" + nbt_compound([(10, "n", nested)])
        body = with_patch(300, 1, [(0, nested)])
        with self.assertRaises(obs.TruncatedObservation):
            obs.decode_stack(body, 0)


class MenuTests(unittest.TestCase):
    def test_menu_image_decodes_slots_and_cursor(self):
        body = (
            varint(4) + varint(7) + varint(3)
            + b"\x00" + stack_bytes(42, 16) + b"\x00"
            + stack_bytes(42, 2)
        )
        self.assertEqual(
            obs.decode_menu(body),
            {
                "window": 4,
                "state_id": 7,
                "slots": [
                    None,
                    {"item_id": 42, "count": 16, "components": {}, "removed": []},
                    None,
                ],
                "cursor": {"item_id": 42, "count": 2, "components": {}, "removed": []},
            },
        )

    def test_menu_bearing_components_decode(self):
        named = with_patch(443, 1, [(6, b"\x08" + struct.pack(">H", 4) + b"Shop")])
        carried = with_patch(443, 2, [(6, b"\x08" + struct.pack(">H", 4) + b"Shop")])
        body = varint(1) + varint(9) + varint(1) + named + carried
        menu = obs.decode_menu(body)
        self.assertEqual(menu["slots"][0]["components"]["minecraft:custom_name"], "Shop")
        self.assertEqual(menu["cursor"]["components"]["minecraft:custom_name"], "Shop")

    def test_menu_trailing_bytes_rejected(self):
        with self.assertRaises(obs.TruncatedObservation):
            obs.decode_menu(varint(0) + varint(0) + varint(0) + b"\x00" + b"\x63")

    def test_menu_too_many_slots_rejected(self):
        with self.assertRaises(obs.TruncatedObservation):
            obs.decode_menu(varint(0) + varint(0) + varint(300) + b"\x00")

    def test_truncated_menu_rejected(self):
        with self.assertRaises(obs.TruncatedObservation):
            obs.decode_menu(varint(0) + varint(1) + varint(1))


class MerchantTests(unittest.TestCase):
    @staticmethod
    def cost(item, count):
        return varint(item) + varint(count) + varint(0)

    def offer_body(self):
        output = stack_bytes(264, 1)
        return (
            varint(1) + varint(1)
            + self.cost(376, 5) + output + b"\x01" + self.cost(377, 1)
            + b"\x00" + struct.pack(">i", 2) + struct.pack(">i", 12)
            + struct.pack(">i", 3) + struct.pack(">i", 0)
            + struct.pack(">f", 1.0) + struct.pack(">i", 7)
            + varint(2) + varint(10) + b"\x01\x00"
        )

    def test_merchant_offers_decode(self):
        result = obs.decode_merchant_offers(self.offer_body())
        self.assertEqual(
            result,
            {
                "container_id": 1,
                "offers": [
                    {
                        "input1": {"item_id": 376, "count": 5},
                        "input2": {"item_id": 377, "count": 1},
                        "output": {"item_id": 264, "count": 1, "components": {}, "removed": []},
                        "out_of_stock": False,
                        "uses": 2,
                        "max_uses": 12,
                        "xp": 3,
                        "special_price": 0,
                        "price_multiplier": 1.0,
                        "demand": 7,
                    }
                ],
                "level": 2,
                "xp": 10,
                "show_progress": True,
                "can_restock": False,
            },
        )

    def test_nonempty_predicate_rejected_without_desync(self):
        body = (
            varint(1) + varint(1)
            + varint(376) + varint(5) + varint(1)
            + stack_bytes(264, 1) + b"\x00\x00"
            + struct.pack(">i", 0) + struct.pack(">i", 1) + struct.pack(">i", 0)
            + struct.pack(">i", 0) + struct.pack(">f", 0.0) + struct.pack(">i", 0)
            + varint(1) + varint(0) + b"\x00\x00"
        )
        with self.assertRaisesRegex(UnsupportedObservation, "exact component"):
            obs.decode_merchant_offers(body)

    def test_truncated_merchant_rejected(self):
        with self.assertRaises(obs.TruncatedObservation):
            obs.decode_merchant_offers(varint(1) + varint(1) + varint(376))

    def test_invalid_counters_rejected(self):
        output = stack_bytes(264, 1)
        body = (
            varint(1) + varint(1) + self.cost(376, 5) + output + b"\x00\x00"
            + struct.pack(">i", 5) + struct.pack(">i", 2) + struct.pack(">i", 0)
            + struct.pack(">i", 0) + struct.pack(">f", 0.0) + struct.pack(">i", 0)
            + varint(1) + varint(0) + b"\x00\x00"
        )
        with self.assertRaises(UnsupportedObservation):
            obs.decode_merchant_offers(body)

    def test_merchant_trailing_bytes_rejected(self):
        with self.assertRaises(obs.TruncatedObservation):
            obs.decode_merchant_offers(self.offer_body() + b"\x00")


class RegistryTests(unittest.TestCase):
    @staticmethod
    def string(value):
        raw = value.encode()
        return varint(len(raw)) + raw

    def test_received_names_keep_wire_registry_order_without_payloads(self):
        body = self.string("minecraft:enchantment") + varint(2)
        body += self.string("minecraft:sharpness") + b"\x00"
        body += self.string("minecraft:mending") + b"\x00"
        self.assertEqual(obs.decode_registry(body), (
            "minecraft:enchantment", ["minecraft:sharpness", "minecraft:mending"]))

    def test_optional_compound_payload_is_consumed_not_guessed(self):
        body = self.string("minecraft:enchantment") + varint(2)
        body += self.string("minecraft:mending") + b"\x01\x0a\x00"
        body += self.string("minecraft:sharpness") + b"\x00"
        self.assertEqual(obs.decode_registry(body)[1],
                         ["minecraft:mending", "minecraft:sharpness"])

    def test_registry_bounds_and_trailing_bytes_fail_closed(self):
        prefix = self.string("minecraft:enchantment")
        for suffix in (varint(-1), varint(65537), varint(1),
                       varint(0) + b"\x00"):
            with self.subTest(suffix=suffix), self.assertRaises(obs.TruncatedObservation):
                obs.decode_registry(prefix + suffix)


class ChatTests(unittest.TestCase):
    def test_literal_chat_text_without_raw_bytes(self):
        payload = text_nbt("Hello")
        result = obs.decode_chat_component(payload + b"\x00")
        self.assertEqual(result["text"], "Hello")
        self.assertIsNone(result["translate"])
        self.assertFalse(result["overlay"])

    def test_styled_compound_chat_renders_text(self):
        payload = b"\x0a" + nbt_compound(
            [(8, "text", nbt_str_payload("Hi")), (8, "color", nbt_str_payload("gold"))]
        )
        result = obs.decode_chat_component(payload + b"\x01")
        self.assertEqual(result["text"], "Hi")
        self.assertTrue(result["overlay"])

    def test_translate_preserved_not_localized(self):
        # NBT lists carry element type + count, then element payloads only.
        args = (
            b"\x08" + struct.pack(">i", 2)
            + struct.pack(">H", 1) + b"A" + struct.pack(">H", 1) + b"B"
        )
        payload = b"\x0a" + nbt_compound(
            [(8, "translate", nbt_str_payload("chat.type.text")), (9, "with", args)]
        )
        result = obs.decode_chat_component(payload + b"\x00")
        self.assertEqual(result["text"], "")
        self.assertEqual(result["translate"], "chat.type.text")
        self.assertEqual([arg["text"] for arg in result["with"]], ["A", "B"])

    def test_invalid_chat_shape_rejected(self):
        with self.assertRaises(UnsupportedObservation):
            obs.decode_chat_component(b"\x03" + struct.pack(">i", 7) + b"\x00")


if __name__ == "__main__":
    unittest.main()
