#!/usr/bin/env python3
"""Hermetic unit tests for protocol_station_verbs.ProtocolStationVerbs.

No server is started here: ``send`` is faked and only the exact bytes passed
to ``observer.send`` are asserted against the pinned layouts in local
``mc-protocol`` source (see the mixin docstring). The real release binary is
exercised by the parent matrix via cmd_run. Run with:
python3 -m unittest tools.harness.test_protocol_station_verbs
"""

from __future__ import annotations

import unittest

from . import wire
from .matrix_api import MissingCapability
from .protocol_station_verbs import ProtocolStationVerbs


class _FakeObserver:
    def __init__(self, window=4):
        self.current_window = window
        self.sent: list[tuple[int, bytes]] = []
        self.recipe_displays = None

    def send(self, packet_id: int, body: bytes = b"") -> None:
        self.sent.append((packet_id, bytes(body)))


class _Client(ProtocolStationVerbs):
    def __init__(self):
        self.observer = _FakeObserver()
        self.items = {1: 'minecraft:stone', 42: 'minecraft:diamond'}


def _writes(client):
    return client.observer.sent


class ContainerButtonTests(unittest.TestCase):
    def test_id_and_varint_layout(self):
        c = _Client()
        c.container_button(2)
        # ServerboundContainerButtonClick ID 0x11: VarInt window + VarInt button.
        # Window 4 -> 04; inventory.rs round-trips container 300 as 0xAC 0x02.
        self.assertEqual(_writes(c), [(0x11, bytes([4, 2])), (0x0D, b"")])

    def test_multibyte_window_roundtrip(self):
        self.assertEqual(wire.write_varint(300), bytes([0xAC, 0x02]))
        c = _Client()
        c.observer.current_window = 300
        c.container_button(2)
        self.assertEqual(_writes(c)[0], (0x11, bytes([0xAC, 0x02, 2])))


class SelectTradeTests(unittest.TestCase):
    def test_correct_serverbound_id(self):
        c = _Client()
        c.select_trade(3)
        # ServerboundSelectTrade ID 0x33 (merchant.rs), not 0x34 (clientbound offers).
        self.assertEqual(_writes(c), [(0x33, bytes([3])), (0x0D, b"")])


class RenameItemTests(unittest.TestCase):
    def test_plain_ascii_layout(self):
        c = _Client()
        c.rename_item('hi')
        self.assertEqual(_writes(c), [(0x30, bytes([2]) + b'hi'), (0x0D, b"")])

    def test_empty_name_clears(self):
        c = _Client()
        c.rename_item('')
        self.assertEqual(_writes(c), [(0x30, b'\x00'), (0x0D, b"")])

    def test_utf16_limit_counts_code_units_not_bytes(self):
        c = _Client()
        c.rename_item('😀' * 16383)  # 16383 surrogate pairs = 32766 units: allowed.
        body = _writes(c)[0][1]
        c2 = _Client()
        with self.assertRaises(ValueError):
            c2.rename_item('😀' * 16384)  # 32768 units: rejected like write_string(32767).
        self.assertEqual(_writes(c2), [])
        self.assertEqual(len(body), 3 + 16383 * 4)


class SetBeaconTests(unittest.TestCase):
    def test_none_none_is_two_false_bytes(self):
        c = _Client()
        c.set_beacon(None, None)
        self.assertEqual(_writes(c), [(0x34, b'\x00\x00'), (0x0D, b"")])

    def test_named_effects_use_registry_ids(self):
        c = _Client()
        c.set_beacon('minecraft:speed', 'regeneration')  # 0 and 9 per name_for_id table.
        self.assertEqual(_writes(c), [(0x34, b'\x01\x00\x01\x09'), (0x0D, b"")])


class CrafterSlotTests(unittest.TestCase):
    def test_disable_sends_new_state_false(self):
        c = _Client()
        c.observer.current_window = 130
        c.toggle_crafter_slot(8, True)
        # ServerboundContainerSlotStateChanged ID 0x14: slot VarInt, container
        # VarInt, bool; crafter_tests.rs pins slot 8 / container 130.
        self.assertEqual(_writes(c)[0], (0x14, bytes([8, 0x82, 1, 0])))
        self.assertEqual(_writes(c)[1], (0x0D, b""))

    def test_enable_sends_new_state_true(self):
        c = _Client()
        c.observer.current_window = 130
        c.toggle_crafter_slot(8, False)
        self.assertEqual(_writes(c)[0], (0x14, bytes([8, 0x82, 1, 1])))


class CreativeSlotTests(unittest.TestCase):
    def test_plain_stack_matches_pinned_bytes(self):
        c = _Client()
        c.observer.current_window = 0
        c.set_creative_slot(36, 'minecraft:stone', 64)
        # serverbound_and_slots.rs: slot 36 i16 + count 64 + item 1 + 0/0 patch.
        self.assertEqual(_writes(c)[0], (0x38, bytes([0x00, 0x24, 0x40, 0x01, 0x00, 0x00])))


class PlaceRecipeTests(unittest.TestCase):
    def test_no_mapping_is_missing_capability(self):
        c = _Client()
        with self.assertRaisesRegex(MissingCapability, 'recipe_display_mapping'):
            c.place_recipe('minecraft:oak_planks')
        self.assertEqual(_writes(c), [])

    def test_unknown_recipe_names_exact_id(self):
        c = _Client()
        c.observer.recipe_displays = {'minecraft:oak_planks': 7}
        with self.assertRaisesRegex(MissingCapability, r'recipe_display:minecraft:stick'):
            c.place_recipe('minecraft:stick')
        self.assertEqual(_writes(c), [])

    def test_no_string_to_id_inference(self):
        # '300' must not map to display 300 without a received entry.
        c = _Client()
        c.observer.recipe_displays = {}
        with self.assertRaises(MissingCapability):
            c.place_recipe('300')
        self.assertEqual(_writes(c), [])

    def test_layout_and_use_max_flag(self):
        c = _Client()
        c.observer.recipe_displays = {'minecraft:oak_planks': 300}
        c.place_recipe('minecraft:oak_planks')
        # ServerboundPlaceRecipe ID 0x27: VarInt window + VarInt display + bool.
        self.assertEqual(_writes(c)[0], (0x27, bytes([4]) + wire.write_varint(300) + b'\x00'))
        c2 = _Client()
        c2.observer.recipe_displays = {'minecraft:oak_planks': 300}
        c2.place_recipe('minecraft:oak_planks', all=True)
        self.assertEqual(_writes(c2)[0][1][-1:], b'\x01')


class StaleClickTests(unittest.TestCase):
    def test_stale_state_encoded_verbatim(self):
        c = _Client()
        c.stale_click(12, kind='quick_move', button=1, state_id=7)
        # ContainerClick ID 0x12 via wire.click_body: 04 window 07 state
        # 00 0c slot i16, 01 button i8, 01 mode, 00 00 empty prediction.
        self.assertEqual(_writes(c)[0], (0x12, bytes([4, 7, 0, 12, 1, 1, 0, 0])))
        self.assertEqual(_writes(c)[1], (0x0D, b""))


if __name__ == '__main__':
    unittest.main()
