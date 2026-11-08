"""Fail-closed native wave selection and canonical receipt-to-ledger tests."""
import json
import tempfile
import unittest
from pathlib import Path

from tools.harness.backends import native_matrix


class NativeMatrixTests(unittest.TestCase):
    def test_native_selection_preserves_windows_as_explicit_blockers(self):
        rows = [{"id": "linux", "context": {"client_kind": "native_linux"}},
                {"id": "windows", "context": {"client_kind": "native_windows"}},
                {"id": "java", "context": {"client_kind": "vanilla_java_26_1_2"}}]
        self.assertEqual([row["id"] for row in native_matrix.select(rows)], ["linux", "windows"])

    def test_receipt_records_exact_status_without_promoting_blocked(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            ledger = path / "ledger.jsonl"
            receipt = {"details": {"matrix": {"ledger": str(ledger), "server_sha": "a" * 64,
                "rows": [{"id": "tr:a", "status": "blocked", "run_dir": str(path),
                          "note": "windows_mcp_lane"}]}}}
            native_matrix.record_receipt(receipt, path / "result.json")
            line = json.loads(ledger.read_text())
            self.assertEqual(set(line), {"id", "status", "driver", "server_sha", "run_dir", "ts", "note"})
            self.assertEqual(line["status"], "blocked")
            self.assertEqual(line["driver"], "native_mcp")
            self.assertTrue(line["ts"].endswith("Z"))
            self.assertIn("windows_mcp_lane", line["note"])

    def test_pending_uses_last_ledger_status_and_never_hides_blocked(self):
        rows = [{"id": "x"}, {"id": "y"}]
        self.assertEqual(native_matrix.pending(rows, {"x": "pass", "y": "blocked"}), [{"id": "y"}])

    def test_missing_native_network_tools_are_precise(self):
        scenario = {"context": {"client_kind": "native_linux", "network_condition": "slow_reader_backpressure"}}
        self.assertEqual(native_matrix.transport_blocker(scenario), "slow_reader_backpressure: native MCP exposes no socket-reader rate control")
        scenario["context"]["client_kind"] = "native_windows"
        self.assertEqual(native_matrix.transport_blocker(scenario), "windows_mcp_lane: Windows VM is Main-owned; Linux evidence cannot substitute")

    def test_native_snapshot_normalizes_received_menu_and_entity_identity(self):
        from tools.harness.backends.native_matrix_client import NativeMatrixClient
        class Mcp:
            def call_tool(self, tool, arguments):
                if tool == 'minecraft_list_entities':
                    return {'entities':[{'entity_id':7,'entity_uuid':'u','entity_type':'minecraft:cow',
                                        'x':1,'y':2,'z':3,'health':10}], 'truncated':False}
                return {'in_play':True,'player':{'x':1,'y':2,'z':3,'health':20,'food':19,
                                               'experience_level':4,'total_experience':40},
                        'container':{'container_id':2,'container_state_id':9,'slots':[
                            {'slot':0,'item_id':'minecraft:iron_pickaxe','count':1,'damage':10,
                             'max_damage':250,'enchantments':[{'id':'minecraft:mending','level':1}]}],
                            'trades':{'offers':[{'buy_a':{'item_id':'minecraft:emerald','count':2},
                                'buy_b':None,'sell':{'item_id':'minecraft:bread','count':3},'uses':1,'max_uses':4}],
                                      'villager_level':2,'villager_xp':10,'can_restock':True},
                            'carried':{'item_id':'minecraft:diamond','count':1}},
                        'inventory':[{'slot':40,'item_id':'minecraft:torch','count':1},
                                     {'slot':41,'item_id':None,'count':None}],
                        'recent_chat':['received']}
        result = NativeMatrixClient('test',Mcp(),'127.0.0.1:1',[]).snapshot()
        self.assertEqual(result['menu']['state_id'],9)
        self.assertEqual(result['entities'][0]['id'],7)
        self.assertEqual(result['entities'][0]['position'],[1,2,3])
        self.assertNotIn('profession',result['entities'][0])
        self.assertEqual(result['cursor']['count'],1)
        self.assertEqual([s['slot'] for s in result['inventory']],[45])
        self.assertEqual(result['experience_level'],4)
        self.assertEqual(result['experience_total'],40)
        self.assertEqual(result['menu']['slots'][0]['components']['minecraft:damage'],10)
        self.assertEqual(result['menu']['slots'][0]['components']['minecraft:enchantments'],
                         {'minecraft:mending':1})
        self.assertEqual(result['menu']['offers'][0]['input1']['count'],2)
        self.assertEqual(result['menu']['offers'][0]['output']['count'],3)
        self.assertEqual(result['menu']['offers'][0]['uses'],1)
        self.assertIsNone(result['menu']['offers'][0]['input2'])
        self.assertEqual(result['menu']['level'],2)

    def test_native_wait_ticks_respects_live_tool_bound(self):
        from tools.harness.backends.native_matrix_client import NativeMatrixClient
        class Mcp:
            def __init__(self):
                self.ticks=[]
            def call_tool(self, tool, arguments):
                self.ticks.append(arguments['ticks'])
                return {}
        mcp=Mcp()
        NativeMatrixClient('test',mcp,'127.0.0.1:1',[]).wait_ticks(600)
        self.assertEqual(mcp.ticks,[255,255,90])

    def test_canonical_native_profile_accepts_immutable_candidate(self):
        from unittest.mock import patch
        from tools.harness import __main__ as cli, profiles
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            spec={'scope':'real-client','description':'dispatch unit test',
                  'prerequisites': ((), (), (), ()), 'run':lambda ctx:{'dispatch_only':True}}
            with patch.object(cli,'ARTIFACT_ROOT',root), patch.dict(profiles.PROFILES,{'native-matrix':spec}):
                code=cli.cmd_run(['native-matrix','--client','native','--windowed',
                                  '--server-bin',str(root/'immutable-server')])
            self.assertEqual(code,0)
            receipt=json.loads(next(root.glob('*/result.json')).read_text())
            self.assertEqual(receipt['client'],'native')
            self.assertEqual(receipt['status'],'passed')
            self.assertEqual(receipt['commands'],[])

    def test_native_launch_fails_closed_before_process_without_private_audio(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        from tools.harness.matrix_api import MissingCapability
        lane=object.__new__(native_matrix.Lane)
        lane.directory=Path('/unused')
        lane.processes=[]
        lane.ctx=SimpleNamespace(client_evidence={})
        with patch('tools.harness.audio.private_audio_preflight',
                   side_effect=MissingCapability('private_audio:wrapper_required')), \
             patch.object(native_matrix.native_client,'start') as start:
            with self.assertRaises(MissingCapability):
                lane.launch('test')
            start.assert_not_called()

    def test_profile_fingerprint_is_retained_in_ledger(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            receipt={'details':{'matrix':{'ledger':str(root/'ledger.jsonl'),'server_sha':'a'*64,
                'rows':[{'id':'tr:a@live','status':'blocked','run_dir':str(root),
                         'note':'capability','plugin_set':'b'*64}]}}}
            native_matrix.record_receipt(receipt,root/'result.json')
            self.assertEqual(json.loads((root/'ledger.jsonl').read_text())['plugin_set'],'b'*64)

    def test_composter_selects_seven_received_single_cakes(self):
        from types import SimpleNamespace
        from unittest.mock import Mock, patch
        from tools.harness.backends import native_matrix_interactions as subject
        steps=Mock()
        steps.wait.return_value={'player':{'x':0,'y':100,'z':0,'on_ground':True}}
        steps.call.return_value={'entities':[]}
        lane=SimpleNamespace(steps=steps,names=['test'],console=Mock(),custody_boundary=Mock())
        with patch.object(subject,'_use'),patch.object(subject,'_block'):
            subject.run(lane,{'subject':'block_interact.composter','context':{'player_count':'1'}})
        selects=[call.args[1]['count'] for call in steps.call.call_args_list
                 if call.args[0]=='minecraft_select_hotbar_item']
        self.assertEqual(selects,[1]*7)
        self.assertEqual(lane.custody_boundary.call_count,1)

    def test_sculk_latch_matrix_dispatch_requires_steady_normal_cell(self):
        from types import SimpleNamespace
        from unittest.mock import Mock, patch
        from tools.harness.backends import native_matrix_interactions as subject
        steps = Mock()
        steps.wait.return_value = {'player': {'x': 0, 'y': 100, 'z': 0, 'on_ground': True}}
        lane = SimpleNamespace(steps=steps, names=['test'], console=Mock(), custody_boundary=Mock())
        context = {'player_count': '1', 'network_condition': 'normal', 'server_lifecycle': 'steady'}
        with patch.object(subject, '_sculk_latch') as latch:
            subject.run(lane, {'subject': 'redstone.sculk_sensor_signal', 'context': context})
            latch.assert_called_once_with(steps, (0, 100, 0), fixture=lane.console)
            with self.assertRaises(NotImplementedError):
                subject.run(lane, {'subject': 'redstone.sculk_sensor_signal',
                                   'context': {**context, 'server_lifecycle': 'save_all_checkpoint'}})

    def test_ground_custody_copy_is_not_ignored(self):
        from unittest.mock import Mock
        from tools.harness.backends.native_matrix_custody import _no_ground_copy
        steps=Mock()
        steps.call.return_value={'truncated':False,'entities':[
            {'item':{'item_id':'minecraft:torch','count':1}}]}
        _no_ground_copy(steps,{'minecraft:torch'},'conserved')
        steps.check.assert_called_once_with('conserved',False)


if __name__ == "__main__":
    unittest.main()
