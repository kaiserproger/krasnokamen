"""Tracked 26.1.2 wire framing, Status and offline-join helpers.

Layouts follow ``crates/mc-protocol`` (framing/compression, Status, Login,
Configuration and the Play ``LoginPlay`` id); the packet ids are asserted by
that crate's tests. Only what harness gates need lives here, so scenarios do
not depend on ignored ``.analysis`` evidence files.
"""

from __future__ import annotations

import json
import socket
import struct
import uuid
import zlib
from typing import Any

RELEASE = "26.1.2"
PROTOCOL = 775

# crates/mc-protocol packet ids used by the offline join below.
LOGIN_CLIENTBOUND_DISCONNECT = 0x00
LOGIN_CLIENTBOUND_SUCCESS = 0x02
LOGIN_CLIENTBOUND_SET_COMPRESSION = 0x03
LOGIN_SERVERBOUND_START = 0x00
LOGIN_SERVERBOUND_ACK = 0x03
CONFIG_SERVERBOUND_KNOWN_PACKS = 0x07
CONFIG_SERVERBOUND_ACK_FINISH = 0x03
CONFIG_CLIENTBOUND_FINISH = 0x03
CONFIG_CLIENTBOUND_KNOWN_PACKS = 0x0E
PLAY_CLIENTBOUND_LOGIN = 0x31
PLAY_CLIENTBOUND_KEEPALIVE = 0x2C


def write_varint(value: int) -> bytes:
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


def read_exact(sock: socket.socket, length: int) -> bytes:
    result = bytearray()
    while len(result) < length:
        chunk = sock.recv(length - len(result))
        if not chunk:
            raise RuntimeError("peer closed before complete frame")
        result.extend(chunk)
    return bytes(result)


def read_varint(sock: socket.socket) -> int:
    value = 0
    for shift in range(0, 35, 7):
        part = read_exact(sock, 1)[0]
        value |= (part & 0x7F) << shift
        if not part & 0x80:
            return value
    raise RuntimeError("overlong VarInt")


def read_varint_bytes(data: bytes, offset: int = 0) -> tuple[int, int]:
    value = 0
    for shift in range(0, 35, 7):
        if offset >= len(data):
            raise RuntimeError("truncated VarInt")
        part = data[offset]
        offset += 1
        value |= (part & 0x7F) << shift
        if not part & 0x80:
            return value, offset
    raise RuntimeError("overlong VarInt")


def frame(packet_id: int, body: bytes = b"") -> bytes:
    payload = write_varint(packet_id) + body
    return write_varint(len(payload)) + payload


def read_frame(sock: socket.socket) -> tuple[int, bytes]:
    """One uncompressed frame (login before ``Set Compression``)."""
    length = read_varint(sock)
    payload = read_exact(sock, length)
    packet_id, offset = read_varint_bytes(payload)
    return packet_id, payload[offset:]


def mc_string(value: str) -> bytes:
    data = value.encode()
    return write_varint(len(data)) + data


def read_mc_string(data: bytes) -> str:
    length, offset = read_varint_bytes(data)
    end = offset + length
    if end != len(data):
        raise RuntimeError("unexpected trailing bytes in string packet")
    return data[offset:end].decode()


def handshake(host: str, port: int, next_state: int, protocol: int = PROTOCOL) -> bytes:
    return (
        write_varint(protocol)
        + mc_string(host)
        + struct.pack(">H", port)
        + write_varint(next_state)
    )


def read_compressed_frame(sock: socket.socket) -> tuple[int, bytes]:
    """One frame after ``Set Compression``: length, data_length, body."""
    length = read_varint(sock)
    payload = read_exact(sock, length)
    data_length, offset = read_varint_bytes(payload)
    body = payload[offset:] if data_length == 0 else zlib.decompress(payload[offset:])
    packet_id, offset = read_varint_bytes(body)
    return packet_id, body[offset:]


def send_compressed_frame(
    sock: socket.socket, packet_id: int, body: bytes = b""
) -> None:
    payload = write_varint(0) + write_varint(packet_id) + body
    sock.sendall(write_varint(len(payload)) + payload)


def probe_status(address: tuple[str, int], *, timeout: float = 10.0) -> dict[str, Any]:
    """Status request + ping/pong; asserts the supported release metadata."""
    with socket.create_connection(address, timeout=timeout) as sock:
        sock.settimeout(timeout)
        sock.sendall(frame(0, handshake("localhost", 25565, 1)))
        sock.sendall(frame(0))
        packet_id, body = read_frame(sock)
        if packet_id != 0:
            raise RuntimeError(f"unexpected status packet id {packet_id:#x}")
        status = json.loads(read_mc_string(body))
        if status["version"] != {"name": RELEASE, "protocol": PROTOCOL}:
            raise RuntimeError(f"wrong supported metadata: {status['version']!r}")
        sock.sendall(frame(1, struct.pack(">q", 0x102030405060708)))
        packet_id, body = read_frame(sock)
        if packet_id != 1 or body != struct.pack(">q", 0x102030405060708):
            raise RuntimeError("ping/pong was not preserved")
        return status


def join_offline_socket(
    address: tuple[str, int], name: str, *, timeout: float = 120.0,
    account_uuid: uuid.UUID | None = None, protocol: int = PROTOCOL,
) -> tuple[socket.socket, dict[str, Any]]:
    """Join an offline server through LoginPlay and return the live Play socket.

    The caller owns and must close the returned socket.
    """
    host, port = address
    seen: list[int] = []
    sock = socket.create_connection(address, timeout=timeout)
    try:
        sock.settimeout(timeout)
        sock.sendall(frame(0, handshake(host, port, 2, protocol=protocol)))
        sock.sendall(
            frame(LOGIN_SERVERBOUND_START, mc_string(name) + (account_uuid or uuid.uuid4()).bytes)
        )

        packet_id, body = read_frame(sock)
        if packet_id == LOGIN_CLIENTBOUND_DISCONNECT:
            raise RuntimeError(f"login disconnected: {body[:200]!r}")
        if packet_id != LOGIN_CLIENTBOUND_SET_COMPRESSION:
            raise RuntimeError(f"expected SetCompression, got {packet_id:#x}")
        threshold, _ = read_varint_bytes(body)
        packet_id, body = read_compressed_frame(sock)
        if packet_id != LOGIN_CLIENTBOUND_SUCCESS:
            raise RuntimeError(
                f"expected LoginSuccess, got {packet_id:#x} (compression {threshold})"
            )
        accepted_uuid = str(uuid.UUID(bytes=body[:16]))
        send_compressed_frame(sock, LOGIN_SERVERBOUND_ACK)

        while True:
            packet_id, body = read_compressed_frame(sock)
            seen.append(packet_id)
            if packet_id == (0x0F if protocol == 777 else CONFIG_CLIENTBOUND_KNOWN_PACKS):
                # Echo the advertised pack: the client then "owns"
                # minecraft:core:<release>; registry names are still received.
                send_compressed_frame(sock, CONFIG_SERVERBOUND_KNOWN_PACKS, body)
                break
            if len(seen) > 64:
                raise RuntimeError(f"no KnownPacks in configuration: {seen}")

        seen = []
        registries = {}
        while True:
            packet_id, body = read_compressed_frame(sock)
            seen.append(packet_id)
            if packet_id == 0x07:  # clientbound configuration RegistryData
                from .protocol_observations import decode_registry
                registry_name, entries = decode_registry(body)
                registries[registry_name] = {
                    str(numeric_id): name for numeric_id, name in enumerate(entries)}
            if packet_id == CONFIG_CLIENTBOUND_FINISH:
                break
            if len(seen) > 256:
                raise RuntimeError(f"no FinishConfiguration: {seen}")
        send_compressed_frame(sock, CONFIG_SERVERBOUND_ACK_FINISH)
        config_frames = len(seen)

        seen = []
        while True:
            packet_id, body = read_compressed_frame(sock)
            seen.append(packet_id)
            if packet_id == (0x32 if protocol == 777 else PLAY_CLIENTBOUND_LOGIN):
                return sock, {
                    "name": name,
                    "entity_id": struct.unpack_from(">i", body)[0],
                    "uuid": accepted_uuid,
                    "compression_threshold": threshold,
                    "configuration_frames": config_frames,
                    "registries": registries,
                    "play_frames_before_login": len(seen) - 1,
                }
            if len(seen) > 512:
                raise RuntimeError(f"no LoginPlay after configuration: {seen}")
    except BaseException:
        sock.close()
        raise


def wait_play_loop(sock: socket.socket, *, timeout: float = 120.0) -> None:
    """Block until the first Play KeepAlive.

    The keepalive is written by the play writer after the initial join burst,
    so receiving it proves the connection reached the ordinary play loop and a
    later close takes the normal leave path.
    """
    sock.settimeout(timeout)
    while True:
        packet_id, _ = read_compressed_frame(sock)
        if packet_id == PLAY_CLIENTBOUND_KEEPALIVE:
            return


def join_offline(
    address: tuple[str, int], name: str, *, timeout: float = 120.0
) -> dict[str, Any]:
    """Offline Login -> Configuration -> first Play ``LoginPlay`` frame."""
    sock, result = join_offline_socket(address, name, timeout=timeout)
    sock.close()
    return result


class UnsupportedObservation(RuntimeError):
    """The wire observer cannot prove an oracle for this payload."""


def pack_block_position(position: tuple[int, int, int]) -> bytes:
    x, y, z = position
    return struct.pack(">Q", ((x & 0x3FFFFFF) << 38) | ((z & 0x3FFFFFF) << 12) | (y & 0xFFF))


def click_body(window: int, state_id: int, slot: int, button: int, mode: int) -> bytes:
    """ContainerClick: empty changed-slot map and empty HashedStack prediction."""
    if mode not in range(7):
        raise ValueError(f"unknown click mode {mode}")
    return (write_varint(window) + write_varint(state_id) + struct.pack(">hb", slot, button)
            + write_varint(mode) + b"\x00\x00")


def decode_section_blocks(body: bytes) -> dict[tuple[int, int, int], int]:
    """Decode SectionBlocksUpdate 0x54 (mc-protocol chunks.rs), including VarLong."""
    if len(body) < 9:
        raise RuntimeError('truncated section block update')
    section = struct.unpack_from('>Q', body)[0]
    def signed(value, bits):
        return value - (1 << bits) if value & (1 << (bits - 1)) else value
    x = signed(section >> 42, 22) * 16
    z = signed((section >> 20) & 0x3fffff, 22) * 16
    y = signed(section & 0xfffff, 20) * 16
    count, offset = read_varint_bytes(body, 8)
    if not 0 <= count <= 4096:
        raise RuntimeError('invalid section block count')
    result = {}
    for _ in range(count):
        value = 0
        for shift in range(0, 70, 7):
            if offset >= len(body):
                raise RuntimeError('truncated section block VarLong')
            byte = body[offset]; offset += 1
            if shift == 63 and byte > 1:
                raise RuntimeError('overflow section block VarLong')
            value |= (byte & 127) << shift
            if not byte & 128:
                break
        else:
            raise RuntimeError('overlong section block VarLong')
        state = value >> 12
        if state > 0x7fffffff:
            raise RuntimeError('invalid section block state')
        result[(x + ((value >> 8) & 15), y + (value & 15), z + ((value >> 4) & 15))] = state
    if offset != len(body):
        raise RuntimeError('trailing section block update bytes')
    return result


def play_packet_id(packet_id: int, *, inbound: bool, protocol: int) -> int:
    """Map decoder ids using local reports/packets.json for protocols 775/777."""
    if protocol == 775:
        return packet_id
    if protocol != 777:
        raise UnsupportedObservation(f"unsupported wire protocol: {protocol}")
    if inbound:
        if packet_id in {37, 83, 123}:  # new packets, never feed old decoders
            return -1
        return packet_id - (packet_id > 37) - (packet_id > 83) - (packet_id > 123)
    if 46 <= packet_id <= 61:
        return packet_id + 1
    if packet_id == 63:  # swing became punch
        return 46
    if packet_id in {62, 64}:
        raise UnsupportedObservation("26.3 spectator_action body unavailable")
    return packet_id


class PlayObserver:
    """Event-driven public-wire observer for plain-stack custody probes.

    Existing Rust mechanic-spec remains the component/entity observer. This
    helper extends the offline socket used by pause-empty with KeepAlive,
    teleport/chunk acknowledgements and received menu images. Every waiter
    uses a condition notification from actual packets, never timed polling.
    """

    def __init__(self, address: tuple[str, int], name: str, *, account_uuid: uuid.UUID,
                 timeout: float = 60.0, trace_path=None, protocol: int = PROTOCOL):
        import collections
        import threading
        self.sock, self.join = join_offline_socket(address, name, timeout=timeout,
                                                  account_uuid=account_uuid, protocol=protocol)
        self.sock.settimeout(timeout)
        self.timeout = timeout
        self.protocol = protocol
        self.condition = threading.Condition()
        self.write_lock = threading.Lock()
        self.frames = collections.Counter()
        self.revision = 0
        self.menus: dict[int, dict[str, Any]] = {}
        self.menu_errors: dict[int, str] = {}
        self.events: list[str] = []
        self.chat_components = collections.deque(maxlen=256)
        self.chat_errors = collections.deque(maxlen=256)
        self.merchant_offers: dict[int, dict] = {}
        self.merchant_errors: dict[int, str] = {}
        self.menu_types: dict[int, int] = {}
        self.menu_properties: dict[int, dict[int, int]] = {}
        from .protocol_entities import ProtocolEntities
        from .protocol_observations import decode_stack
        self.entities = ProtocolEntities(decode_stack=decode_stack,
                                         local_entity_id=self.join.get('entity_id'))
        self.health = None
        self.food = None
        self.game_time = None
        self.block_ack_sequence = None
        self.blocks: dict[tuple[int, int, int], int] = {}
        self.passengers: dict[int, list[int]] = {}
        self.chunks: set[tuple[int, int]] = set()
        self.chunk_sections = {}
        self.chunk_min_y = None
        self.current_window = 0
        self.position = None
        self.teleports = 0
        self.closed = False
        self.error = None
        self.sequence = 0
        self.trace = open(trace_path, "w") if trace_path else None
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def send(self, packet_id: int, body: bytes = b"") -> None:
        with self.write_lock:
            send_compressed_frame(self.sock, play_packet_id(packet_id, inbound=False, protocol=self.protocol), body)

    def _read(self) -> None:
        from .protocol_observations import (decode_menu, decode_stack,
                                            decode_merchant_offers, decode_chat_component)
        from .protocol_entities import HANDLED_IDS
        try:
            while not self.closed:
                packet_id, body = read_compressed_frame(self.sock)
                packet_id = play_packet_id(packet_id, inbound=True, protocol=self.protocol)
                with self.condition:
                    self.frames[packet_id] += 1
                    if self.trace:
                        self.trace.write(json.dumps({"id": packet_id, "bytes": len(body),
                                                     "prefix": body[:96].hex()}) + "\n")
                        self.trace.flush()
                    self.revision += 1
                    if packet_id in HANDLED_IDS:
                        self.entities.consume(packet_id, body)
                    if packet_id == PLAY_CLIENTBOUND_KEEPALIVE:
                        self.send(0x1C, body)
                    elif packet_id == 0x48:  # SynchronizePlayerPosition, login_flow.rs
                        teleport, offset = read_varint_bytes(body)
                        self.position = list(struct.unpack_from(">ddd", body, offset))
                        self.teleports += 1
                        if self.protocol == 777:
                            yaw, pitch = struct.unpack_from(">ff", body, offset + 48)
                            self.send(0x00, write_varint(teleport) + struct.pack(">dddff", *self.position, yaw, pitch))
                        else:
                            self.send(0x00, write_varint(teleport))
                            self.send(0x1E, struct.pack(">dddB", *self.position, 1))
                        self.send(0x0D)
                    elif packet_id == 0x6B:  # SetPassengers, entity.rs
                        vehicle, offset = read_varint_bytes(body)
                        count, offset = read_varint_bytes(body, offset)
                        if not 0 <= count <= 1024:
                            raise RuntimeError("invalid passenger count")
                        passengers = []
                        for _ in range(count):
                            passenger, offset = read_varint_bytes(body, offset)
                            passengers.append(passenger)
                        if offset != len(body):
                            raise RuntimeError("trailing passenger packet bytes")
                        self.passengers[vehicle] = passengers
                    elif packet_id == 0x0B:  # ChunkBatchFinished
                        self.send(0x0B, struct.pack(">f", 64.0))
                    elif packet_id == 0x2D:  # LevelChunkWithLight
                        from .protocol_chunks import decode_chunk
                        chunk, sections = decode_chunk(body)
                        self.chunk_sections[chunk] = sections
                        self.blocks = {position: state for position, state in self.blocks.items()
                                       if (position[0] // 16, position[2] // 16) != chunk}
                        self.chunks.add(chunk)
                    elif packet_id == 0x25:  # ForgetLevelChunk: x low32, z high32
                        packed = struct.unpack_from(">Q", body)[0]
                        x, z = packed & 0xFFFFFFFF, packed >> 32
                        chunk = (x - (1 << 32) if x & (1 << 31) else x,
                                 z - (1 << 32) if z & (1 << 31) else z)
                        self.chunks.discard(chunk)
                        self.chunk_sections.pop(chunk, None)
                        self.blocks = {position: state for position, state in self.blocks.items()
                                       if (position[0] // 16, position[2] // 16) != chunk}
                    elif packet_id == 0x52:  # Respawn/dimension switch invalidates residency
                        self.chunks.clear()
                        self.blocks.clear()
                        self.chunk_sections.clear()
                    elif packet_id == 0x3B:  # OpenScreen, inventory.rs
                        self.current_window, offset = read_varint_bytes(body)
                        self.menu_types[self.current_window], _ = read_varint_bytes(body, offset)
                    elif packet_id == 0x12:
                        window, _ = read_varint_bytes(body)
                        try:
                            self.menus[window] = decode_menu(body)
                            self.menu_errors.pop(window, None)
                            self._cache_player_suffix(window)
                        except UnsupportedObservation as error:
                            self.menu_errors[window] = str(error)
                    elif packet_id == 0x13:  # ContainerSetData
                        window, offset = read_varint_bytes(body)
                        key, value = struct.unpack_from(">hh", body, offset)
                        self.menu_properties.setdefault(window, {})[key] = value
                    elif packet_id == 0x60:  # SetCursorItem
                        try:
                            stack, offset = decode_stack(body, 0)
                            if offset != len(body):
                                raise RuntimeError('trailing cursor update bytes')
                            if self.current_window in self.menus:
                                self.menus[self.current_window]["cursor"] = stack
                        except UnsupportedObservation as error:
                            self.menu_errors[self.current_window] = str(error)
                    elif packet_id == 0x68:  # SetHealth, env_state.rs
                        self.health = struct.unpack_from(">f", body)[0]
                        self.food, _ = read_varint_bytes(body, 4)
                    elif packet_id == 0x71:  # SetTime, env_state.rs
                        self.game_time = struct.unpack_from(">q", body)[0]
                    elif packet_id == 0x34:  # MerchantOffers
                        window, _ = read_varint_bytes(body)
                        try:
                            self.merchant_offers[window] = decode_merchant_offers(body)
                            self.merchant_errors.pop(window, None)
                        except UnsupportedObservation as error:
                            self.merchant_errors[window] = str(error)
                    elif packet_id == 0x79:  # SystemChat: real component, no raw NBT garbage
                        try:
                            component = decode_chat_component(body)
                            self.chat_components.append(component)
                            self.events.append(component['text'] or json.dumps(component, ensure_ascii=False))
                        except UnsupportedObservation as error:
                            self.chat_errors.append(str(error))
                    elif packet_id == 0x04:  # BlockChangedAck: admission, not success
                        sequence, offset = read_varint_bytes(body)
                        if offset != len(body):
                            raise RuntimeError('trailing block acknowledgement bytes')
                        self.block_ack_sequence = sequence
                    elif packet_id == 0x54:  # SectionBlocksUpdate
                        self.blocks.update(decode_section_blocks(body))
                    elif packet_id == 0x08:  # BlockUpdate
                        packed = struct.unpack_from(">Q", body)[0]
                        x, z, y = (packed >> 38) & 0x3FFFFFF, (packed >> 12) & 0x3FFFFFF, packed & 0xFFF
                        position = (x - (1 << 26) if x & (1 << 25) else x,
                                    y - 4096 if y & 2048 else y,
                                    z - (1 << 26) if z & (1 << 25) else z)
                        self.blocks[position], _ = read_varint_bytes(body, 8)
                    elif packet_id == 0x14:  # ContainerSetSlot
                        window, offset = read_varint_bytes(body)
                        state_id, offset = read_varint_bytes(body, offset)
                        slot = struct.unpack_from(">h", body, offset)[0]
                        try:
                            stack, offset = decode_stack(body, offset + 2)
                            if offset != len(body):
                                raise RuntimeError("trailing slot update bytes")
                            if window in self.menus and 0 <= slot < len(self.menus[window]["slots"]):
                                self.menus[window]["slots"][slot] = stack
                                self.menus[window]["state_id"] = state_id
                                self._cache_player_suffix(window)
                        except UnsupportedObservation as error:
                            self.menu_errors[window] = str(error)
                    elif packet_id == 0x20:  # PlayDisconnect, login_flow.rs
                        raise RuntimeError(f"server disconnect: {body[:512].hex()}")
                    self.condition.notify_all()
        except BaseException as error:
            with self.condition:
                self.error = str(error)
                self.condition.notify_all()

    def wait(self, predicate, *, description: str, timeout: float | None = None):
        import time
        deadline = time.monotonic() + (self.timeout if timeout is None else timeout)
        with self.condition:
            while True:
                result = predicate(self)
                if result:
                    return result
                if self.error:
                    raise RuntimeError(f"{description}: {self.error}")
                left = deadline - time.monotonic()
                if left <= 0:
                    raise TimeoutError(f"{description}; frames={dict(self.frames)}, menu_errors={self.menu_errors}")
                self.condition.wait(left)

    def ready(self) -> None:
        self.wait(lambda p: p.position is not None and p.frames[PLAY_CLIENTBOUND_KEEPALIVE],
                  description="initial position and ordinary play loop")

    def use_block(self, position: tuple[int, int, int]) -> None:
        self.sequence += 1
        self.send(0x42, b"\x00" + pack_block_position(position) + b"\x01"
                  + struct.pack(">fff", 0.5, 0.5, 0.5) + b"\x00\x00"
                  + write_varint(self.sequence))
        self.send(0x0D)

    def click(self, window: int, slot: int, *, button: int = 0, mode: int = 0,
              state_id: int | None = None) -> None:
        with self.condition:
            state = self.menus[window]["state_id"] if state_id is None else state_id
        self.send(0x12, click_body(window, state, slot, button, mode))
        self.send(0x0D)

    def _cache_player_suffix(self, window: int) -> None:
        """Received container suffix remains authoritative after menu closure."""
        player, menu = self.menus.get(0), self.menus.get(window)
        if (window != 0 and player is not None and menu is not None
                and len(player["slots"]) >= 45 and len(menu["slots"]) >= 36):
            player["slots"][9:36] = menu["slots"][-36:-9]
            player["slots"][36:45] = menu["slots"][-9:]

    def close_menu(self, window: int) -> None:
        self.send(0x13, write_varint(window))
        self.send(0x0D)
        self.current_window = 0

    def close(self) -> None:
        self.closed = True
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.sock.close()
        self.reader.join(timeout=5)
        if self.trace:
            self.trace.close()
