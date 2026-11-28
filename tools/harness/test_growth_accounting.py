import copy
from pathlib import Path
import os
import tempfile
import unittest

from .growth_accounting import custody, live_carries, reconcile
from .journal_witness import JournalWitness


class AccountingTests(unittest.TestCase):
    names = {1: 'minecraft:dirt', 2: 'minecraft:wheat', 3: 'minecraft:oak_log', 4: 'minecraft:oak_planks', 5: 'minecraft:chest'}

    def test_zero_inventory_identity_is_not_custody_but_any_item_still_is(self):
        audit = self.snapshot(64)
        actor = {'uuid': 'unsettled', 'item': None, 'villager': {'inventory': [None]}}
        audit['dimensions'] = {'overworld': {'entities': {'1': actor}, 'players': {}}}
        live = live_carries({'resident': 'Villager has the following entity data: []'})
        self.assertEqual(custody(audit, self.names)['carries'], live)
        actor['villager']['inventory'] = [{'item_id': 1, 'count': 1}]
        self.assertNotEqual(custody(audit, self.names)['carries'], live)
        self.assertEqual(custody(audit, self.names)['categories']['villager'], {'minecraft:dirt': 1})

    def snapshot(self, dirt):
        return {'banks': [{'position': [1, 2, 3], 'slots': [{'item_id': 1, 'count': dirt}]}],
                'dimensions': {}, 'structure_postimages': {}}

    def receipt(self, cut=0, consumed=0):
        return {'snapshot': {'watermark': 1, 'stages': [{'materials': [{'resource': 'minecraft:dirt', 'quantity': 1}]}],
                             'consumed': [{'resource': 'minecraft:dirt', 'quantity': consumed}]},
                'cut_credits': {'1': cut} if cut else {}, 'crafting_delta': {}}

    def test_exact_cut_and_fill_balance(self):
        before, after = self.snapshot(64), self.snapshot(63)
        after['structure_postimages']['structure/a'] = self.receipt(cut=1, consumed=2)
        self.assertEqual(reconcile(before, after, self.names)['status'], 'PASS')
        after['banks'][0]['slots'][0]['count'] -= 1
        self.assertEqual(reconcile(before, after, self.names)['unexplained_construction_delta'], {'minecraft:dirt': -1})

    def test_missing_receipt_cannot_reclassify_known_soil_as_unrelated(self):
        result = reconcile(self.snapshot(64), self.snapshot(65), self.names, {'minecraft:dirt'})
        self.assertEqual(result['status'], 'FAIL')
        self.assertEqual(result['unexplained_construction_delta'], {'minecraft:dirt': 1})

    def test_transient_ground_custody_fails_even_after_pickup(self):
        before, after = self.snapshot(64), self.snapshot(65)
        after['structure_postimages']['structure/a'] = self.receipt(cut=1)
        after['ground_witnesses'] = {'1': {'tick': 17, 'uuid': 'ground-soil', 'count': 1}}
        result = reconcile(before, after, self.names)
        self.assertEqual(result['unexplained_construction_delta'], {})
        self.assertEqual(result['status'], 'FAIL')
        self.assertIn('minecraft:dirt', result['construction_kinds_on_ground'])

    def test_unfreeze_preserves_natural_ground_custody_but_rejects_any_loss(self):
        before = self.snapshot(64)
        before['dimensions'] = {'overworld': {'entities': {'1': {
            'uuid': 'natural-drop', 'item': {'item_id': 2, 'count': 3}, 'villager': None,
        }}, 'players': {}}}
        after = copy.deepcopy(before)
        required = set(custody(before, self.names)['total'])
        intact = reconcile(before, after, self.names, required, require_no_ground=False)
        self.assertEqual(intact['status'], 'PASS')
        self.assertEqual(intact['unexplained_construction_delta'], {})
        self.assertEqual(reconcile(before, after, self.names, required)['status'], 'FAIL')
        after['dimensions']['overworld']['entities']['1']['item']['count'] -= 1
        lost = reconcile(before, after, self.names, required, require_no_ground=False)
        self.assertEqual(lost['status'], 'FAIL')
        self.assertEqual(lost['unexplained_construction_delta'], {'minecraft:wheat': -1})

    def test_unrelated_loss_is_exposed_raw_not_balanced_away(self):
        before, after = self.snapshot(64), self.snapshot(64)
        before['banks'][0]['slots'].append({'item_id': 2, 'count': 3})
        result = reconcile(before, after, self.names, {'minecraft:dirt'})
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual(result['unrelated_raw']['minecraft:wheat']['residual'], -3)
        self.assertEqual(result['unrelated_raw']['minecraft:wheat']['before'], 3)
        self.assertEqual(result['unrelated_raw']['minecraft:wheat']['after'], 0)

    def test_recipe_inputs_and_outputs_are_both_required(self):
        before, after = self.snapshot(64), self.snapshot(64)
        before['banks'][0]['slots'].append({'item_id': 3, 'count': 1})
        after['banks'][0]['slots'].append({'item_id': 4, 'count': 4})
        record = self.receipt()
        record['crafting_delta'] = {'3': -1, '4': 4}
        after['structure_postimages']['structure/a'] = record
        self.assertEqual(reconcile(before, after, self.names)['status'], 'PASS')
        del record['crafting_delta']['3']
        self.assertEqual(reconcile(before, after, self.names, {'minecraft:oak_log'})['status'], 'FAIL')

    def test_zero_net_intermediate_input_is_still_exactly_reconciled(self):
        before, after = self.snapshot(64), self.snapshot(64)
        before['banks'][0]['slots'].append({'item_id': 3, 'count': 2})
        after['banks'][0]['slots'].append({'item_id': 5, 'count': 1})
        record = self.receipt()
        record['crafting_delta'] = {'3': -2, '4': 0, '5': 1}
        after['structure_postimages']['structure/a'] = record
        self.assertEqual(reconcile(before, after, self.names)['status'], 'PASS')
        after['banks'][0]['slots'].append({'item_id': 4, 'count': 1})
        result = reconcile(before, after, self.names)
        self.assertEqual(result['status'], 'FAIL')
        self.assertEqual(result['unexplained_construction_delta'], {'minecraft:oak_planks': 1})

    def test_atomic_journal_publication_is_archived_without_polling(self):
        with tempfile.TemporaryDirectory() as root:
            source, archive = Path(root) / 'source', Path(root) / 'archive'
            source.mkdir()
            watcher = JournalWitness(source, archive)
            temporary = source / 'batch-00000000000000000001-00000000000000000001.bin.tmp'
            temporary.write_bytes(b'published native batch')
            published = temporary.with_suffix('')
            os.rename(temporary, published)
            watcher.wait_published(1, 5)
            watcher.close()
            self.assertEqual((archive / published.name).read_bytes(), b'published native batch')


if __name__ == '__main__':
    unittest.main()
