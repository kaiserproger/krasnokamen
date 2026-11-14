"""Order one declared server fixture immediately after a physical menu click.

Unlike the ordinary unmodified wire observer, this isolated fixture sends one
explicit operator command on the existing player connection. The client still
receives the server's actual slot update and all real clicks travel unchanged.
"""
from __future__ import annotations

import json
import select
import socket
import threading
import time
from pathlib import Path

from .light_wire import LightWireProxy, Reader, WireLight


def varint(value: int) -> bytes:
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


class CorrectionWire(LightWireProxy):
    def __init__(self, server_port: int, artifact: Path, report: Path) -> None:
        packets = json.loads(report.read_text())['play']['serverbound']
        self.click_id = packets['minecraft:container_click']['protocol_id']
        self.close_id = packets['minecraft:container_close']['protocol_id']
        self.command_id = packets['minecraft:chat_command']['protocol_id']
        self.command: str | None = None
        self.pending = False
        self.sent = threading.Event()
        self.close_events: list[tuple[str, int, int | None]] | None = None
        self.close_seen = threading.Event()
        self.close_packet_seen = threading.Event()
        self.receipt: list[dict[str, object]] = []
        self.receipt_path = artifact / 'shift-correction-wire.json'
        super().__init__(server_port, artifact, client_packet_observer=self._on_click, decode_light=False)

    def arm(self, command: str) -> None:
        with self.lock:
            if self.command is not None or self.pending:
                raise RuntimeError('correction fixture is already armed')
            self.command = command
            self.sent.clear()
            self.receipt.append({'event': 'armed', 'command': command, 'monotonic_ns': time.monotonic_ns()})

    def arm_close(self) -> None:
        with self.lock:
            self.close_events = []
            self.close_seen.clear()
            self.close_packet_seen.clear()

    def _on_click(self, wire: WireLight, phase: str, packet: int, payload: bytes) -> None:
        if phase == 'play' and self.close_events is not None and packet in (self.click_id, self.close_id):
            reader = Reader(payload)
            container = reader.varint()
            slot = None
            if packet == self.click_id:
                reader.varint()
                slot = int.from_bytes(reader.take(2), 'big', signed=True)
            event = ('click' if packet == self.click_id else 'close', container, slot)
            self.close_events.append(event)
            self.receipt.append({'event': 'menu_transition', 'packet': event,
                                 'monotonic_ns': time.monotonic_ns()})
            if packet == self.close_id:
                self.close_packet_seen.set()
            if len(self.close_events) == 3:
                self.close_seen.set()
                self.close_events = None
            return
        if self.command is None or phase != 'play' or packet != self.click_id:
            return
        reader = Reader(payload)
        container, state = reader.varint(), reader.varint()
        slot = int.from_bytes(reader.take(2), 'big', signed=True)
        if container == 0 or slot != 0:
            raise RuntimeError(f'correction fixture expected chest slot 0, got {container}:{slot}')
        self.pending = True
        self.receipt.append({'event': 'physical_click_forwarded_next', 'container': container,
                             'state': state, 'slot': slot, 'monotonic_ns': time.monotonic_ns()})

    def _forward(self, client: socket.socket, wire: WireLight) -> None:
        try:
            with client, socket.create_connection(('127.0.0.1', self.server_port), timeout=10) as server:
                server_buffer, client_buffer = bytearray(), bytearray()
                phase, compressed = 'login', False
                while True:
                    ready = select.select([client, server, self.wake], [], [])[0]
                    if self.wake in ready:
                        return
                    for source, target in ((client, server), (server, client)):
                        if source not in ready:
                            continue
                        data = source.recv(65536)
                        if not data:
                            return
                        if source is client:
                            client_buffer.extend(data)
                            self._frames(client_buffer, wire, phase, compressed, serverbound=True)
                            target.sendall(data)
                            with self.lock:
                                if self.pending:
                                    command = self.command.encode()
                                    body = varint(self.command_id) + varint(len(command)) + command
                                    if compressed:
                                        body = b'\0' + body
                                    target.sendall(varint(len(body)) + body)
                                    self.receipt.append({'event': 'fixture_command_sent',
                                                         'monotonic_ns': time.monotonic_ns()})
                                    self.pending = False
                                    self.command = None
                                    self.sent.set()
                            continue
                        server_buffer.extend(data)
                        phase, compressed = self._frames(server_buffer, wire, phase, compressed)
                        target.sendall(data)
        except ConnectionError:
            return
        except Exception as error:
            self.errors.append(f'{type(error).__name__}: {error}')
            self.sent.set()
            self.close_packet_seen.set()
            self.close_seen.set()

    def wait_sent(self, timeout: float) -> None:
        if not self.sent.wait(timeout):
            raise TimeoutError('correction fixture was not triggered by a physical chest click')
        if self.errors:
            raise RuntimeError(f'correction wire failed: {self.errors}')
        if not any(event['event'] == 'fixture_command_sent' for event in self.receipt):
            raise RuntimeError('correction fixture was not sent')

    def wait_closed(self, timeout: float) -> None:
        if not self.close_packet_seen.wait(timeout):
            raise TimeoutError('did not observe the physical menu close on the wire')
        if self.errors:
            raise RuntimeError(f'correction wire failed: {self.errors}')

    def wait_close(self, timeout: float) -> None:
        if not self.close_seen.wait(timeout):
            raise TimeoutError('did not observe click, close and click on the wire')
        if self.errors:
            raise RuntimeError(f'correction wire failed: {self.errors}')
        events = [entry['packet'] for entry in self.receipt if entry['event'] == 'menu_transition']
        if events != [('click', 0, 9), ('close', 0, None), ('click', 0, 9)]:
            raise RuntimeError(f'unexpected close transition: {events}')

    def close(self) -> None:
        super().close()
        self.receipt_path.write_text(json.dumps({'scope': 'one declared serverbound fixture command',
                                                 'events': self.receipt, 'errors': self.errors}, indent=2))
