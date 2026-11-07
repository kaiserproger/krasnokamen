"""Concurrent inputs require exact consumed-input log evidence."""
import json
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest

from .matrix_api import MissingCapability
from .protocol_concurrency import concurrent_clicks


class ConcurrentAdmissionTests(unittest.TestCase):
    def test_conflicting_clicks_are_bound_to_recorded_ticks(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp) / 'input-log'
            directory.mkdir()
            log = directory / '1-1.jsonl'
            lock = threading.Lock()
            def append(tick, inputs):
                with lock, log.open('a') as output:
                    output.write(json.dumps({'Tick': {'tick': tick, 'inputs': inputs}}) + '\n')
            append(1, [{'Input': {'Join': {'name': 'A', 'connection': 1}}},
                       {'Input': {'Join': {'name': 'B', 'connection': 2}}}])
            def client(name, connection):
                def click_slot(slot, kind='pickup'):
                    append(10 + connection, [{'Input': {'ContainerClick': {
                        'connection': connection, 'window': 1, 'state_id': 2,
                        'slot': slot, 'button': 0, 'mode': 'QuickMove'}}}])
                return SimpleNamespace(name=name, snapshot=lambda: {'menu': {'id': 1, 'state_id': 2}},
                                       click_slot=click_slot)
            clients = [client('A', 1), client('B', 2)]
            server = SimpleNamespace(world_dir=Path(tmp), command=lambda value: append(
                2, [{'Input': {'ConsoleCommand': value}}]))
            ctx = SimpleNamespace(server=server, state={})
            receipts = concurrent_clicks(ctx, [(c, 'click_slot', {'slot': 0, 'kind': 'quick_move'})
                                               for c in clients])
            self.assertEqual([item['tick'] for item in receipts], [11, 12])
            self.assertEqual([item['input']['connection'] for item in receipts], [1, 2])

    def test_gap_never_proves_input_admission(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp) / 'input-log'
            directory.mkdir()
            (directory / '1-1.jsonl').write_text('{"Gap":{"from":1,"to":2}}\n')
            client = SimpleNamespace(name='A')
            ctx = SimpleNamespace(server=SimpleNamespace(world_dir=Path(tmp), command=lambda _: None), state={})
            with self.assertRaisesRegex(MissingCapability, 'input_log:gap'):
                concurrent_clicks(ctx, [(client, 'click_slot', {'slot': 0, 'kind': 'quick_move'})])
