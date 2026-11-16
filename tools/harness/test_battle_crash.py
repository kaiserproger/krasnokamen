import unittest
from uuid import UUID

from tools.harness.battle_crash import (MOMENTS, campaign, census, combine_censuses, compare_durable, compare_entities,
                                        cut_verdict, fields, grant_images, projectile_lifetimes,
                                        published_removal, recipient_custody, simulation_tick, uuid_snbt)


class BattleCrashTest(unittest.TestCase):
    def test_census_splits_nested_snbt_without_losing_item_fields(self):
        line = ('Husk has the following entity data: {UUID: [I; 1, 2, 3, 4], '
                'equipment: {mainhand: {id: "minecraft:sword", components: {name: "a,b"}}}, '
                'Pos: [1.0d, 2.0d, 3.0d]}')
        result = census(line)
        self.assertEqual(result['[I; 1, 2, 3, 4]']['type'], 'Husk')
        self.assertIn('name: "a,b"', result['[I; 1, 2, 3, 4]']['holders']['equipment'])
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            census(line + '\n' + line)
        with self.assertRaises(ValueError):
            fields('{UUID: [I; 1, 2')
        self.assertEqual(fields('{"krasnokamen:Faction": 7, UUID: [I; 1, 2, 3, 4]}')['krasnokamen:Faction'], '7')
        with self.assertRaisesRegex(ValueError, 'duplicate SNBT field'):
            fields('{UUID: [I; 1], UUID: [I; 2]}')

    def test_grant_requires_correct_player_item_and_count_in_published_frame(self):
        player = UUID('00112233-4455-6677-8899-aabbccddeeff')
        image = {'uuid': str(player), 'inventory': [{'item_id': 1023, 'count': 5}]}
        wal = {'frames': [{'file': 'batch.bin', 'lo': 8, 'hi': 8,
                          'entries': [{'Dimension': {'entries': [{'Player': image}]}}]}]}
        self.assertEqual(grant_images(wal, player, 1023), [{'file': 'batch.bin', 'lo': 8, 'hi': 8}])
        self.assertEqual(grant_images(wal, player, 1), [])
        self.assertEqual(grant_images(wal, UUID(int=1), 1023), [])
        image['inventory'][0]['count'] = 4
        self.assertEqual(grant_images(wal, player, 1023), [])

    def test_recipient_requires_exact_stack_and_empty_cursor(self):
        granted = [{'item_id': 'minecraft:netherite_ingot', 'count': 5, 'slot': 36, 'components': {}}]
        self.assertEqual(recipient_custody(granted, granted, None), (True, True))
        self.assertEqual(recipient_custody(granted, [], None), (False, True))
        self.assertEqual(recipient_custody(granted, granted, granted[0]), (True, False))
        self.assertEqual(recipient_custody(granted, [{**granted[0], 'slot': 37}], None), (False, False))

    def test_early_entities_are_kept_when_players_rejoin_later(self):
        early = {'arrow': 'early arrow', 'soldier': 'soldier', 'player': 'stale player'}
        late = {'soldier': 'later soldier', 'player': 'rejoined', 'recipient': 'given'}
        self.assertEqual(combine_censuses(early, late, {'player', 'recipient'}, 'recipient'),
                         {'arrow': 'early arrow', 'soldier': 'soldier', 'player': 'rejoined'})

    def test_census_clock_requires_authoritative_command_reply(self):
        class Console:
            def __init__(self, reply):
                self.reply = reply

            def command(self, text):
                self.text = text
                return self.reply

        console = Console('The time is 3401')
        self.assertEqual(simulation_tick(console), 3401)
        self.assertEqual(console.text, 'time query gametime')
        self.assertEqual(simulation_tick(Console('commands.time.query: 3402')), 3402)
        with self.assertRaises(ValueError):
            simulation_tick(Console('unrelated 3403'))

    def test_inactive_old_arrow_and_low_life_arrow_are_never_excused(self):
        arrow = {'type': 'Arrow', 'holders': {}, 'life': '70s', 'in_ground': '1b'}
        before = {'old': arrow, 'fresh': {**arrow, 'life': '2s'},
                  'item': {'type': 'Item', 'holders': {'Item': 'diamond'}}}
        # A large global-time gap does not prove this arrow's chunk was active.
        self.assertEqual(compare_entities(before, {})['missing'], ['fresh', 'item', 'old'])
        self.assertEqual(compare_entities({'old': arrow}, {'new': arrow})['new'], ['new'])
        nonexpired_removed = compare_entities(before, {'old': arrow, 'item': before['item']})
        self.assertEqual(nonexpired_removed['missing'], ['fresh'])
        self.assertFalse(not (nonexpired_removed['missing'] or nonexpired_removed['new']
                             or nonexpired_removed['changed']))
        item_removed = compare_entities(before, {'old': arrow, 'fresh': before['fresh']})
        self.assertEqual(item_removed['missing'], ['item'])

    def test_old_arrow_is_required_until_upper_bound_allows_expiry(self):
        arrow = {'type': 'Arrow', 'holders': {}, 'life': '80s', 'in_ground': '1b'}
        proof = {'old': {'lifetime': 100, 'file': 'batch.bin', 'tick_hi': 10}}
        self.assertEqual(compare_entities({'old': arrow}, {}, proof, 19)['missing'], ['old'])
        possible = compare_entities({'old': arrow}, {}, proof, 20)
        self.assertEqual(possible['missing'], [])
        self.assertEqual(possible['unverified']['old']['elapsed_upper'], 20)
        self.assertEqual(cut_verdict(possible, valid=True, custody_ok=True, probe_ok=True), 'incomplete')
        self.assertEqual(compare_entities({'old': arrow}, {}, {}, 20)['missing'], ['old'])
        self.assertEqual(compare_entities({'old': arrow}, {}, proof, None)['missing'], ['old'])

    def test_projectile_lifetime_must_be_in_validated_prior_wal_image(self):
        player = UUID('00112233-4455-6677-8899-aabbccddeeff')
        entity = {'uuid': str(player), 'projectile': {'kind': 'Arrow', 'ground_lifetime': 100}}
        wal = {'frames': [{'file': 'old.bin', 'hi': 7, 'entries': [{'Entity': entity}]},
                          {'file': 'future.bin', 'hi': 9, 'entries': [{'Entity': {
                              'uuid': str(player), 'projectile': {'kind': 'Arrow', 'ground_lifetime': 1200}}}]}]}
        self.assertEqual(projectile_lifetimes(wal, 8)[uuid_snbt(str(player))],
                         {'lifetime': 100, 'file': 'old.bin', 'tick_hi': 7})
        self.assertEqual(projectile_lifetimes(wal, 6), {})

    def test_durable_holders_are_exact_and_missing_entities_fail(self):
        expected = {'husk': {'kind': 'minecraft:husk', 'holders': {'equipment': '{mainhand: sword}'}},
                    'arrow': {'kind': 'minecraft:arrow', 'holders': {}},
                    'player': {'kind': 'minecraft:player', 'holders': {'Inventory': '[]'}}}
        actual = {'husk': {'type': 'Husk', 'holders': {'equipment': '{mainhand: sword}'}},
                  'arrow': {'type': 'Arrow', 'holders': {}},
                  'player': {'type': 'Proto1', 'holders': {'Inventory': '[]'}}}
        self.assertEqual(compare_durable(expected, actual),
                         {'missing': [], 'new': [], 'changed': [], 'unverified': {}})
        switched = {**actual, 'husk': {'type': 'Husk', 'holders': {'equipment': '{mainhand: spear}'}}}
        self.assertEqual(compare_durable(expected, switched)['changed'], ['husk'])
        self.assertEqual(compare_durable(expected, {k: v for k, v in actual.items() if k != 'arrow'})['missing'],
                         ['arrow'])
        self.assertEqual(compare_durable(expected, {**actual, 'extra': actual['husk']})['new'], ['extra'])

    def test_tombstone_requires_continuous_identity_scoped_wal(self):
        ident = '00112233-4455-6677-8899-aabbccddeeff'
        image = {'Entity': {'id': 42, 'uuid': ident, 'tick': 4}}
        removal = {'RemoveEntity': {'id': 42, 'tick': 6}}
        frames = [{'lo': n, 'hi': n, 'file': f'{n}.bin', 'sha256': str(n), 'entries': entries}
                  for n, entries in [(4, [image]), (5, []), (6, [removal])]]
        wal = {'frames': frames, 'published_through': 6}
        self.assertEqual(published_removal(wal, uuid_snbt(ident), 4, 5)['removal_frame'], '6.bin')
        self.assertIsNone(published_removal(wal, uuid_snbt(ident), 4, 6))
        self.assertIsNone(published_removal({**wal, 'frames': [frames[0], frames[2]]},
                                            uuid_snbt(ident), 4, 5))
        reassigned = {'Entity': {'id': 42, 'uuid': '11112233-4455-6677-8899-aabbccddeeff', 'tick': 5}}
        changed = [frames[0], {**frames[1], 'entries': [reassigned]}, frames[2]]
        self.assertIsNone(published_removal({**wal, 'frames': changed}, uuid_snbt(ident), 4, 5))

    def test_campaign_fails_closed_when_any_moment_is_missing_or_failed(self):
        rows = [{'moment': moment, 'status': 'passed', 'moment_observed': True, 'crash_copy': '/evidence'}
                for moment in MOMENTS for _ in range(5)]
        self.assertEqual(campaign(rows)['status'], 'passed')
        self.assertEqual(campaign(rows[:-1])['status'], 'failed')
        rows[0]['status'] = 'failed'
        result = campaign(rows)
        self.assertEqual(result['status'], 'failed')
        incomplete = [{'moment': moment, 'status': 'passed', 'moment_observed': True}
                      for moment in MOMENTS for _ in range(5)]
        incomplete[0]['status'] = 'incomplete'
        summary = campaign(incomplete)
        self.assertEqual(summary['status'], 'incomplete')
        self.assertEqual(summary['moments'][MOMENTS[0]]['incomplete'], 1)
        self.assertEqual(summary['moments'][MOMENTS[0]]['failed'], 0)
        self.assertEqual(result['moments'][MOMENTS[0]]['failed'], 1)
        clean = [{'moment': moment, 'status': 'passed', 'moment_observed': True}
                 for moment in MOMENTS for _ in range(5)]
        clean.append({'moment': MOMENTS[1], 'status': 'failed', 'moment_observed': False,
                      'ledger_candidate': {'reason': 'published image missing'}})
        self.assertEqual(campaign(clean)['status'], 'failed')


if __name__ == '__main__':
    unittest.main()
