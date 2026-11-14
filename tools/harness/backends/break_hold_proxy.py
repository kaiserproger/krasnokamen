"""BREAK_DL_ATTACK=hold* breaker glue (not for merge): a loopback Minecraft proxy that holds one login
connection at a protocol point.

Offline mode, no encryption. Packet ids are 26.3 (data/vanilla/26.3/reports/packets.json):
login clientbound login_finished 0x02 and login_compression 0x03; login serverbound login_acknowledged
0x03; configuration serverbound finish_configuration 0x03; play serverbound chunk_batch_received 0x0b and
player_loaded 0x2c. Modes: ``gate`` (the first login connection does not reach the server until
released), ``config`` (its finish_configuration acknowledgement is held, so the server keeps it in
configuration), ``chunk`` (its first chunk_batch_received/player_loaded is held, so the first chunk batch
stays unacknowledged). Only client-to-server bytes are held; server-to-client bytes keep flowing.
"""
from __future__ import annotations

import socket
import threading
import time
import zlib
from typing import Any

LOGIN_FINISHED = 0x02
LOGIN_COMPRESSION = 0x03
LOGIN_ACK = 0x03
FINISH_CONFIG = 0x03
CHUNK_BATCH_RECEIVED = 0x0B
PLAYER_LOADED = 0x2C
HOLD_LIMIT_SECONDS = 120.0


def _varint(buf: bytes | bytearray, pos: int) -> tuple[int, int] | None:
    value = shift = 0
    while True:
        if pos >= len(buf):
            return None
        byte = buf[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return (value - (1 << 32) if value >= 1 << 31 else value), pos
        shift += 7
        if shift > 35:
            raise ValueError("varint too long")


def _frame(buf: bytearray) -> tuple[bytes, int] | None:
    head = _varint(buf, 0)
    if head is None:
        return None
    length, start = head
    if len(buf) < start + length:
        return None
    return bytes(buf[start:start + length]), start + length


def _packet(payload: bytes, compressed: bool) -> tuple[int, bytes]:
    body = payload
    if compressed:
        data_length, pos = _varint(payload, 0)
        body = payload[pos:] if data_length == 0 else zlib.decompress(payload[pos:])
    packet_id, pos = _varint(body, 0)
    return packet_id, body[pos:]


class HoldProxy:
    def __init__(self, upstream: str, mode: str) -> None:
        host, port = upstream.rsplit(":", 1)
        self.upstream = (host, int(port))
        self.mode = mode
        self.login_seen = threading.Event()
        self.held = threading.Event()
        self.release = threading.Event()
        self.events: list[dict[str, Any]] = []
        self._claimed = False
        self._lock = threading.Lock()
        self._origin = time.monotonic()
        self.listener = socket.create_server(("127.0.0.1", 0))
        self.address = f"127.0.0.1:{self.listener.getsockname()[1]}"
        threading.Thread(target=self._accept, daemon=True).start()

    def _event(self, conn: int, event: str, **extra: Any) -> None:
        self.events.append({"t": round(time.monotonic() - self._origin, 3), "wall": time.time(),
                            "conn": conn, "event": event, **extra})

    def close(self) -> None:
        self.release.set()
        self.listener.close()

    def _accept(self) -> None:
        index = 0
        while True:
            try:
                client, _ = self.listener.accept()
            except OSError:
                return
            index += 1
            threading.Thread(target=self._serve, args=(client, index), daemon=True).start()

    def _serve(self, client: socket.socket, conn: int) -> None:
        buf = bytearray()
        try:
            while (got := _frame(buf)) is None:
                data = client.recv(65536)
                if not data:
                    client.close()
                    return
                buf += data
            payload, end = got
            _, body = _packet(payload, False)
            _, pos = _varint(body, 0)
            text_length, pos = _varint(body, pos)
            intent, _ = _varint(body, pos + text_length + 2)
        except (OSError, ValueError, TypeError) as error:
            self._event(conn, "bad_handshake", error=str(error))
            client.close()
            return
        login = intent in (2, 3)
        with self._lock:
            hold = login and not self._claimed
            self._claimed = self._claimed or hold
        self._event(conn, "handshake", intent=intent, held_connection=hold)
        if login:
            self.login_seen.set()
        if hold and self.mode == "gate":
            self.held.set()
            self._event(conn, "hold", phase="before_connect")
            released = self.release.wait(HOLD_LIMIT_SECONDS)
            self._event(conn, "release", released=released)
        try:
            upstream = socket.create_connection(self.upstream)
        except OSError as error:
            self._event(conn, "upstream_failed", error=str(error))
            client.close()
            return
        state: dict[str, Any] = {"compressed": False, "phase": "login" if login else "status"}
        threading.Thread(target=self._down, args=(upstream, client, conn, state, login), daemon=True).start()
        self._up(client, upstream, conn, state, buf, hold and self.mode in ("config", "chunk"))

    def _up(self, client: socket.socket, upstream: socket.socket, conn: int, state: dict[str, Any],
            buf: bytearray, hold: bool) -> None:
        try:
            if not hold:
                upstream.sendall(bytes(buf))
                return self._raw(client, upstream, conn, "up")
            _, end = _frame(buf)
            upstream.sendall(bytes(buf[:end]))
            del buf[:end]
            while True:
                while (got := _frame(buf)) is not None:
                    payload, end = got
                    packet_id, _ = _packet(payload, state["compressed"])
                    phase = state["phase"]
                    target = ((self.mode == "config" and phase == "configuration" and packet_id == FINISH_CONFIG)
                              or (self.mode == "chunk" and phase == "play"
                                  and packet_id in (CHUNK_BATCH_RECEIVED, PLAYER_LOADED)))
                    if target:
                        self.held.set()
                        self._event(conn, "hold", phase=phase, packet=hex(packet_id))
                        released = self.release.wait(HOLD_LIMIT_SECONDS)
                        self._event(conn, "release", released=released)
                        upstream.sendall(bytes(buf))
                        buf.clear()
                        return self._raw(client, upstream, conn, "up")
                    upstream.sendall(bytes(buf[:end]))
                    del buf[:end]
                    if phase == "login" and packet_id == LOGIN_ACK:
                        state["phase"] = "configuration"
                        self._event(conn, "phase", phase="configuration")
                    elif phase == "configuration" and packet_id == FINISH_CONFIG:
                        state["phase"] = "play"
                        self._event(conn, "phase", phase="play")
                data = client.recv(65536)
                if not data:
                    break
                buf += data
        except (OSError, ValueError, zlib.error) as error:
            self._event(conn, "up_error", error=str(error))
        self._close(conn, "up", client, upstream)

    def _down(self, upstream: socket.socket, client: socket.socket, conn: int, state: dict[str, Any],
              login: bool) -> None:
        buf = bytearray()
        try:
            while login:
                data = upstream.recv(65536)
                if not data:
                    return self._close(conn, "down", upstream, client)
                buf += data
                while (got := _frame(buf)) is not None:
                    payload, end = got
                    packet_id, body = _packet(payload, state["compressed"])
                    if packet_id == LOGIN_COMPRESSION and not state["compressed"]:
                        state["compressed"] = _varint(body, 0)[0] >= 0
                    client.sendall(bytes(buf[:end]))
                    del buf[:end]
                    if packet_id == LOGIN_FINISHED:
                        login = False
                        break
            client.sendall(bytes(buf))
        except (OSError, ValueError, zlib.error) as error:
            self._event(conn, "down_error", error=str(error))
            return self._close(conn, "down", upstream, client)
        self._raw(upstream, client, conn, "down")

    def _raw(self, src: socket.socket, dst: socket.socket, conn: int, direction: str) -> None:
        try:
            while data := src.recv(65536):
                dst.sendall(data)
        except OSError:
            pass
        self._close(conn, direction, src, dst)

    def _close(self, conn: int, direction: str, *sockets: socket.socket) -> None:
        self._event(conn, "closed", direction=direction)
        for sock in sockets:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
