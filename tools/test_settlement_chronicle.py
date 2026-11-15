import copy
import unittest

from tools.settlement_chronicle import audit_events, extract


def town(sid=1, population=4, buildings=None):
    return {"id": sid, "name": f"Town{sid}", "owner": "owner-uuid", "population": population,
            "buildings": buildings or []}


def run(*censuses):
    return {"seed": 100, "server": {"head": "candidate", "sha256": "binary-hash"},
            "snapshots": [{"tick": tick, "game_time": age, "settlements": towns}
                          for tick, age, towns in censuses]}


class ChronicleTests(unittest.TestCase):
    def test_baseline_is_not_founding_or_completion(self):
        data = run((100, 10, [town(buildings=[{"id": 11, "level": 1, "blueprint": "spanish_house"}])]),
                   (200, 110, [town(buildings=[{"id": 11, "level": 1, "blueprint": "spanish_house"}])]))
        self.assertEqual(extract(data, [])['events'], [])

    def test_real_changes_have_interval_dates_and_no_invented_lineage(self):
        data = run((100, 23900, [town()]),
                   (400, 24200, [town(population=10, buildings=[{"id": 11, "level": 1, "blueprint": "spanish_house"}]), town(2)]),
                   (500, 24300, [town(population=10), town(2)]))
        result = extract(data, [{"tick": 100}, {"tick": 102}, {"tick": 500}])
        events = result['events']
        self.assertEqual([e['population'] for e in events if e['kind'] == 'population_milestone'], [5, 10])
        building = next(e for e in events if e['kind'] == 'building_completed')
        self.assertEqual((building['after_tick'], building['tick'], building['day']), (100, 400, 1))
        self.assertTrue(building['caption'].endswith('(observed by this time)'))
        self.assertEqual(sum(e['kind'] == 'founded' for e in events), 1)
        self.assertFalse(any(e['kind'] == 'daughter_founded' for e in events))
        self.assertEqual(result['coverage']['interior_missing_ticks'], 398)

    def test_exact_native_audit_replaces_interval_completion_and_proves_daughter(self):
        data = run((100, 0, [town()]), (500, 400, [town(buildings=[{"id": 11, "level": 1, "blueprint": "spanish_house"}]), town(2)]))
        log = ('INFO settlement_audit: settlement_building_completed settlement=1 building=11 blueprint=spanish_house level=1 tick=300 game_time=200\n'
               'INFO settlement_audit: settlement_daughter_founded parent=1 settlement=2 tick=400 game_time=300\n')
        result = extract(data, [], log)
        self.assertEqual([e['kind'] for e in result['events']], ['building_completed', 'daughter_founded'])
        self.assertEqual(result['events'][0]['tick'], 300)
        self.assertTrue(all(e['precision'] == 'native_audit' for e in result['events']))
        self.assertIn('daughter of Town1', result['events'][1]['caption'])
        with self.assertRaisesRegex(ValueError, 'same-owner'):
            invalid = copy.deepcopy(data)
            invalid['snapshots'][-1]['settlements'][-1]['owner'] = 'foreign'
            extract(invalid, [], log)

    def test_missing_audit_fields_and_uncertified_completion_refuse(self):
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            audit_events('settlement_daughter_founded settlement=2 tick=12 game_time=12')
        data = run((1, 1, [town()]), (500, 500, [town()]))
        with self.assertRaisesRegex(ValueError, 'certificate'):
            extract(data, [], 'settlement_building_completed settlement=1 building=11 blueprint=spanish_house level=1 tick=300 game_time=300')

    def test_reappearance_and_population_recrossing_are_not_new_events(self):
        data = run((1, 1, [town(population=5)]), (2, 2, []), (3, 3, [town(population=4)]), (4, 4, [town(population=5)]))
        self.assertEqual(extract(data, [])['events'], [])

    def test_new_town_inherits_buildings_without_fake_completion(self):
        data = run((1, 1, [town()]),
                   (2, 2, [town(), town(2, buildings=[{"id": 21, "level": 1, "blueprint": "spanish_house"}])]))
        events = extract(data, [])['events']
        self.assertEqual([event['kind'] for event in events], ['founded'])

    def test_reversed_clocks_duplicate_telemetry_and_bad_milestones_refuse(self):
        with self.assertRaises(ValueError):
            extract(run((2, 2, []), (1, 1, [])), [])
        data = run((1, 1, []), (2, 2, []))
        with self.assertRaises(ValueError):
            extract(data, [{'tick': 1}, {'tick': 1}])
        with self.assertRaises(ValueError):
            extract(data, [], milestone=0)


if __name__ == '__main__':
    unittest.main()
