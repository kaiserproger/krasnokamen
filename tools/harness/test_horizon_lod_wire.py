"""Byte-layout pins for the Horizon `horizon:lod` client helper, and its
handshake against a scripted server (no Krasnokamen server, no Horizon)."""
from __future__ import annotations

import socket
import struct
import threading
import unittest

from . import horizon_lod_wire as hz
from . import wire


def column(height: int, state: int = 9, water: tuple[int, int] | None = None) -> hz.Column:
    return hz.Column(hz.Surface(height, state, water, "minecraft:plains"),
                     (hz.Span(height - 2, height, state, "minecraft:plains", 0xF0),))


def tile(base: int = 60) -> list[hz.Column]:
    return [column(base + i % 7, water=(62, 86) if i % 5 == 0 else None)
            for i in range(hz.SIDE * hz.SIDE)]


class LayoutTests(unittest.TestCase):
    def test_offer_bytes_match_lod_net_encode(self):
        # lod_net.rs Offer::encode: varints VERSION, distance, in_flight, per_second.
        self.assertEqual(hz.Offer(16).encode(), b"\x01\x10\x08\x40")
        self.assertEqual(hz.decode_offer(b"\x01\x10\x08\x40"), hz.Offer(16, 8, 64))
        self.assertEqual(hz.decode_offer(hz.Offer(5000).encode()).distance, hz.MAX_DISTANCE)

    def test_offer_that_serves_nothing_or_speaks_another_version_is_none(self):
        for data in (b"\x02\x10\x08\x40", b"\x01\x00\x08\x40", b"\x01\x10\x00\x40",
                     b"\x01\x10\x08\x00", b"\x01\x10\x08", b"", b"\xff\xff\xff\xff\x0f\x10\x08\x40"):
            self.assertIsNone(hz.decode_offer(data), data)

    def test_request_bytes_match_lod_net_encode(self):
        # varint id, i32 x, i32 z (big endian), varint stride, bool approximate.
        data = hz.Request(5, -32, 48, 2, True).encode()
        self.assertEqual(data, b"\x05" + struct.pack(">ii", -32, 48) + b"\x02\x01")
        self.assertEqual(hz.decode_request(data), hz.Request(5, -32, 48, 2, True))
        with self.assertRaises(hz.ProtocolError):
            hz.decode_request(data + b"\x00")

    def test_refusal_is_the_id_then_one(self):
        self.assertEqual(hz.encode_reply(7, None), b"\x07\x01")
        self.assertEqual(hz.decode_reply(b"\x07\x01"), hz.Reply(7, True))

    def test_tile_reply_round_trips(self):
        data = hz.encode_reply(3, tile())
        reply = hz.decode_reply(data)
        self.assertEqual((reply.id, reply.refused, len(reply.columns)), (3, False, 256))
        self.assertEqual(reply.columns[1].surface.height, 61)
        self.assertEqual(reply.columns[5].surface.water, (62, 86))
        self.assertEqual(hz.reply_id(data), 3)

    def test_a_reply_horizon_would_refuse_is_an_error(self):
        good = tile()
        for name, columns in {
            "not a whole tile": good[:255],
            "overlapping spans": [hz.Column(None, (hz.Span(0, 10, 1, "a", 0), hz.Span(5, 12, 1, "a", 0)))]
                                 + good[1:],
            "empty span": [hz.Column(None, (hz.Span(4, 4, 1, "a", 0),))] + good[1:],
            "height out of range": [column(5000)] + good[1:],
            "too many spans": [hz.Column(None, tuple(hz.Span(i, i + 1, 1, "a", 0) for i in range(300)))
                               for _ in range(256)],
        }.items():
            with self.subTest(name), self.assertRaises(hz.ProtocolError):
                hz.decode_reply(hz.encode_reply(1, columns))

    def test_biome_index_outside_the_palette_is_an_error(self):
        data = bytearray(hz.encode_reply(1, tile()))
        # id, 0, palette count 1, "minecraft:plains", 256 columns ...
        palette_end = 2 + 1 + 1 + len("minecraft:plains")
        self.assertEqual(data[2], 1)
        # The first column's surface biome is the last varint of its surface block.
        first_surface = palette_end + 2 + 1 + 4 + 1 + 1 + 4 + 1  # columns(2), flag, h, state, water...
        data[first_surface] = 9
        with self.assertRaises(hz.ProtocolError):
            hz.decode_reply(bytes(data))


class FakeServer:
    """Scripted login/configuration/play for one connection; records what the client sent."""

    def __init__(self, offer: bytes | None):
        self.offer = offer
        self.seen: dict[str, object] = {"config": []}
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(1)
        self.error: BaseException | None = None
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    @property
    def address(self):
        return self.listener.getsockname()

    def run(self):
        try:
            conn, _ = self.listener.accept()
            conn.settimeout(20)
            with conn:
                self.script(conn)
        except BaseException as error:  # reported by the test
            self.error = error
        finally:
            self.listener.close()

    def script(self, conn):
        packet_id, body = wire.read_frame(conn)
        self.seen["handshake_id"] = packet_id
        packet_id, body = wire.read_frame(conn)
        self.seen["login_start_id"] = packet_id
        conn.sendall(wire.frame(wire.LOGIN_CLIENTBOUND_SET_COMPRESSION, wire.write_varint(256)))
        wire.send_compressed_frame(conn, wire.LOGIN_CLIENTBOUND_SUCCESS,
                                   bytes(16) + wire.mc_string("Probe") + wire.write_varint(0))
        packet_id, _ = wire.read_compressed_frame(conn)
        assert packet_id == wire.LOGIN_SERVERBOUND_ACK
        # Horizon's order: brand, then ClientInformation.
        packet_id, body = wire.read_compressed_frame(conn)
        assert packet_id == hz.CFG_SB_CUSTOM_PAYLOAD
        self.seen["brand"] = hz._split_payload(body)
        packet_id, body = wire.read_compressed_frame(conn)
        assert packet_id == hz.CFG_SB_CLIENT_INFORMATION
        self.seen["client_information"] = body
        wire.send_compressed_frame(conn, hz.CFG_CB_CUSTOM_PAYLOAD,
                                   hz._payload(hz.BRAND, wire.mc_string("krasnokamen")))
        if self.offer is not None:
            wire.send_compressed_frame(conn, hz.CFG_CB_CUSTOM_PAYLOAD,
                                       hz._payload(hz.CHANNEL, self.offer))
        wire.send_compressed_frame(conn, hz.CFG_CB_SELECT_KNOWN_PACKS, b"\x00")
        packet_id, _ = wire.read_compressed_frame(conn)
        assert packet_id == hz.CFG_SB_SELECT_KNOWN_PACKS
        wire.send_compressed_frame(conn, hz.CFG_CB_FINISH)
        packet_id, _ = wire.read_compressed_frame(conn)
        assert packet_id == hz.CFG_SB_FINISH
        wire.send_compressed_frame(conn, hz.PLAY_CB_LOGIN, struct.pack(">i", 7))
        wire.send_compressed_frame(conn, hz.PLAY_CB_PLAYER_POSITION,
                                   wire.write_varint(1) + struct.pack(">ddd", 8.5, 70.0, -3.5) + bytes(24)
                                   + struct.pack(">ff", 90.0, 10.0) + bytes(4))
        packet_id, body = wire.read_compressed_frame(conn)
        assert packet_id == hz.PLAY_SB_ACCEPT_TELEPORTATION
        assert body == (wire.write_varint(1) + struct.pack(">dddff", 8.5, 70.0, -3.5, 90.0, 10.0))
        packet_id, _ = wire.read_compressed_frame(conn)
        assert packet_id == hz.PLAY_SB_CLIENT_TICK_END
        wire.send_compressed_frame(conn, hz.PLAY_CB_KEEP_ALIVE, struct.pack(">q", 5))
        packet_id, body = wire.read_compressed_frame(conn)
        assert (packet_id, body) == (hz.PLAY_SB_KEEP_ALIVE, struct.pack(">q", 5))
        # One tile request, answered; one refused.
        for _ in range(2):
            packet_id, body = wire.read_compressed_frame(conn)
            assert packet_id == hz.PLAY_SB_CUSTOM_PAYLOAD
            channel, data = hz._split_payload(body)
            assert channel == hz.CHANNEL
            request = hz.decode_request(data)
            reply = hz.encode_reply(request.id, tile() if request.stride == 1 else None)
            wire.send_compressed_frame(conn, hz.PLAY_CB_CUSTOM_PAYLOAD, hz._payload(hz.CHANNEL, reply))


class ClientTests(unittest.TestCase):
    def test_horizon_handshake_and_tiles_against_a_scripted_server(self):
        server = FakeServer(hz.Offer(16).encode())
        client = hz.HorizonLikeClient(server.address, "Probe", view_distance=4, timeout=20)
        try:
            client.join()
            self.assertEqual(client.configuration_channels(), [hz.BRAND, hz.CHANNEL])
            self.assertEqual(hz.decode_offer(client.offer_payloads()[0]), hz.Offer(16))
            client.wait_ready()
            self.assertEqual(client.position, [8.5, 70.0, -3.5])
            answers = client.fetch([(0, 0, 1), (0, 0, 2)])
            self.assertEqual([a.refused for a in answers.values()], [False, True])
        finally:
            client.close()
        server.thread.join(10)
        self.assertIsNone(server.error)
        self.assertEqual(server.seen["brand"], (hz.BRAND, wire.mc_string("vanilla")))
        self.assertEqual(server.seen["client_information"][:6], wire.mc_string("en_us"))

    def test_no_offer_when_the_server_sends_none(self):
        server = FakeServer(None)
        client = hz.HorizonLikeClient(server.address, "Probe", timeout=20)
        try:
            client.join()
            self.assertEqual(client.offer_payloads(), [])
            client.wait_ready()
            client.fetch([(0, 0, 1), (0, 0, 2)])
        finally:
            client.close()
        server.thread.join(10)


if __name__ == "__main__":
    unittest.main()
