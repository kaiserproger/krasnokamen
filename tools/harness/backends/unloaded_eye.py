"""Causal unloaded-eye scene, ported from client 8a7817c8.

Forward unmodified real-server packets in order; hold traffic immediately after
its far teleport until the real client captures the not-yet-loaded destination.
"""
import json
from pathlib import Path
import select
import socket
import struct
import threading

from .. import runtime
from .light_wire import LightWireProxy, Reader

DESTINATION = [1000.5, 176.0, 1000.5]


class UnloadedEyeProxy(LightWireProxy):
    def __init__(self, port, artifact):
        self.held = threading.Event()
        self.release = threading.Event()
        packets = json.loads((runtime.REPO_ROOT / "data/vanilla/26.3/reports/packets.json").read_text())
        self.position_id = packets["play"]["clientbound"]["minecraft:player_position"]["protocol_id"]
        super().__init__(port, artifact, self.observe_packet)

    def observe_packet(self, wire, phase, packet, payload):
        if phase == "play" and packet == self.position_id:
            reader = Reader(payload)
            teleport_id = reader.varint()
            coordinates = list(struct.unpack(">ddd", reader.take(24)))
            wire.events.append({"kind": "player_position", "teleport_id": teleport_id,
                                "position": coordinates})

    def start(self):
        return int(self.address.rsplit(':', 1)[1])

    def _forward(self, client, wire):
        try:
            with client, socket.create_connection(('127.0.0.1', self.server_port), timeout=10) as server:
                buffer = bytearray()
                phase, compressed = 'login', False
                while True:
                    ready = select.select([client, server, self.wake], [], [])[0]
                    if self.wake in ready:
                        return
                    if client in ready:
                        data = client.recv(65536)
                        if not data:
                            return
                        server.sendall(data)
                    if server not in ready:
                        continue
                    data = server.recv(65536)
                    if not data:
                        return
                    buffer.extend(data)
                    while buffer:
                        header = Reader(bytes(buffer[:5]))
                        try:
                            length = header.varint()
                        except ValueError:
                            break
                        if length > 16 * 1024 * 1024:
                            raise ValueError('oversized local server frame')
                        size = header.offset + length
                        if len(buffer) < size:
                            break
                        frame = bytearray(buffer[:size])
                        del buffer[:size]
                        phase, compressed = self._frames(frame.copy(), wire, phase, compressed)
                        client.sendall(frame)
                        if (not self.held.is_set() and wire.events
                                and wire.events[-1].get('kind') == 'player_position'
                                and wire.events[-1].get('position') == DESTINATION):
                            self.held.set()
                            if not self.release.wait(timeout=30):
                                raise TimeoutError('client never released unloaded teleport gate')
        except ConnectionError:
            return
        except Exception as error:
            self.errors.append(f'{type(error).__name__}: {error}')

    def close(self):
        self.release.set()
        super().close()


def execute(probe, artifact, steps, client):
    from . import scene_shot

    proxy = probe.client.scene_proxy
    first_tp = next(index for index, step in enumerate(steps) if 'tp' in step)
    shots = scene_shot.execute(probe, artifact, steps[:first_tp], client)
    coordinates = steps[first_tp]['tp']
    probe.call('minecraft_send_chat', {'message': 'tp @s ' + ' '.join(map(str, coordinates)), 'command': True})
    try:
        if not proxy.held.wait(timeout=min(30, probe.remaining())):
            raise TimeoutError('server never sent the far teleport')
        state = probe.wait(lambda state: state.get('in_play') is not True
                           or (state.get('player') is not None
                               and abs(state['player']['x'] - DESTINATION[0]) < 0.01),
                           'teleport_received_without_terrain', outside_play=True)
        probe.check('unloaded_eye_session_survives', state.get('in_play') is True)
        probe.check('unloaded_eye_unknown', state['player'].get('eye_medium') is None)
        (artifact / 'unloaded-destination.json').write_text(json.dumps(state, indent=2) + '\n')
        shots += scene_shot.execute(probe, artifact, [{'shot': 'unloaded-destination'}], client)
    finally:
        proxy.release.set()
    shots += scene_shot.execute(probe, artifact, steps[first_tp + 1:], client)
    for name in ('water-arrived', 'still-playing'):
        state = json.loads((artifact / f'{name}.json').read_text())
        probe.check(f'{name}_play', state.get('in_play') is True)
        probe.check(f'{name}_water', state.get('player', {}).get('eye_medium') == 'water')
    frames = [json.loads(Path(shot).with_suffix('.json').read_text()) for shot in shots]
    probe.check('actual_window_gpu_complete', all(
        frame.get('gpu_completion') is True
        and frame.get('render_target') == 'actual_window_scene_target' for frame in frames))
    ids = [int(frame['frame_id']) for frame in frames]
    probe.check('frames_continue_after_terrain', len(ids) == 4 and ids == sorted(set(ids)))
    events = proxy.query(lambda wire: list(wire.events))
    teleport = next(index for index, row in enumerate(events)
                    if row.get('kind') == 'player_position' and row.get('position') == DESTINATION)
    chunk = next(index for index, row in enumerate(events)
                 if row.get('kind') == 'chunk' and row.get('chunk') == [62, 62])
    probe.check('teleport_precedes_destination_chunk', teleport < chunk)
    return shots
