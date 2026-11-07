"""Failing-first regression tests for protocol_entities (no server)."""
import struct
import unittest
import uuid

from .protocol_entities import ProtocolEntities


def varint(value):
    value &= 0xFFFFFFFF
    out = bytearray()
    while True:
        bits = value & 0x7F
        value >>= 7
        if value:
            out.append(bits | 0x80)
        else:
            out.append(bits)
            return bytes(out)


def add_body(entity_id=7, x=8.5, y=64.0, z=-3.25, entity_type=25,
             entity_uuid=None, data=0):
    body = varint(entity_id)
    body += (entity_uuid or uuid.UUID(int=0x0102030405060708090A0B0C0D0E0F)).bytes
    body += varint(entity_type)
    body += struct.pack(">ddd", x, y, z)
    body += b"\x00"  # zero LP movement
    body += b"\x00\x00\x00"  # packed pitch/yaw/head-yaw
    body += varint(data)
    return body

def relative_body(entity_id=7, dx=0, dy=0, dz=0, rot=False):
    body = varint(entity_id) + struct.pack(">hhh", dx, dy, dz)
    return body + (b"\x00\x00" if rot else b"") + b"\x01"

def absolute_body(entity_id=7, x=1.0, y=2.0, z=3.0):
    # VarInt id + PositionMoveRotation (f64 xyz, f64 velocity xyz, f32
    # yaw/pitch) + bool onGround, per entity.rs EntityPositionSync.
    return (varint(entity_id) + struct.pack(">ddddddff", x, y, z,
                                           0.0, 0.0, 0.0, 0.0, 0.0)
            + b"\x01")




def fake_stack_decoder(data, offset):
    count = data[offset]
    item_id = data[offset + 1]
    components = {"present": bool(data[offset + 2])}
    return ({"item_id": item_id, "count": count, "components": components,
             "removed": []}, offset + 3)


class ProtocolEntitiesTests(unittest.TestCase):
    def test_263_linear_and_stepped_movement(self):
        tracker = ProtocolEntities(protocol=777)
        self.consume_add(tracker, x=1.0, y=2.0, z=3.0)
        linear = varint(7) + varint(1) + struct.pack('>hhh', 4096, 0, -2048)
        self.assertTrue(tracker.consume(0x35, linear), tracker.snapshot())
        stepped = (varint(7) + varint(4) + varint(2) + struct.pack('>hhh', 4096, 0, 0)
                   + varint(3) + struct.pack('>hhh', 0, 2048, 0))
        self.assertTrue(tracker.consume(0x35, stepped), tracker.snapshot())
        self.assertEqual(tracker.snapshot()['entities'][0]['position'], [3.0, 2.5, 2.5])
        absolute = varint(7) + varint(0) + struct.pack('>dddff', 4.0, 5.0, 6.0, 0.0, 0.0) + bytes([1])
        self.assertTrue(tracker.consume(0x23, absolute), tracker.snapshot())
        self.assertEqual(tracker.snapshot()['entities'][0]['position'], [4.0, 5.0, 6.0])
        self.assertFalse(tracker.consume(0x35, varint(7) + varint(2)))
        self.assertEqual(tracker.snapshot()['unavailable'], 'bad_movement_step_count')

    def setUp(self):
        self.tracked = ProtocolEntities(decode_stack=fake_stack_decoder)

    def consume_add(self, tracker=None, **kwargs):
        tracker = tracker or self.tracked
        self.assertTrue(tracker.consume(0x01, add_body(**kwargs)))
        return tracker

    def test_add_move_remove_round_trip(self):
        self.consume_add()
        self.assertTrue(self.tracked.consume(
            0x35, relative_body(dx=4096, dy=-2048, dz=512)))
        snapshot = self.tracked.snapshot()
        self.assertEqual(snapshot["entities"][0]["position"], [9.5, 63.5, -3.125])
        self.assertEqual(snapshot["entities"][0]["spawn_position"], [8.5, 64.0, -3.25])
        self.assertTrue(self.tracked.consume(0x4D, varint(1) + varint(7)))
        snapshot = self.tracked.snapshot()
        self.assertEqual(snapshot["entities"], [])
        self.assertIsNone(snapshot["unavailable"])

    def test_absolute_sync_applies_without_inventing_entities(self):
        self.assertTrue(self.tracked.consume(0x35, relative_body(dx=8)))
        self.assertEqual(self.tracked.snapshot()["entities"], [])
        self.consume_add(x=1.0, y=2.0, z=3.0)
        self.assertTrue(self.tracked.consume(
            0x23, absolute_body(x=-40.5, y=70.0, z=1000.25)))
        self.assertEqual(
            self.tracked.snapshot()["entities"][0]["position"],
            [-40.5, 70.0, 1000.25])

    def test_rotation_only_never_moves_position(self):
        self.consume_add(x=1.0, y=2.0, z=3.0)
        body = varint(7) + b"\x40\x80" + b"\x01"
        self.assertTrue(self.tracked.consume(0x38, body))
        self.consume_add(entity_id=9, x=4.0, y=5.0, z=6.0)
        self.assertTrue(self.tracked.consume(
            0x36, relative_body(entity_id=9, dx=16, rot=True)))
        snapshot = self.tracked.snapshot()
        self.assertEqual(snapshot["entities"][0]["position"], [1.0, 2.0, 3.0])
        self.assertEqual(snapshot["entities"][1]["position"],
                         [4.00390625, 5.0, 6.0])

    def test_negative_coordinates_preserved(self):
        self.consume_add(entity_id=11, x=-30000000.5, y=-64.0, z=-0.125)
        entry = self.tracked.snapshot()["entities"][0]
        self.assertEqual(entry["position"], [-30000000.5, -64.0, -0.125])
        self.assertEqual(entry["uuid"], "00010203-0405-0607-0809-0a0b0c0d0e0f")
        self.assertEqual(entry["type"], 25)

    def test_remove_then_respawn_clears_everything(self):
        self.consume_add(entity_id=7)
        self.consume_add(entity_id=9)
        self.assertTrue(self.tracked.consume(
            0x4D, varint(2) + varint(7) + varint(99)))
        self.assertEqual(
            [entry["id"] for entry in self.tracked.snapshot()["entities"]], [9])
        self.assertTrue(self.tracked.consume(0x52, b"ignored-dimension-body"))
        self.assertEqual(self.tracked.snapshot()["entities"], [])

    def test_unknown_packet_returns_false_without_unavailable(self):
        self.assertFalse(self.tracked.consume(0x99, b"\x01\x02"))
        self.assertIsNone(self.tracked.snapshot()["unavailable"])

    def test_truncated_body_fails_closed(self):
        self.assertFalse(self.tracked.consume(0x01, add_body()[:-4]))
        self.assertEqual(self.tracked.snapshot()["entities"], [])
        self.assertIsNotNone(self.tracked.snapshot()["unavailable"])
        self.consume_add(entity_id=21)
        self.assertTrue(self.tracked.consume(0x35, relative_body(entity_id=21, dx=8)))
        self.assertIsNotNone(self.tracked.snapshot()["unavailable"])
        self.tracked.consume(0x52, b"respawn")
        self.assertIsNone(self.tracked.snapshot()["unavailable"])

    def test_trailing_bytes_rejected(self):
        self.consume_add()
        self.assertFalse(self.tracked.consume(0x35, relative_body() + b"\x00"))
        self.assertIsNotNone(self.tracked.snapshot()["unavailable"])
        self.assertEqual(
            self.tracked.snapshot()["entities"][0]["position"], [8.5, 64.0, -3.25])

    def test_unknown_metadata_serializer_fails_closed_partial(self):
        self.consume_add()
        body = varint(7) + bytes([4, 99]) + varint(5) + b"\xff"
        self.assertFalse(self.tracked.consume(0x63, body))
        snapshot = self.tracked.snapshot()
        self.assertTrue(snapshot["entities"][0]["partial"])
        self.assertTrue(snapshot["partial"])
        self.assertIn("unknown_metadata_serializer:99", snapshot["unavailable"])
        self.assertNotIn("health", snapshot["entities"][0])
        # A partial update cannot reconstruct the unknown fields previously lost.
        self.assertTrue(self.tracked.consume(
            0x63, varint(7) + bytes([9, 3]) + struct.pack(">f", 12.5) + b"\xff"))
        snapshot = self.tracked.snapshot()
        self.assertTrue(snapshot["entities"][0]["partial"])
        self.assertEqual(snapshot["entities"][0]["health"], 12.5)

    def test_item_stack_never_invented_and_uses_callback(self):
        self.consume_add()
        plain = ProtocolEntities()
        self.consume_add(plain)
        body = varint(7) + bytes([8, 7, 5, 3, 1]) + b"\xff"
        self.assertFalse(plain.consume(0x63, body))
        self.assertIn("item_stack_decoder_missing", plain.snapshot()["unavailable"])
        self.assertNotIn("item", plain.snapshot()["entities"][0])
        self.assertTrue(self.tracked.consume(0x63, body))
        # Wire order is count then item id (play.rs ItemStack encode);
        # payload bytes above are count=5, id=3, components-present=1.
        self.assertEqual(self.tracked.snapshot()["entities"][0]["item"],
                         {"item_id": 3, "count": 5,
                          "components": {"present": True}, "removed": []})

    def test_real_component_stack_decoder_contract(self):
        from .protocol_observations import decode_stack
        tracker = ProtocolEntities(decode_stack=decode_stack)
        self.consume_add(tracker)
        body = varint(7) + bytes([8, 7, 5, 3, 0, 0]) + b"\xff"
        self.assertTrue(tracker.consume(0x63, body), tracker.snapshot())
        self.assertEqual(tracker.snapshot()["entities"][0]["item"],
                         {"item_id": 3, "count": 5, "components": {}, "removed": []})

    def test_overflowed_entity_varint_fails_closed(self):
        self.assertFalse(self.tracked.consume(0x4D, b"\x01\xff\xff\xff\xff\x1f"))
        self.assertIsNotNone(self.tracked.snapshot()["unavailable"])

    def test_health_and_local_xp_values_through_known_indices(self):
        self.consume_add()
        body = (varint(7) + bytes([9, 3]) + struct.pack(">f", 6.5)
                + bytes([8, 1]) + varint(300) + b"\xff")
        self.assertTrue(self.tracked.consume(0x63, body))
        entry = self.tracked.snapshot()["entities"][0]
        self.assertEqual(entry["health"], 6.5)
        self.assertEqual(entry["xp_value"], 300)
        # Real local SetExperience packet: f32 progress, VarInt level,
        # VarInt total (env_state.rs encode order, not struct order).
        xp = struct.pack(">f", 0.5) + varint(3) + varint(7)
        self.assertTrue(self.tracked.consume(0x67, xp))
        snapshot = self.tracked.snapshot()
        self.assertAlmostEqual(snapshot["xp"]["progress"], 0.5)
        self.assertEqual(snapshot["xp"]["level"], 3)
        self.assertEqual(snapshot["xp"]["total"], 7)

    def test_effects_round_trip_through_real_packets(self):
        self.consume_add()
        update = (varint(7) + varint(5) + varint(2) + varint(200)
                  + bytes([0x03]))
        self.assertTrue(self.tracked.consume(0x84, update))
        effects = self.tracked.snapshot()["entities"][0]["effects"]
        self.assertEqual(effects["5"]["amplifier"], 2)
        self.assertEqual(effects["5"]["duration_ticks"], 200)
        self.assertTrue(effects["5"]["flags"]["ambient"])
        self.assertTrue(effects["5"]["flags"]["visible"])
        self.assertTrue(self.tracked.consume(0x4E, varint(7) + varint(5)))
        self.assertEqual(self.tracked.snapshot()["entities"][0]["effects"], {})

    def test_local_effects_use_login_identity_without_inventing_spawn(self):
        tracker = ProtocolEntities(local_entity_id=42)
        self.assertNotIn('effects', tracker.snapshot()['local'])
        update = varint(42) + varint(5) + varint(2) + varint(200) + b'\x03'
        self.assertTrue(tracker.consume(0x84, update))
        self.assertEqual(tracker.snapshot()['local']['effects']['5']['amplifier'], 2)
        self.assertEqual(tracker.snapshot()['entities'], [])
        self.assertTrue(tracker.consume(0x4E, varint(42) + varint(5)))
        self.assertEqual(tracker.snapshot()['local']['effects'], {})
        tracker.consume(0x52, b'respawn')
        self.assertNotIn('effects', tracker.snapshot()['local'])

    def test_snapshot_is_json_ready_and_defensive(self):
        self.consume_add()
        import json
        first = json.dumps(self.tracked.snapshot())
        snapshot = self.tracked.snapshot()
        snapshot["entities"][0]["position"][0] = 999999.0
        self.assertNotEqual(
            json.loads(first)["entities"][0]["position"][0], 999999.0)


if __name__ == "__main__":
    unittest.main()
