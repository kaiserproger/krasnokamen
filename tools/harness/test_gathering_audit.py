"""Small CRC-framed fixture logs exercise D54 causal (not residual) credit."""
import copy
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import zlib

from tools.harness import gathering_audit as audit
from tools.harness.growth_accounting import reconcile


NAMES = {1: 'minecraft:cobblestone', 2: 'minecraft:acacia_log',
         3: 'minecraft:wheat', 4: 'minecraft:wheat_seeds', 5: 'minecraft:egg'}
STATES = {0: ('minecraft:air', {}), 1: ('minecraft:stone', {}),
          2: ('minecraft:acacia_log', {}), 3: ('minecraft:wheat', {'age': '7'}),
          4: ('minecraft:wheat', {'age': '0'})}


def actor(identity, profession, position, inventory=()):
    return {'id': identity, 'uuid': f'actor-{identity}', 'pos': position, 'tick': 0,
            'villager': {'data': {'profession': profession}, 'inventory': list(inventory),
                         'mining_progress': {'building': 1, 'revision': 1,
                             'bounds': {'min': [0, 0, 0], 'max': [20, 20, 20]},
                             'lane': 0, 'level': 0, 'pass': 0, 'cell': 0, 'layer': 0},
                         'lumber_progress': {'building': 2, 'has_target': True,
                             'base': [5, 2, 5], 'height': 4, 'layer': 0}}}


def stack(item, count):
    return {'item_id': item, 'count': count}


def write_batch(root, tick, entries):
    payload = b''.join(json.dumps(row).encode() + b'\n' for row in entries)
    frame = struct.pack('<IIQQII', 0x534F4C4A, 1, tick, tick, len(payload), zlib.crc32(payload))
    (root / f'batch-{tick:020d}-{tick:020d}.bin').write_bytes(frame + payload)


class GatheringAuditTests(unittest.TestCase):
    def fixture(self, root):
        miner = actor(1, 'Miner', [1.5, 2, 1.5])
        logger = actor(2, 'Lumberjack', [5.5, 2, 5.5])
        farmer = actor(3, 'Farmer', [8.5, 2, 8.5])
        before = {'checkpoint_tick': 0, 'banks': [{'position': [2, 2, 2], 'slots': []}],
                  'globals': {'checkpoint': {
                      'climate': {'game_clock': {'running': {'held_ticks': 0}}},
                      'debt': [{'chunk': [0, 0], 'remaining': 0, 'active': True}]}},
                  'dimensions': {audit.DIMENSION: {'entities': {'1': miner, '2': logger, '3': farmer}}}}
        mined = copy.deepcopy(miner)
        mined['tick'] = 1
        mined['villager']['inventory'] = [stack(1, 4)]
        mined['villager']['mining_progress']['layer'] = 1
        logged = copy.deepcopy(logger)
        logged['tick'] = 1
        logged['villager']['inventory'] = [stack(2, 4)]
        logged['villager']['lumber_progress']['layer'] = 1
        farmed = copy.deepcopy(farmer)
        farmed['tick'] = 1
        farmed['villager']['inventory'] = [stack(3, 4), stack(4, 4)]
        write_batch(root, 1, [{'Block': {'pos': [1, 3, 1], 'state': 0}},
                              {'Block': {'pos': [5, 2, 5], 'state': 0}},
                              {'Block': {'pos': [8, 2, 8], 'state': 4}},
                              *({'Entity': e} for e in (mined, logged, farmed))])
        deposited = copy.deepcopy(mined)
        deposited['tick'] = 2
        deposited['villager']['inventory'] = []
        write_batch(root, 2, [{'BlockEntity': {'pos': [2, 2, 2], 'slots': [stack(1, 4)]}},
                              {'Entity': deposited}])
        preimages = {(1, 3, 1): 1, (5, 2, 5): 2, (8, 2, 8): 3}
        return before, preimages

    def construction(self, missing=0):
        return {'before': {'total': {}},
                'after': {'total': {name: 4 - (missing if name == 'minecraft:cobblestone' else 0)
                                    for name in list(NAMES.values())[:4]}},
                'journaled': {'cut': {}, 'crafting': {}, 'consumed': {}},
                'structure_records': {}, 'construction_kinds_on_ground': {}}

    def observed(self, root):
        before, preimages = self.fixture(root)
        return audit.observe(before, root, 2, NAMES, STATES, preimages, 4, {})

    def test_fixture_log_closes_all_items_with_actor_block_and_storage_receipts(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.observed(Path(directory))
        self.assertEqual(audit.full_balance(self.construction(), result)['status'], 'PASS')
        self.assertEqual(len(result['gathering']), 3)
        self.assertEqual(result['gathering'][0]['position'], [1, 3, 1])
        self.assertEqual(result['deposits'], [{'wal': 'batch-00000000000000000002-00000000000000000002.bin',
                          'tick': 2, 'actor': 'actor-1', 'storage': [2, 2, 2],
                          'items': {'minecraft:cobblestone': 4}}])

    def test_without_attribution_balance_does_not_close(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(audit, 'gathering_event', return_value=None):
            result = self.observed(Path(directory))
        balance = audit.full_balance(self.construction(), result)
        self.assertEqual(balance['status'], 'FAIL')
        self.assertEqual(balance['unexplained_delta'], self.construction()['after']['total'])

    def test_missing_item_remains_red_with_gathering_enabled(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.observed(Path(directory))
        balance = audit.full_balance(self.construction(missing=1), result)
        self.assertEqual(balance['status'], 'FAIL')
        self.assertEqual(balance['unexplained_delta'], {'minecraft:cobblestone': -1})

    def test_no_credit_without_block_cursor_or_configured_yield(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before, preimages = self.fixture(root)
            entries = next(audit.batches(root, 0, 1))[3]
            new = next(value for kind, value in entries if kind == 'Entity')
            old = before['dimensions'][audit.DIMENSION]['entities']['1']
            changes = [v for k, v in entries if k == 'Block']
            self.assertIsNone(audit.gathering_event(old, new, [], preimages, STATES, NAMES, 4))
            self.assertIsNone(audit.gathering_event(old, new, changes, preimages, STATES, NAMES, 1))
            old['villager']['mining_progress'] = copy.deepcopy(new['villager']['mining_progress'])
            self.assertIsNone(audit.gathering_event(old, new, changes, preimages, STATES, NAMES, 4))

    def test_unrelated_item_loss_is_failure_not_an_exemption(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.observed(Path(directory))
        construction = self.construction()
        construction['before']['total']['minecraft:egg'] = 1
        self.assertEqual(audit.full_balance(construction, result)['unexplained_delta'], {'minecraft:egg': -1})

    def test_archive_gap_and_crc_failure_are_not_silently_skipped(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            file = next(root.glob('batch-*-00000000000000000001.bin'))
            original = file.read_bytes()
            file.write_bytes(original[:-1] + b'x')
            with self.assertRaisesRegex(ValueError, 'invalid WAL'):
                list(audit.batches(root, 0, 2))
            file.unlink()
            with self.assertRaisesRegex(ValueError, 'noncontiguous'):
                list(audit.batches(root, 0, 2))

    def test_natural_ground_expiry_requires_lifetime_not_disappearance(self):
        for age, expected in ((5999, {'minecraft:egg': 1}), (10, {})):
            with self.subTest(age=age), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                before, preimages = self.fixture(root)
                drop = {'id': 4, 'uuid': 'drop', 'item': stack(5, 1), 'age': age, 'pos': [1, 2, 1]}
                before['dimensions'][audit.DIMENSION]['entities']['4'] = drop
                write_batch(root, 1, [{'RemoveEntity': {'id': 4, 'tick': 1}}])
                write_batch(root, 2, [])
                result = audit.observe(before, root, 2, NAMES, STATES, preimages, 4, {})
                self.assertEqual(result['other_consumed'], expected)

    def test_sparse_state_age_is_an_expiry_preimage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before, preimages = self.fixture(root)
            before['dimensions'][audit.DIMENSION]['entities']['4'] = {
                'id': 4, 'uuid': 'drop', 'item': stack(5, 1), 'age': 0, 'pos': [1, 2, 1]}
            write_batch(root, 1, [{'EntityState': {'id': 4, 'state': {'age': 5999}}}])
            write_batch(root, 2, [{'RemoveEntity': {'id': 4, 'tick': 2}}])
            result = audit.observe(before, root, 2, NAMES, STATES, preimages, 4, {})
            self.assertEqual(result['other_consumed'], {'minecraft:egg': 1})
            self.assertEqual(result['ground_expiry'][0]['observed_age'], 5999)

    def test_catchup_spend_but_not_cancellation_proves_expiry(self):
        for tail, expected in ((800, {'minecraft:egg': 1}), (0, {})):
            with self.subTest(tail=tail), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                before, preimages = self.fixture(root)
                before['globals']['checkpoint']['debt'][0]['remaining'] = 1000
                before['dimensions'][audit.DIMENSION]['entities']['4'] = {
                    'id': 4, 'uuid': 'drop', 'item': stack(5, 1), 'age': 5900, 'pos': [1, 2, 1]}
                write_batch(root, 1, [
                    {'SimDebt': {'image': {'chunk': [0, 0], 'remaining': tail, 'active': True}}},
                    {'RemoveEntity': {'id': 4, 'tick': 1}}])
                write_batch(root, 2, [])
                result = audit.observe(before, root, 2, NAMES, STATES, preimages, 4, {})
                self.assertEqual(result['other_consumed'], expected)

    def test_egg_birth_requires_unique_source_timer_reset(self):
        for reset, expected in ((True, {'minecraft:egg': 1}), (False, {})):
            with self.subTest(reset=reset), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                before, preimages = self.fixture(root)
                chicken = {'id': 4, 'uuid': 'chicken', 'type_id': 10, 'alive': True,
                           'pos': [2, 2, 2], 'state': {'age': 100, 'entity_life': {'harvest_ticks': 1}}}
                before['dimensions'][audit.DIMENSION]['entities']['4'] = chicken
                write_batch(root, 1, [
                    {'EntityState': {'id': 4, 'state': {
                        'age': 101, 'entity_life': {'harvest_ticks': 7000 if reset else 0}}}},
                    {'Entity': {'id': 5, 'uuid': 'egg', 'type_id': 11, 'tick': 1,
                                'pos': [2, 2, 2], 'age': 0, 'item': stack(5, 1)}}])
                write_batch(root, 2, [])
                result = audit.observe(before, root, 2, NAMES, STATES, preimages, 4, {10: 'minecraft:chicken'})
                self.assertEqual(result['other_produced'], expected)

    def test_death_loot_requires_new_handled_death_and_bounded_quantity(self):
        names = {**NAMES, 6: 'minecraft:rotten_flesh'}
        for count, handled, expected in ((2, False, {'minecraft:rotten_flesh': 2}),
                                          (3, False, {}), (2, True, {})):
            with self.subTest(count=count, handled=handled), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                before, preimages = self.fixture(root)
                zombie = {'id': 4, 'uuid': 'zombie', 'type_id': 10, 'alive': True,
                          'pos': [2, 2, 2], 'age': 10,
                          'state': {'age': 10, 'loot': {'death_handled': handled}}}
                before['dimensions'][audit.DIMENSION]['entities']['4'] = zombie
                corpse = copy.deepcopy(zombie)
                corpse['alive'] = False
                corpse['state']['loot']['death_handled'] = True
                write_batch(root, 1, [
                    {'Entity': corpse},
                    {'Entity': {'id': 5, 'uuid': 'loot', 'type_id': 11, 'tick': 1,
                                'pos': [2, 2, 2], 'age': 0, 'item': stack(6, count)}}])
                write_batch(root, 2, [])
                result = audit.observe(before, root, 2, names, STATES, preimages, 4, {10: 'minecraft:zombie'})
                self.assertEqual(result['other_produced'], expected)

    def test_leaf_decay_does_not_credit_two_drops_from_one_block(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before, preimages = self.fixture(root)
            names = {**NAMES, 6: 'minecraft:birch_sapling'}
            states = {**STATES, 5: ('minecraft:birch_leaves', {})}
            preimages[1, 2, 3] = 5
            write_batch(root, 1, [
                {'Block': {'pos': [1, 2, 3], 'state': 0}},
                *({'Entity': {'id': identity, 'uuid': f'drop-{identity}',
                              'tick': 1, 'age': 0, 'item': stack(6, 1), 'pos': [1.5, 2.5, 3.5]}}
                  for identity in (5, 6))])
            write_batch(root, 2, [])
            result = audit.observe(before, root, 2, names, states, preimages, 4, {})
            self.assertEqual(result['other_produced'], {'minecraft:birch_sapling': 1})

    def test_held_clock_step_but_not_held_wall_tick_ages_ground_items(self):
        for step, expected in ((0, {}), (1, {'minecraft:egg': 1})):
            with self.subTest(step=step), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                before, preimages = self.fixture(root)
                before['globals']['checkpoint']['climate']['game_clock'] = {'held': {'game_ticks': 0}}
                before['dimensions'][audit.DIMENSION]['entities']['4'] = {
                    'id': 4, 'uuid': 'drop', 'item': stack(5, 1), 'age': 5999, 'pos': [1, 2, 1]}
                write_batch(root, 1, [
                    {'Climate': {'value': {'game_clock': {'held': {'game_ticks': step}}}}},
                    {'RemoveEntity': {'id': 4, 'tick': 1}}])
                write_batch(root, 2, [])
                result = audit.observe(before, root, 2, NAMES, STATES, preimages, 4, {})
                self.assertEqual(result['other_consumed'], expected)

    def test_sparse_chicken_countdown_reset_is_position_scoped(self):
        for initial, age, following, expected in (
                (9000, 9100, 7000, 2), (9000, 101, 8999, 0), (0, 101, 7000, 0)):
            with self.subTest(initial=initial, following=following), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                before, preimages = self.fixture(root)
                entries = []
                for identity in (4, 5):
                    position = [identity, 2, 2]
                    before['dimensions'][audit.DIMENSION]['entities'][str(identity)] = {
                        'id': identity, 'uuid': f'chicken-{identity}', 'type_id': 10, 'alive': True,
                        'pos': position, 'state': {'age': 100, 'entity_life': {'harvest_ticks': initial}}}
                    entries.extend([
                        {'EntityState': {'id': identity, 'state': {
                            'age': age, 'entity_life': {'harvest_ticks': following}}}},
                        {'Entity': {'id': identity + 2, 'uuid': f'egg-{identity}', 'tick': 1,
                                    'pos': position, 'age': 0, 'item': stack(5, 1)}}])
                write_batch(root, 1, entries)
                write_batch(root, 2, [])
                result = audit.observe(before, root, 2, NAMES, STATES, preimages, 4, {10: 'minecraft:chicken'})
                self.assertEqual(result['other_produced'], {'minecraft:egg': expected} if expected else {})

    def test_scute_birth_requires_one_counter_increment_and_one_item(self):
        names = {**NAMES, 6: 'minecraft:armadillo_scute'}
        for drops, count, expected in ((3, 1, {'minecraft:armadillo_scute': 1}), (2, 1, {}), (3, 2, {})):
            with self.subTest(drops=drops, count=count), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                before, preimages = self.fixture(root)
                before['dimensions'][audit.DIMENSION]['entities']['4'] = {
                    'id': 4, 'uuid': 'armadillo', 'type_id': 10, 'alive': True,
                    'pos': [2, 2, 2], 'state': {'age': 100, 'entity_life': {'armadillo': {'drops': 2}}}}
                write_batch(root, 1, [
                    {'EntityState': {'id': 4, 'state': {
                        'age': 101, 'entity_life': {'armadillo': {'drops': drops}}}}},
                    {'Entity': {'id': 5, 'uuid': 'scute', 'tick': 1, 'pos': [2, 2, 2],
                                'age': 0, 'item': stack(6, count)}}])
                write_batch(root, 2, [])
                result = audit.observe(before, root, 2, names, STATES, preimages, 4, {10: 'minecraft:armadillo'})
                self.assertEqual(result['other_produced'], expected)

    def test_leaf_decay_has_separate_bounded_stick_and_sapling_budgets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before, preimages = self.fixture(root)
            names = {**NAMES, 6: 'minecraft:birch_sapling', 7: 'minecraft:stick'}
            states = {**STATES, 5: ('minecraft:birch_leaves', {})}
            preimages[1, 2, 3] = 5
            write_batch(root, 1, [
                {'Block': {'pos': [1, 2, 3], 'state': 0}},
                *({'Entity': {'id': identity, 'uuid': f'drop-{identity}', 'tick': 1,
                              'age': 0, 'item': stack(item, count), 'pos': [1.5, 2.5, 3.5]}}
                  for identity, item, count in ((5, 7, 2), (6, 6, 1), (7, 7, 1)))])
            write_batch(root, 2, [])
            result = audit.observe(before, root, 2, names, states, preimages, 4, {})
            self.assertEqual(result['other_produced'], {'minecraft:stick': 2, 'minecraft:birch_sapling': 1})

    def test_additional_death_sources_keep_species_quantity_and_position_bounds(self):
        cases = (('drowned', 'rotten_flesh', 2), ('zombie_villager', 'rotten_flesh', 2),
                 ('spider', 'string', 2), ('cat', 'string', 2), ('pig', 'porkchop', 3),
                 ('enderman', 'ender_pearl', 1), ('iron_golem', 'iron_ingot', 5),
                 ('iron_golem', 'poppy', 2))
        for species, item, limit in cases:
            names = {**NAMES, 6: 'minecraft:' + item}
            for count, position in ((limit, [2, 2, 2]), (limit + 1, [2, 2, 2]), (limit, [3, 2, 2])):
                with self.subTest(species=species, item=item, count=count, position=position), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    before, preimages = self.fixture(root)
                    source = {'id': 4, 'uuid': species, 'type_id': 10, 'alive': True,
                              'pos': [2, 2, 2], 'state': {'age': 10, 'loot': {'death_handled': False}}}
                    before['dimensions'][audit.DIMENSION]['entities']['4'] = source
                    corpse = copy.deepcopy(source)
                    corpse['alive'] = False
                    corpse['state']['loot']['death_handled'] = True
                    write_batch(root, 1, [
                        {'Entity': corpse},
                        {'Entity': {'id': 5, 'uuid': 'loot', 'tick': 1, 'pos': position,
                                    'age': 0, 'item': stack(6, count)}}])
                    write_batch(root, 2, [])
                    result = audit.observe(before, root, 2, names, STATES, preimages, 4, {10: 'minecraft:' + species})
                    self.assertEqual(result['other_produced'],
                                     {names[6]: limit} if count == limit and position == [2, 2, 2] else {})

    def test_warehouse_uses_structure_identity_across_preparation_origin_offset(self):
        building = {'min': [109, 66, 106], 'max': [113, 71, 114]}
        projects = {'storage-1': {'structure': 'storage-1', 'blueprint': 'storage',
                                 'origin': building['min'], 'builder': ['builder', None]}}
        records = {'structure/storage-1': {
            'snapshot': {'origin': [109, 64, 106], 'state': 'committed', 'consumed': []},
            'cut_credits': {'1': 1}}}
        after = {'banks': [{'position': [110, 67, 111], 'slots': [stack(2, 4)]}]}
        proof = audit.warehouse_proof(building, projects, records, after, {'carries': {}}, {1: 'minecraft:dirt'})
        self.assertEqual(proof['status'], 'PASS')
        self.assertEqual(proof['structure_key'], 'structure/storage-1')
        carried = audit.warehouse_proof(building, projects, records, after,
                                       {'carries': {'builder': {'minecraft:dirt': 1}}}, {1: 'minecraft:dirt'})
        self.assertEqual(carried['status'], 'FAIL')
        missing = audit.warehouse_proof(building, {}, records, after, {'carries': {}}, {1: 'minecraft:dirt'})
        self.assertEqual(missing['status'], 'FAIL')

    def finalizer_fixture(self, root, entries):
        archive = root / 'journal-witness'
        archive.mkdir()
        write_batch(archive, 1, entries)
        before = {'checkpoint_tick': 0, 'banks': [], 'structure_postimages': {},
                  'globals': {'checkpoint': {'climate': {
                      'game_clock': {'running': {'held_ticks': 0}}}}},
                  'dimensions': {audit.DIMENSION: {'tick': 0, 'entities': {}, 'players': {}}}}
        after = copy.deepcopy(before)
        after['checkpoint_tick'] = after['dimensions'][audit.DIMENSION]['tick'] = 1
        config = root / 'server.toml'
        config.write_text('[test_acceleration]\ngathering_yield = 4.0\n')
        (root / 'run.json').write_text(json.dumps({'status': 'failed', 'snapshots': []}))
        return before, after, config

    def test_finalizer_without_block_frames_never_requests_an_empty_native_probe(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before, after, config = self.finalizer_fixture(root, [])
            def forbidden_probe(*args):
                self.fail('no Block frames require no physical block preimages')
            result = audit.audit_growth(before, after, NAMES, root, root / 'binary',
                                        config, root / 'source', forbidden_probe, reconcile)
            self.assertEqual(result['warehouse_proofs'], [])
            self.assertEqual(result['causal_observer']['gathering'], [])
            self.assertEqual(json.loads((root / 'run.json').read_text())['status'], 'failed')

    def test_finalizer_without_census_keeps_real_block_preimages_and_no_warehouse_proof(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before, after, config = self.finalizer_fixture(root, [
                {'Block': {'pos': [1, 2, 3], 'state': 0}}])
            def physical_probe(binary, config, source, positions, receipt):
                self.assertEqual(positions, {(1, 2, 3)})
                return {'source_unchanged': True, 'checkpoint_tick': 0,
                        'blocks': [{'x': 1, 'y': 2, 'z': 3, 'state': 0}]}
            result = audit.audit_growth(before, after, NAMES, root, root / 'binary',
                                        config, root / 'source', physical_probe, reconcile)
            self.assertEqual(result['warehouse_proofs'], [])
            self.assertEqual(json.loads((root / 'run.json').read_text())['status'], 'failed')

    def test_retained_checkpoint_project_survives_first_frame_completion(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before, preimages = self.fixture(root)
            project = {'structure': 'retained-storage', 'blueprint': 'storage',
                       'origin': [109, 66, 106], 'builder': ['builder', None]}
            before['globals']['settlement/1'] = {
                'dimension': audit.DIMENSION, 'value': {'id': 1, 'projects': [project]}}
            write_batch(root, 1, [{'Settlement': {'image': {'id': 1, 'value': {'projects': []}}}}])
            result = audit.observe(before, root, 2, NAMES, STATES, preimages, 4, {})
            self.assertEqual(result['project_structures']['retained-storage'],
                             {'settlement': 1, 'wal': 'checkpoint/0', **project})

    def test_project_identity_survives_completed_project_removal(self):
        projects = {}
        project = {'structure': 'storage-1', 'blueprint': 'storage',
                   'origin': [109, 66, 106], 'builder': ['builder', None]}
        audit.record_projects(projects, [('Settlement', {'image': {
            'id': 1, 'value': {'projects': [project]}}})], 'creation-frame')
        audit.record_projects(projects, [('Settlement', {'image': {
            'id': 1, 'value': {'projects': []}}})], 'completion-frame')
        self.assertEqual(projects['storage-1'], {'settlement': 1, 'wal': 'creation-frame', **project})


if __name__ == '__main__':
    unittest.main()
