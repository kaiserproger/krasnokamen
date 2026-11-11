"""Concurrent inputs require exact consumed-input log evidence."""
import json
import struct
import zlib
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest

from .matrix_api import MissingCapability
from .protocol_concurrency import (concurrent_clicks, match_click_and_break, submit_click_and_break,
                                   verify_destroy_tick_fence, committed_destroy_records)
from .matrix_domains.death_respawn import destroy_census


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

    def test_destroy_tick_fence_fails_closed(self):
        click = {'connection': 7, 'window': 2, 'state_id': 3, 'slot': 0,
                 'button': 0, 'mode': 'Pickup'}
        start = {'connection': 7, 'position': {'x': 1, 'y': 2, 'z': 3}, 'action': 'Start'}
        match = lambda inputs: match_click_and_break(
            [{'tick': 25, 'inputs': inputs}], connection=7, window=2, state_id=3, slot=0, position=(1, 2, 3))
        found = match([{'Input': {'ContainerClick': click}}, {'Input': {'Dig': start}}])
        self.assertEqual(verify_destroy_tick_fence(found, 25, 25), 25)
        # A deferred break commits its removal after the start tick; an unknown
        # (pre-tick) record cannot prove anything.
        for removal, container in ((26, 25), (25, 26), (None, 25), (25, None)):
            with self.subTest(removal=removal, container=container):
                with self.assertRaises(MissingCapability):
                    verify_destroy_tick_fence(found, removal, container)
        split = dict(found, start=(26, found['start'][1], found['start'][2]))
        with self.assertRaisesRegex(MissingCapability, 'different_ticks'):
            verify_destroy_tick_fence(split, 25, 25)
        with self.assertRaisesRegex(MissingCapability, 'missing_destroy_inputs'):
            match([{'Input': {'ContainerClick': click}}])
        with self.assertRaisesRegex(MissingCapability, 'ambiguous_destroy_inputs'):
            match([{'Input': {'ContainerClick': click}}, {'Input': {'Dig': start}}, {'Input': {'Dig': start}}])
        with self.assertRaisesRegex(MissingCapability, 'destroy_input_order'):
            match([{'Input': {'Dig': start}}, {'Input': {'ContainerClick': click}}])

    def test_destroy_census_fixture_negative_control(self):
        ids = {'minecraft:diamond', 'minecraft:iron_ingot'}
        inventory = [{'item_id': 'minecraft:iron_ingot', 'count': 2, 'components': {}}]
        container = [{'item_id': 'minecraft:diamond', 'count': 5, 'components': {}}]
        baseline = destroy_census(inventory + container, ids)
        ground = [{'item_id': 'minecraft:diamond', 'count': 4, 'components': {}}]
        after = inventory + [{'item_id': 'minecraft:diamond', 'count': 1, 'components': {}}] + ground
        self.assertEqual(destroy_census(after, ids), baseline)
        self.assertNotEqual(destroy_census(after + [ground[0]], ids), baseline)
        self.assertNotEqual(destroy_census(after[:-1], ids), baseline)
        with self.assertRaisesRegex(MissingCapability, 'invalid_census_stack'):
            destroy_census([{'item_id': 'minecraft:diamond'}], ids)
        damaged = lambda n: {'item_id': 'minecraft:netherite_axe', 'count': 1,
                             'components': {'minecraft:damage': n}}
        self.assertEqual(destroy_census([damaged(0)], {'minecraft:netherite_axe'}),
                         destroy_census([damaged(1)], {'minecraft:netherite_axe'}))
        changed = [{'item_id': 'minecraft:diamond', 'count': 5,
                    'components': {'minecraft:custom_name': 'different'}}]
        self.assertNotEqual(destroy_census(changed, ids), destroy_census(container, ids))

    def test_destroy_wal_requires_both_records(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp) / 'journal'
            directory.mkdir()
            block = {'actor': 'A', 'pos': [1, 2, 3], 'tick': 25, 'change': {'Block': {}}}
            container = {'actor': 'A', 'pos': [1, 2, 3], 'tick': 25, 'change': {'Container': {}}}
            ctx = SimpleNamespace(server=SimpleNamespace(world_dir=Path(tmp)))
            def write(records):
                payload = (json.dumps({'BlockLog': {'value': {'records': records}}}) + '\n').encode()
                header = struct.pack('<IIQQII', 0x534F4C4A, 1, 25, 25, len(payload), zlib.crc32(payload))
                (directory / 'batch-25-25.bin').write_bytes(header + payload)
            write([block, container])
            self.assertEqual(committed_destroy_records(ctx, 25, 'A', (1, 2, 3)),
                             {'block': block, 'container': container})
            write([block])
            with self.assertRaisesRegex(MissingCapability, 'missing_or_ambiguous'):
                committed_destroy_records(ctx, 25, 'A', (1, 2, 3))

    def test_single_actor_batch_matches_click_and_dig(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp) / 'input-log'
            directory.mkdir()
            log = directory / '1-1.jsonl'
            def append(tick, inputs):
                with log.open('a') as stream:
                    stream.write(json.dumps({'Tick': {'tick': tick, 'inputs': inputs}}) + '\n')
            append(1, [{'Input': {'Join': {'name': 'A', 'connection': 7}}}])
            receipt = {'kind': 'click_slot', 'container_id': 2, 'state_id': 3}
            def submit(actions):
                self.assertEqual([action['kind'] for action in actions], ['click_slot', 'break_block'])
                append(25, [{'Input': {'ContainerClick': {'connection': 7, 'window': 2,
                    'state_id': 3, 'slot': 0, 'button': 0, 'mode': 'Pickup'}}},
                    {'Input': {'Dig': {'connection': 7, 'position': {'x': 1, 'y': 2, 'z': 3},
                                       'action': 'Start'}}}])
                return {'sent': [receipt, {'kind': 'break_block'}]}
            client = SimpleNamespace(name='A', submit_input_batch=submit)
            server = SimpleNamespace(world_dir=Path(tmp), command=lambda value: append(
                2, [{'Input': {'ConsoleCommand': value}}]))
            result = submit_click_and_break(SimpleNamespace(server=server, state={}), client, 0, (1, 2, 3))
            self.assertEqual([result['inputs'][part][0] for part in ('click', 'start')], [25, 25])
            # Without the ids the click packet carried, the click cannot be identified.
            del receipt['state_id']
            with self.assertRaisesRegex(MissingCapability, 'client_batch:click_receipt'):
                submit_click_and_break(SimpleNamespace(server=server, state={}), client, 0, (1, 2, 3))
