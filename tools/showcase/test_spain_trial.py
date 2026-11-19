"""Bounded received-entity census used by the real Spain trial."""
from collections import Counter
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from tools.showcase.spain_trial import hud_visible, next_order, order_build, publish, resident_census, settlement_tick_tail, spawn_on, supply


class ScratchPublishTests(unittest.TestCase):
    def test_receipt_paths_name_the_copy_and_keep_the_run_history(self):
        with tempfile.TemporaryDirectory() as base:
            # mkdtemp names scratch after --out; json.dumps escapes non-ASCII and quotes.
            scratch = Path(base) / 'испания "x"-scratch'
            (scratch / 'screenshots').mkdir(parents=True)
            frame = scratch / 'screenshots/frame-000.png'
            (scratch / 'screenshots/frame-000.json').write_text(json.dumps({'retained_path': str(frame)}))
            run = {'server': {'command': ['krasnokamen', '--config', str(scratch / 'server.toml')]},
                   'census': {'command': ['census', str(scratch / 'world')], 'receipt': str(scratch / 'census.json')},
                   'frames': [{'capture': {'path': str(frame)}}], 'scratch': str(scratch)}
            receipt = Path(base) / 'испания "x"'
            publish(scratch, receipt, run)
            published = json.loads((receipt / 'run.json').read_text())
            frame_meta = json.loads((receipt / 'screenshots/frame-000.json').read_text())
            self.assertFalse(scratch.exists())
            self.assertEqual(published['census']['receipt'], str(receipt / 'census.json'))
            self.assertEqual(published['frames'][0]['capture']['path'], str(receipt / 'screenshots/frame-000.png'))
            self.assertEqual(frame_meta['retained_path'], str(receipt / 'screenshots/frame-000.png'))
            self.assertEqual(published['as_run']['census_command'], ['census', str(scratch / 'world')])
            self.assertEqual(published['scratch'], str(scratch))


class SpawnAffinityTests(unittest.TestCase):
    def test_child_starts_on_the_role_cpus_and_the_driver_mask_returns(self):
        before = os.sched_getaffinity(0)
        cpu = min(before)
        with spawn_on({cpu}):
            child = subprocess.run([sys.executable, '-c', 'import os; print(sorted(os.sched_getaffinity(0)))'],
                                   capture_output=True, text=True, check=True)
        self.assertEqual(child.stdout.split()[-1], f'[{cpu}]')
        self.assertEqual(os.sched_getaffinity(0), before)


class HudObservationTests(unittest.TestCase):
    def test_visible_hud_is_detected_without_a_managed_setting(self):
        self.assertTrue(hud_visible({"client_settings": {}, "ui_nodes": [{"id": "hud/hotbar"}]}))
        self.assertTrue(hud_visible({"ui_nodes": [{"id": "chat/line/1"}]}))

    def test_hidden_hud_requires_no_hud_or_chat_nodes(self):
        self.assertFalse(hud_visible({"ui_nodes": []}))
        self.assertFalse(hud_visible({"ui_nodes": [{"id": "world/label"}]}))


class ResidentCensusTests(unittest.TestCase):
    def test_pages_filtered_villagers_without_large_results(self):
        probe = Mock()
        rows = [{"entity_id": n} for n in range(9)]
        probe.call.side_effect = [
            {"entities": rows[:8], "visible_count": 9, "truncated": True, "next_cursor": "7"},
            {"entities": rows[8:], "visible_count": 9, "truncated": False, "next_cursor": None},
        ]
        self.assertEqual(resident_census(probe), {
            "resident_census": rows, "resident_visible_count": 9,
        })
        for call in probe.call.call_args_list:
            name, args = call.args
            self.assertEqual(name, "minecraft_list_entities")
            self.assertEqual(args["limit"], 8)
            self.assertEqual(args["entity_type"], "minecraft:villager")
        self.assertEqual(probe.call.call_args_list[0].args[1]["cursor"], "")
        self.assertEqual(probe.call.call_args_list[1].args[1]["cursor"], "7")

    def test_empty_received_census_is_not_invented_population(self):
        probe = Mock()
        probe.call.return_value = {
            "entities": [], "visible_count": 0, "truncated": False, "next_cursor": None,
        }
        self.assertEqual(resident_census(probe)["resident_census"], [])

    def test_truncated_without_cursor_refuses_incomplete_receipt(self):
        probe = Mock()
        probe.call.return_value = {
            "entities": [], "visible_count": 9, "truncated": True, "next_cursor": None,
        }
        with self.assertRaisesRegex(RuntimeError, "truncated without"):
            resident_census(probe)

    def test_repeated_cursor_cannot_loop(self):
        probe = Mock()
        probe.call.return_value = {
            "entities": [], "visible_count": 9, "truncated": True, "next_cursor": "7",
        }
        with self.assertRaisesRegex(RuntimeError, "did not advance"):
            resident_census(probe)
        self.assertEqual(probe.call.call_count, 2)


class ScriptedChat:
    """A client probe whose server chat arrives one line per poll, newest first like the client buffer."""

    def __init__(self, replies):
        self.replies, self.chat, self.check = list(replies), [], Mock()

    def call(self, tool, args):
        assert tool == 'minecraft_send_chat', tool
        if not args['command']:
            self.chat.insert(0, f"<SpainCulture> {args['message']}")
        return {}

    def wait(self, predicate, label, timeout_seconds=None):
        state = {'recent_chat': list(self.chat)}
        while not predicate(state):
            if not self.replies:
                raise TimeoutError(f'timed out waiting for {label}')
            self.chat.insert(0, self.replies.pop(0))
            state = {'recent_chat': list(self.chat)}
        return state


class SupplyTests(unittest.TestCase):
    def test_tops_up_later_chest_before_spending_earlier_empty_slots(self):
        first = {slot: {'item_id': 'minecraft:stone', 'count': 64} for slot in range(26)}
        second = {slot: {'item_id': 'minecraft:stone', 'count': 64} for slot in range(27)}
        second[0] = {'item_id': 'minecraft:terracotta', 'count': 40}
        stock, run, console = Counter(terracotta=24, white_bed=1), {}, Mock()
        with patch('tools.showcase.spain_trial.village._open_chest', side_effect=[first, second]):
            supply(Mock(), console, [(1, 2, 3), (4, 5, 6)], stock, run)
        self.assertEqual(stock, {})
        self.assertEqual(run['material_fixture_commands'], [
            'item replace block 4 5 6 container.0 with minecraft:terracotta 64',
            'item replace block 1 2 3 container.26 with minecraft:white_bed 1',
        ])
        self.assertEqual(second[0]['count'], 40, 'top-up planning does not pretend the console ran')

    def test_full_storage_retains_the_bounded_remainder_for_a_later_census(self):
        slots = {slot: {'item_id': 'minecraft:stone', 'count': 64} for slot in range(27)}
        console, stock, run = Mock(), Counter(white_bed=1), {}
        with patch('tools.showcase.spain_trial.village._open_chest', return_value=slots):
            supply(Mock(), console, [(1, 2, 3)], stock, run)
        self.assertEqual(stock, {'white_bed': 1})
        self.assertEqual(run['material_fixture_remaining'], {'white_bed': 1})
        self.assertEqual(run['material_fixture_commands'], [])
        del slots[26]  # ordinary construction made room before the next census
        with patch('tools.showcase.spain_trial.village._open_chest', return_value=slots):
            supply(Mock(), console, [(1, 2, 3)], stock, run)
        self.assertEqual(stock, {})
        self.assertEqual(run['material_fixture_remaining'], {})
        self.assertEqual(run['material_fixture_commands'], [
            'item replace block 1 2 3 container.26 with minecraft:white_bed 1'])


class BuildingQueueTests(unittest.TestCase):
    def test_autonomous_completed_fortress_is_not_ordered_again(self):
        parent = {'buildings': [{'blueprint': 'spanish_church'}, {'blueprint': 'spanish_fortress'}],
                  'project': {'projects': []}}
        queue = ['spanish_fortress']
        self.assertIsNone(next_order(queue, parent))
        self.assertEqual(queue, [])

    def test_active_autonomous_building_is_skipped_before_the_next_missing_one(self):
        parent = {'buildings': [{'blueprint': 'spanish_house'}],
                  'project': {'projects': [{'blueprint': 'spanish_church'}]}}
        queue = ['spanish_house', 'spanish_church', 'spanish_fortress']
        self.assertEqual(next_order(queue, parent), 'spanish_fortress')
        self.assertEqual(queue, ['spanish_fortress'])

    def test_missing_building_remains_at_the_head_until_admitted(self):
        queue = ['spanish_house', 'spanish_church']
        self.assertEqual(next_order(queue, {'buildings': [], 'project': {'projects': []}}), 'spanish_house')
        self.assertEqual(queue, ['spanish_house', 'spanish_church'])


class BuildingOrderTests(unittest.TestCase):
    def test_started_and_queued_are_native_admission_not_client_send_success(self):
        for reply in ['Started spanish_house construction.', 'Queued spanish_house: busy']:
            with self.subTest(reply=reply):
                probe, run = ScriptedChat([reply]), {}
                order_build(probe, 'spanish_house', run)
                order = dict(run['building_orders'][0])
                self.assertIsInstance(order.pop('reply_ms_with_marker'), int)
                self.assertEqual(order, {
                    'command': 'settlement build spanish_house',
                    'recent_chat': [reply], 'admitted': True,
                })
                probe.check.assert_called_once_with('spanish_house_order_admitted', True)

    def test_an_unrelated_line_before_the_reply_does_not_decide_the_order(self):
        notice = 'village_00000001: a resident arrived.'
        probe, run = ScriptedChat([notice, 'Started spanish_house construction.']), {}
        order_build(probe, 'spanish_house', run)
        self.assertTrue(run['building_orders'][0]['admitted'])
        self.assertEqual(run['building_orders'][0]['recent_chat'],
                         ['Started spanish_house construction.', notice])

    def test_laptop_site_refusal_fails_immediately_and_retains_its_cause(self):
        reply = 'No free loaded construction site is available within the material budget.'
        probe, run = ScriptedChat([reply, 'Started spanish_house construction.']), {}
        with self.assertRaisesRegex(RuntimeError, 'spanish_house.*not admitted.*No free loaded'):
            order_build(probe, 'spanish_house', run)
        self.assertFalse(run['building_orders'][0]['admitted'])
        self.assertEqual(run['building_orders'][0]['recent_chat'], [reply])
        probe.check.assert_not_called()

    def test_absent_or_other_blueprint_acknowledgement_cannot_advance_order(self):
        for lines in [[], ['Started spanish_plaza construction.']]:
            with self.subTest(lines=lines):
                probe, run = ScriptedChat(lines), {}
                with self.assertRaises(TimeoutError):
                    order_build(probe, 'spanish_house', run)
                probe.check.assert_not_called()


class SettlementTickTailTests(unittest.TestCase):
    def test_native_laps_are_joined_to_search_ticks_without_reusing_a_previous_lap(self):
        result = settlement_tick_tail([
            'DEBUG settlement_storehouse_search game_tick=20 settlement=1 attempts=64\n',
            '  commit detail ms: bees_conversion=0.01 settlements=120.01 villager_brain=1.00\n',
            'phase ms: tick=20 intents=0 dimension=overworld commit=121.02\n',
            '  commit detail ms: settlements=100.00 villager_brain=1.00\n',
            'phase ms: tick=21 intents=0 dimension=overworld commit=101.00\n',
            '  commit detail ms: settlements=3.50 villager_brain=1.00\n',
            'phase ms: tick=22 intents=0 dimension=overworld commit=4.50\n',
            'phase ms: tick=23 intents=0 dimension=overworld commit=1.00\n',
        ])
        self.assertTrue(result['enabled'])
        self.assertEqual(result['search_ticks'], 1)
        self.assertEqual(result['with_storehouse_search'],
                         {'samples': 1, 'max_ms': 120.01, 'over_100ms': 1})
        self.assertEqual(result['without_storehouse_search'],
                         {'samples': 2, 'max_ms': 100.0, 'over_100ms': 0})

    def test_missing_opt_in_phase_log_is_explicit_not_a_zero_cost_measurement(self):
        result = settlement_tick_tail(['DEBUG settlement_storehouse_search game_tick=20\n'])
        self.assertFalse(result['enabled'])
        self.assertEqual(result['with_storehouse_search'],
                         {'samples': 0, 'max_ms': None, 'over_100ms': 0})


if __name__ == "__main__":
    unittest.main()
