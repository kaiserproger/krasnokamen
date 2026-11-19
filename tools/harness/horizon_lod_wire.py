"""Horizon's `horizon:lod` far-terrain protocol (VERSION 1) as a client sees it.

Layouts and limits are ported from Horizon's own source (MIT, pin bd431b38):
`crates/horizon-client/src/lod_net.rs` (Offer, Request, reply, limits) and
`lod_net/client.rs` (the client's discipline: 8 requests in flight, 64 a
second), and the configuration/play handshake from `net.rs`. The protocol
authority is that source, never this file; `test_horizon_lod_wire.py` pins the
byte layouts against hand-assembled examples of what `lod_net.rs` writes.

`HorizonLikeClient` joins an offline server the way Horizon's client does:
protocol 777 (26.3), brand `minecraft:brand = vanilla` and a ClientInformation
first in configuration, then answers what the server asks (KnownPacks, Ping,
KeepAlive), records every configuration custom payload (so the Offer shows up
or is provably absent), and in play answers keep-alives, teleports and chunk
batches while it reads `horizon:lod` replies.
"""
from __future__ import annotations

import socket
import struct
import uuid
from dataclasses import dataclass, field
from typing import Any

from . import wire

CHANNEL = "horizon:lod"
VERSION = 1
MAX_DISTANCE = 1024
IN_FLIGHT = 8
PER_SECOND = 64
SIDE = 16
MAX_BIOMES = 256
MAX_BIOME_NAME = 256
MAX_SPANS = 16 * 1024
MAX_REPLY = 512 * 1024
HEIGHTS = range(-2048, 4097)  # lod_net.rs HEIGHTS, inclusive
BRAND = "minecraft:brand"
PROTOCOL = 777

# horizon-protocol ids.rs, protocol 777 (26.3).
CFG_CB_CUSTOM_PAYLOAD = 1
CFG_CB_DISCONNECT = 2
CFG_CB_FINISH = 3
CFG_CB_KEEP_ALIVE = 4
CFG_CB_PING = 5
CFG_CB_SELECT_KNOWN_PACKS = 15
CFG_SB_CLIENT_INFORMATION = 0
CFG_SB_CUSTOM_PAYLOAD = 2
CFG_SB_FINISH = 3
CFG_SB_KEEP_ALIVE = 4
CFG_SB_PONG = 5
CFG_SB_SELECT_KNOWN_PACKS = 7
PLAY_CB_CHUNK_BATCH_FINISHED = 11
PLAY_CB_CUSTOM_PAYLOAD = 24
PLAY_CB_DISCONNECT = 32
PLAY_CB_KEEP_ALIVE = 45
PLAY_CB_LEVEL_CHUNK = 46
PLAY_CB_LOGIN = 50
PLAY_CB_PLAYER_POSITION = 73
PLAY_SB_ACCEPT_TELEPORTATION = 0
PLAY_SB_CHUNK_BATCH_RECEIVED = 11
PLAY_SB_CLIENT_TICK_END = 13
PLAY_SB_CUSTOM_PAYLOAD = 22
PLAY_SB_KEEP_ALIVE = 28


class ProtocolError(RuntimeError):
    """Bytes a Horizon client would refuse (`lod_net::decode_reply` errors)."""


# --------------------------------------------------------------- primitives

def _varint(data: bytes, offset: int) -> tuple[int, int]:
    try:
        return wire.read_varint_bytes(data, offset)
    except RuntimeError as error:
        raise ProtocolError(str(error)) from error


def _take(data: bytes, offset: int, size: int) -> tuple[bytes, int]:
    if offset + size > len(data):
        raise ProtocolError("truncated")
    return data[offset:offset + size], offset + size


def _string(data: bytes, offset: int, limit: int) -> tuple[str, int]:
    length, offset = _varint(data, offset)
    if length > limit * 4:
        raise ProtocolError("string too long")
    raw, offset = _take(data, offset, length)
    try:
        text = raw.decode()
    except UnicodeDecodeError as error:
        raise ProtocolError("string is not UTF-8") from error
    if len(text) > limit:
        raise ProtocolError("string too long")
    return text, offset


# -------------------------------------------------------------------- Offer

@dataclass(frozen=True)
class Offer:
    distance: int
    in_flight: int = IN_FLIGHT
    per_second: int = PER_SECOND

    def encode(self) -> bytes:
        return b"".join(wire.write_varint(v)
                        for v in (VERSION, self.distance, self.in_flight, self.per_second))


def decode_offer(data: bytes) -> Offer | None:
    """`Offer::decode`: None when it serves nothing or speaks another version."""
    values, offset = [], 0
    try:
        for _ in range(4):
            value, offset = wire.read_varint_bytes(data, offset)
            if value >= 1 << 31:  # u32::try_from(i32) fails for negatives
                return None
            values.append(value)
    except RuntimeError:
        return None
    version, distance, in_flight, per_second = values
    if version != VERSION:
        return None
    offer = Offer(min(distance, MAX_DISTANCE), min(in_flight, 64), per_second)
    return offer if offer.distance > 0 and offer.in_flight > 0 and offer.per_second > 0 else None


# ------------------------------------------------------------------ Request

@dataclass(frozen=True)
class Request:
    """One far tile: 16 x 16 columns from block (x, z), each `stride` blocks wide."""
    id: int
    x: int
    z: int
    stride: int
    approximate: bool = False

    def encode(self) -> bytes:
        return (wire.write_varint(self.id) + struct.pack(">ii", self.x, self.z)
                + wire.write_varint(self.stride) + (b"\x01" if self.approximate else b"\x00"))


def decode_request(data: bytes) -> Request:
    request_id, offset = _varint(data, 0)
    raw, offset = _take(data, offset, 8)
    x, z = struct.unpack(">ii", raw)
    stride, offset = _varint(data, offset)
    flag, offset = _take(data, offset, 1)
    if offset != len(data) or flag[0] > 1:
        raise ProtocolError("far terrain request: trailing bytes")
    return Request(request_id, x, z, stride, bool(flag[0]))


# -------------------------------------------------------------------- reply

@dataclass(frozen=True)
class Span:
    min: int
    max: int
    state: int
    biome: str
    light: int


@dataclass(frozen=True)
class Surface:
    height: int
    state: int
    water: tuple[int, int] | None
    biome: str


@dataclass(frozen=True)
class Column:
    surface: Surface | None
    spans: tuple[Span, ...]

    @property
    def empty(self) -> bool:
        return self.surface is None and not self.spans


@dataclass(frozen=True)
class Reply:
    id: int
    refused: bool
    columns: tuple[Column, ...] = ()


def reply_id(data: bytes) -> int:
    return _varint(data, 0)[0]


def decode_reply(data: bytes, states: int = 1 << 20) -> Reply:
    """`lod_net::decode_reply`: every check Horizon's client makes before it
    spends memory on a reply. `states` is the client's block-state count; the
    harness has no 26.3 registry, so it bounds ids by a generous constant."""
    if len(data) > MAX_REPLY:
        raise ProtocolError("far terrain: reply too big")
    request_id, offset = _varint(data, 0)
    flag, offset = _take(data, offset, 1)
    if flag[0] != 0:
        return Reply(request_id, True)
    count, offset = _varint(data, offset)
    if not 0 <= count <= MAX_BIOMES:
        raise ProtocolError("far terrain: too many biomes")
    palette = []
    for _ in range(count):
        name, offset = _string(data, offset, MAX_BIOME_NAME)
        palette.append(name)

    def biome(offset: int) -> tuple[str, int]:
        index, offset = _varint(data, offset)
        if index >= len(palette):
            raise ProtocolError("far terrain: biome out of the palette")
        return palette[index], offset

    def state(offset: int) -> tuple[int, int]:
        value, offset = _varint(data, offset)
        if value >= states:
            raise ProtocolError("far terrain: unknown block state")
        return value, offset

    def height(offset: int) -> tuple[int, int]:
        raw, offset = _take(data, offset, 4)
        value = struct.unpack(">i", raw)[0]
        if value not in HEIGHTS:
            raise ProtocolError("far terrain: height out of range")
        return value, offset

    columns, offset = _varint(data, offset)
    if columns != SIDE * SIDE:
        raise ProtocolError("far terrain: not a whole tile")
    out: list[Column] = []
    total = 0
    for _ in range(columns):
        flag, offset = _take(data, offset, 1)
        surface = None
        if flag[0]:
            top, offset = height(offset)
            top_state, offset = state(offset)
            flag, offset = _take(data, offset, 1)
            water = None
            if flag[0]:
                water_height, offset = height(offset)
                water_state, offset = state(offset)
                water = (water_height, water_state)
            surface_biome, offset = biome(offset)
            surface = Surface(top, top_state, water, surface_biome)
        span_count, offset = _varint(data, offset)
        total += span_count
        if total > MAX_SPANS:
            raise ProtocolError("far terrain: too many spans")
        spans: list[Span] = []
        for _ in range(span_count):
            raw, offset = _take(data, offset, 4)
            low, high = struct.unpack(">hh", raw)
            span_state, offset = state(offset)
            span_biome, offset = biome(offset)
            raw, offset = _take(data, offset, 2)
            light = struct.unpack(">H", raw)[0]
            floor = spans[-1].max if spans else HEIGHTS.start
            if low < floor or low >= high or high not in HEIGHTS:
                raise ProtocolError("far terrain: spans out of order")
            spans.append(Span(low, high, span_state, span_biome, light))
        out.append(Column(surface, tuple(spans)))
    return Reply(request_id, False, tuple(out))


def encode_reply(request_id: int, columns: list[Column] | None) -> bytes:
    """`lod_net::encode_reply` for tests and fake servers: None refuses."""
    out = wire.write_varint(request_id)
    if columns is None:
        return out + b"\x01"
    names: list[str] = []

    def local(name: str) -> bytes:
        if name not in names:
            names.append(name)
        return wire.write_varint(names.index(name))

    body = wire.write_varint(len(columns))
    for column in columns:
        if column.surface is None:
            body += b"\x00"
        else:
            surface = column.surface
            body += b"\x01" + struct.pack(">i", surface.height) + wire.write_varint(surface.state)
            if surface.water is None:
                body += b"\x00"
            else:
                body += (b"\x01" + struct.pack(">i", surface.water[0])
                         + wire.write_varint(surface.water[1]))
            body += local(surface.biome)
        body += wire.write_varint(len(column.spans))
        for span in column.spans:
            body += (struct.pack(">hh", span.min, span.max) + wire.write_varint(span.state)
                     + local(span.biome) + struct.pack(">H", span.light))
    out += b"\x00" + wire.write_varint(len(names))
    for name in names:
        out += wire.mc_string(name)
    return out + body


# ------------------------------------------------------------ the connection

def _payload(channel: str, data: bytes) -> bytes:
    return wire.mc_string(channel) + data


def _split_payload(body: bytes) -> tuple[str, bytes]:
    length, offset = _varint(body, 0)
    raw, offset = _take(body, offset, length)
    return raw.decode(), body[offset:]


def client_information(view_distance: int) -> bytes:
    """`ClientInformation::new(view_distance)` as Horizon writes it."""
    return (wire.mc_string("en_us") + struct.pack(">b", view_distance) + wire.write_varint(0)
            + b"\x01" + b"\x7f" + wire.write_varint(1) + b"\x00" + b"\x01" + wire.write_varint(0))


@dataclass
class HorizonLikeClient:
    """A protocol-level stand-in for Horizon's client (see the module text)."""
    address: tuple[str, int]
    name: str
    view_distance: int = 4
    brand: str = "vanilla"
    timeout: float = 60.0
    protocol: int = PROTOCOL
    sock: socket.socket | None = None
    config_payloads: list[tuple[str, bytes]] = field(default_factory=list)
    replies: dict[int, bytes] = field(default_factory=dict)
    position: list[float] | None = None
    chunks: int = 0
    keepalives: int = 0
    play: bool = False
    next_id: int = 0
    # Keep each received chunk's palettes (protocol_chunks.decode_chunk) for oracle use.
    store_chunks: bool = False
    chunk_sections: dict = field(default_factory=dict)

    # -- joining
    def join(self) -> dict[str, Any]:
        sock = socket.create_connection(self.address, timeout=self.timeout)
        self.sock = sock
        sock.settimeout(self.timeout)
        host, port = self.address
        sock.sendall(wire.frame(0, wire.handshake(host, port, 2, protocol=self.protocol)))
        sock.sendall(wire.frame(wire.LOGIN_SERVERBOUND_START,
                                wire.mc_string(self.name) + uuid.uuid4().bytes))
        packet_id, body = wire.read_frame(sock)
        if packet_id == wire.LOGIN_CLIENTBOUND_DISCONNECT:
            raise ProtocolError(f"login disconnected: {body[:200]!r}")
        if packet_id != wire.LOGIN_CLIENTBOUND_SET_COMPRESSION:
            raise ProtocolError(f"expected SetCompression, got {packet_id:#x}")
        packet_id, body = wire.read_compressed_frame(sock)
        if packet_id != wire.LOGIN_CLIENTBOUND_SUCCESS:
            raise ProtocolError(f"expected LoginSuccess, got {packet_id:#x}")
        wire.send_compressed_frame(sock, wire.LOGIN_SERVERBOUND_ACK)
        # handleLoginFinished: the brand first, then the information (net.rs).
        wire.send_compressed_frame(sock, CFG_SB_CUSTOM_PAYLOAD,
                                   _payload(BRAND, wire.mc_string(self.brand)))
        wire.send_compressed_frame(sock, CFG_SB_CLIENT_INFORMATION,
                                   client_information(self.view_distance))
        frames = 0
        while True:
            packet_id, body = wire.read_compressed_frame(sock)
            frames += 1
            if packet_id == CFG_CB_CUSTOM_PAYLOAD:
                self.config_payloads.append(_split_payload(body))
            elif packet_id == CFG_CB_DISCONNECT:
                raise ProtocolError(f"disconnected in configuration: {body[:200]!r}")
            elif packet_id == CFG_CB_KEEP_ALIVE:
                wire.send_compressed_frame(sock, CFG_SB_KEEP_ALIVE, body)
            elif packet_id == CFG_CB_PING:
                wire.send_compressed_frame(sock, CFG_SB_PONG, body)
            elif packet_id == CFG_CB_SELECT_KNOWN_PACKS:
                wire.send_compressed_frame(sock, CFG_SB_SELECT_KNOWN_PACKS, body)
            elif packet_id == CFG_CB_FINISH:
                wire.send_compressed_frame(sock, CFG_SB_FINISH)
                break
            if frames > 4096:
                raise ProtocolError("no FinishConfiguration")
        while True:
            packet_id, body = self.read_play()
            if packet_id == PLAY_CB_LOGIN:
                self.play = True
                return {"entity_id": struct.unpack_from(">i", body)[0],
                        "configuration_frames": frames}

    # -- play
    def configuration_channels(self) -> list[str]:
        return [channel for channel, _ in self.config_payloads]

    def offer_payloads(self) -> list[bytes]:
        return [data for channel, data in self.config_payloads if channel == CHANNEL]

    def read_play(self) -> tuple[int, bytes]:
        """The next play frame, after the answers a client owes the server."""
        assert self.sock is not None
        packet_id, body = wire.read_compressed_frame(self.sock)
        send = lambda i, b=b"": wire.send_compressed_frame(self.sock, i, b)  # noqa: E731
        if packet_id == PLAY_CB_KEEP_ALIVE:
            self.keepalives += 1
            send(PLAY_SB_KEEP_ALIVE, body)
        elif packet_id == PLAY_CB_PLAYER_POSITION:
            teleport, offset = _varint(body, 0)
            self.position = list(struct.unpack_from(">ddd", body, offset))
            yaw, pitch = struct.unpack_from(">ff", body, offset + 48)
            # horizon-protocol AcceptTeleportation: id, x, y, z, yaw, pitch (26.3).
            send(PLAY_SB_ACCEPT_TELEPORTATION,
                 wire.write_varint(teleport) + struct.pack(">dddff", *self.position, yaw, pitch))
            send(PLAY_SB_CLIENT_TICK_END)
        elif packet_id == PLAY_CB_CHUNK_BATCH_FINISHED:
            send(PLAY_SB_CHUNK_BATCH_RECEIVED, struct.pack(">f", 64.0))
        elif packet_id == PLAY_CB_LEVEL_CHUNK:
            self.chunks += 1
            if self.store_chunks:
                from .protocol_chunks import decode_chunk
                chunk, sections = decode_chunk(body)
                self.chunk_sections[chunk] = sections
        elif packet_id == PLAY_CB_DISCONNECT:
            raise ProtocolError(f"disconnected in play: {body[:512].hex()}")
        elif packet_id == PLAY_CB_CUSTOM_PAYLOAD:
            channel, data = _split_payload(body)
            if channel == CHANNEL:
                self.replies[reply_id(data)] = data
        return packet_id, body

    def pump_until(self, predicate) -> None:
        while not predicate():
            self.read_play()

    def next_keepalive(self) -> None:
        """Block until the server's next KeepAlive (a barrier: everything the
        server wrote before it, in order, has been read)."""
        target = self.keepalives + 1
        self.pump_until(lambda: self.keepalives >= target)

    def request(self, x: int, z: int, stride: int = 1, *, approximate: bool = False) -> int:
        assert self.sock is not None
        request = Request(self.next_id, x, z, stride, approximate)
        self.next_id += 1
        wire.send_compressed_frame(self.sock, PLAY_SB_CUSTOM_PAYLOAD,
                                   _payload(CHANNEL, request.encode()))
        return request.id

    def send_raw_request(self, data: bytes) -> None:
        assert self.sock is not None
        wire.send_compressed_frame(self.sock, PLAY_SB_CUSTOM_PAYLOAD, _payload(CHANNEL, data))

    def fetch(self, tiles: list[tuple[int, int, int]]) -> dict[int, Reply | None]:
        """Ask for tiles within Horizon's own limits (at most IN_FLIGHT waiting),
        wait for each answer, decode with Horizon's checks. Keys are request
        ids in `tiles` order starting at the current id."""
        answers: dict[int, Reply | None] = {}
        for start in range(0, len(tiles), IN_FLIGHT):
            ids = [self.request(x, z, stride) for x, z, stride in tiles[start:start + IN_FLIGHT]]
            self.pump_until(lambda: all(i in self.replies for i in ids))
            for i in ids:
                answers[i] = decode_reply(self.replies.pop(i))
        return answers

    def close(self) -> None:
        if self.sock is not None:
            try:
                self.sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            self.sock.close()
            self.sock = None

    def wait_ready(self) -> None:
        """The join burst is over: the spawn position and the first KeepAlive arrived."""
        self.pump_until(lambda: self.position is not None and self.keepalives >= 1)
