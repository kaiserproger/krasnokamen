"""Wire decoder/registration fences, not graphical acceptance evidence."""
from __future__ import annotations

import json
import struct
import tempfile
import unittest
from pathlib import Path

from . import scenarios
from .backends import native_promotion_maps as maps
from .backends import native_promotion_wire as wire
from .backends import native_v2


def varint(value: int) -> bytes:
    result = bytearray()
    while value >= 128:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def string(value: str) -> bytes:
    encoded = value.encode()
    return struct.pack(">H", len(encoded)) + encoded


def map_packet(map_id=5, *, markers=None, patch=None, locked=False) -> bytes:
    result = varint(map_id) + bytes((0, locked, markers is not None))
    if markers is not None:
        result += varint(len(markers))
        for kind, x, y, rotation in markers:
            result += varint(kind) + struct.pack(">bbB", x, y, rotation) + b"\x00"
    if patch is None:
        return result + b"\x00"
    width, height, x, y, colors = patch
    return result + bytes((width, height, x, y)) + varint(len(colors)) + bytes(colors)


class PromotionWireTests(unittest.TestCase):
    def test_map_patch_signed_markers_and_strict_bounds(self):
        payload = map_packet(markers=[(7, -128, 127, 31)], patch=(2, 1, 126, 127, [4, 8]), locked=True)
        decoded = wire.decode_map(payload)
        self.assertEqual(decoded["markers"], [{"kind": 7, "x": -128, "y": 127, "rotation": 15, "name": None}])
        self.assertEqual(decoded["patch"]["colors"], [4, 8])
        self.assertTrue(decoded["locked"])
        for bad in (payload[:-1], payload + b"\x00", map_packet(patch=(2, 1, 127, 127, [4, 8])),
                    map_packet(patch=(2, 2, 0, 0, [4, 8]))):
            with self.subTest(payload=bad), self.assertRaises(ValueError):
                wire.decode_map(bad)

    def test_named_marker_uses_nameless_network_nbt(self):
        payload = b"\x05\x00\x00\x01\x01\x07\x00\x00\x00\x01\x08" + string("Home") + b"\x00"
        self.assertEqual(wire.decode_map(payload)["markers"][0]["name"], "Home")

    def test_placement_preserves_signed_position_and_nested_components(self):
        x, y, z = -37, -12, 128
        packed = ((x & 0x3FFFFFF) << 38) | ((z & 0x3FFFFFF) << 12) | (y & 4095)
        nbt = (b"\x0a\x0a" + string("CustomName") + b"\x08" + string("text") + string("Named banner")
               + b"\x00\x09" + string("patterns") + b"\x0a" + struct.pack(">i", 1)
               + b"\x08" + string("pattern") + string("minecraft:cross") + b"\x08"
               + string("color") + string("red") + b"\x00\x00")
        payload = packed.to_bytes(8, "big") + varint(18) + nbt
        self.assertEqual(wire.decode_placement(payload), {
            "position": [x, y, z], "type": 18,
            "nbt": {"CustomName": {"text": "Named banner"},
                    "patterns": [{"pattern": "minecraft:cross", "color": "red"}]}})
        for bad in (payload[:-1], payload + b"\x00"):
            with self.assertRaises(ValueError):
                wire.decode_placement(bad)

    def test_receipt_applies_sparse_updates_retains_markers_and_is_viewer_fenced(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = root / "packets.json"
            report.write_text(json.dumps({"play": {"clientbound": {
                "minecraft:map_item_data": {"protocol_id": 52},
                "minecraft:block_entity_data": {"protocol_id": 6}}}}))
            report.with_name("registries.json").write_text(json.dumps({
                "minecraft:map_decoration_type": {"entries": {"minecraft:frame": {"protocol_id": 7}}}}))
            receipt = wire.PromotionWireReceipt(report, root)
            viewer = object()
            receipt.server_packet(viewer, "configuration", 52, b"not a map")
            self.assertEqual(receipt.snapshot(viewer)["events"], [])
            receipt.server_packet(viewer, "play", 52, map_packet(markers=[(7, 2, 3, 0)], patch=(2, 1, 126, 127, [4, 8])))
            receipt.server_packet(viewer, "play", 52, map_packet(patch=(1, 1, 126, 127, [12])))
            snapshot = receipt.snapshot(viewer)
            self.assertEqual(snapshot["maps"][5]["colors"][-2:], [12, 8])
            self.assertEqual(len(snapshot["maps"][5]["markers"]), 1)
            snapshot["maps"][5]["colors"][-1] = 99
            self.assertEqual(receipt.snapshot(viewer)["maps"][5]["colors"][-1], 8)
            receipt.server_packet(viewer, "play", 52, map_packet(markers=[]))
            self.assertEqual(receipt.snapshot(viewer)["maps"][5]["markers"], [])
            self.assertTrue((root / "promotion-wire-receipts.json").is_file())
            with self.assertRaisesRegex(RuntimeError, "one graphical viewer"):
                receipt.server_packet(object(), "play", 52, map_packet())
            with self.assertRaisesRegex(RuntimeError, "queried graphical"):
                receipt.snapshot(object())

    def test_every_map_probe_dispatches_through_native_v2_not_the_default_pig_probe(self):
        for kind in maps.SCENARIO_KINDS:
            name = "native-v2-map-" + kind
            with self.subTest(name=name):
                self.assertIn(name, native_v2.NATIVE_SCENARIO_IDS)
                self.assertIn(name, native_v2.RUNNERS)
                self.assertEqual(scenarios.SCENARIOS[name].evidence, "graphical")
                self.assertEqual(scenarios.SCENARIOS[name].profile, "entities")


if __name__ == "__main__":
    unittest.main()
