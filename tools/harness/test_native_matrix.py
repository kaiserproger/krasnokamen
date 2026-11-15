"""Fail-closed native wave selection and canonical receipt-to-ledger tests."""
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from tools.harness.matrix_api import MissingCapability

from tools.harness.backends import native_matrix


class NativeMatrixTests(unittest.TestCase):
    def test_wave_never_credits_missing_slow_or_restart_gap_health(self):
        from unittest.mock import MagicMock

        baseline = {'latest_tick': 10, 'source_skipped_ticks_total': 0,
                    'instance_started_unix_ns': 42}
        good = dict(baseline, latest_tick=11, after_tick=10, overrun=False,
                    samples=[{'tick': 11, 'wall_us': 1000}])
        for fault, expected in (('none', 'pass'), ('slow', 'fail'),
                                ('missing', 'blocked'), ('log', 'fail'), ('restart_gap', 'blocked')):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                binary = root / 'server'
                binary.write_bytes(b'server')
                log = root / 'server.log'
                log.write_text('INFO ready\n')
                row = {'id': 'case', 'subject': 'containers.chest', 'origin': 'full', 'context': {
                    'client_kind': 'native_linux', 'network_condition': 'normal',
                    'server_lifecycle': 'steady', 'player_count': '1',
                    'game_mode': 'survival', 'dimension': 'overworld'}}
                matrix = root / 'matrix.json'
                matrix.write_text(json.dumps({'scenarios': [row]}))
                ctx = SimpleNamespace(extra_args=['--matrix', str(matrix), '--ledger', str(root/'ledger'),
                    '--run-root', str(root/'runs')], client='native', windowed=True, server_bin=binary,
                    plugins_dir=root, timeout_seconds=5, artifact_dir=root/'artifact',
                    client_evidence={}, commands=[], cwds=[], exits=[], logs=[])
                ticks = iter((1, 2))
                host = SimpleNamespace(enabled_plugins=[], running=True, _process=object(), log_path=log,
                    dashboard_full_stats=lambda: {'simulation_tick': next(ticks),
                        'tick': {'source_tick': 2, 'samples': 1, 'p50_us': 1000}})
                client = SimpleNamespace(call_tool=lambda name, *args:
                    {'status': 'ok'} if name == 'minecraft_wait_ticks' else {'in_play': True})
                lane = SimpleNamespace(host=host, clients=[client])
                manager = MagicMock()
                manager.__enter__.return_value = lane
                manager.__exit__.return_value = False
                final = dict(good)
                if fault == 'slow':
                    final['samples'] = [{'tick': 11, 'wall_us': 50000}]
                elif fault == 'missing':
                    final['samples'] = []

                def action(lane, scenario):
                    if fault == 'restart_gap':
                        lane.health.restart_boundaries.append({'kind': 'clean'})

                def capture(*args):
                    if fault == 'log':
                        log.write_text('ERROR during final capture\n')

                with patch.object(native_matrix.native_client, 'resolve_binary', return_value=binary), \
                     patch.object(native_matrix.native_client, 'capture', side_effect=capture), \
                     patch.object(native_matrix, 'Lane', return_value=manager), \
                     patch.object(native_matrix, 'storage', side_effect=action), \
                     patch.object(native_matrix.HealthInterval, '_snapshot', side_effect=[baseline, final]), \
                     patch('tools.harness.matrix_execution.select_plugin_profile', return_value=([], None)), \
                     patch('tools.harness.matrix_execution.copy_plugins', return_value=[]), \
                     patch('tools.harness.matrix_execution.plugin_set_fingerprint', return_value='sha'):
                    if expected == 'pass':
                        native_matrix.run(ctx)
                    else:
                        with self.assertRaisesRegex(RuntimeError, 'native matrix incomplete'):
                            native_matrix.run(ctx)
                self.assertEqual(ctx.client_evidence['matrix']['rows'][0]['status'], expected,
                                 f'wave credited incomplete or unhealthy action interval: {fault}')

    def test_run_classifies_startup_and_oracle_exceptions(self):
        import yaml
        from types import SimpleNamespace
        from unittest.mock import MagicMock
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            binary=root/'server'; binary.write_bytes(b'server')
            row={'id':'case','subject':'containers.chest','origin':'full','context':{
                'client_kind':'native_linux','network_condition':'normal',
                'server_lifecycle':'steady','player_count':'1','game_mode':'survival',
                'dimension':'overworld'}}
            matrix=root/'matrix.json'; matrix.write_text(json.dumps({'scenarios':[row]}))
            for startup,expected in ((True,'blocked'),(False,'fail')):
                run_root=root/('startup' if startup else 'oracle')
                ctx=SimpleNamespace(extra_args=['--matrix',str(matrix),'--ledger',str(root/'ledger'),
                    '--run-root',str(run_root)],client='native',windowed=True,server_bin=binary,
                    plugins_dir=root,timeout_seconds=5,artifact_dir=root/'artifact',
                    client_evidence={},commands=[],cwds=[],exits=[],logs=[])
                host=SimpleNamespace(enabled_plugins=[], log_path=root/'server.log')
                lane=SimpleNamespace(host=host,clients=[object()])
                manager=MagicMock()
                manager.__enter__.side_effect=RuntimeError('missing block_collision_shapes') if startup else None
                manager.__enter__.return_value=lane
                manager.__exit__.return_value=False
                with patch.object(native_matrix.native_client,'resolve_binary',return_value=binary), \
                     patch.object(native_matrix,'Lane',return_value=manager), \
                     patch.object(native_matrix,'HealthInterval',return_value=SimpleNamespace(segments=[],restart_boundaries=[])), \
                     patch.object(native_matrix,'storage',side_effect=AssertionError('oracle failed')) as oracle, \
                     patch('tools.harness.matrix_execution.select_plugin_profile',return_value=([],None)), \
                     patch('tools.harness.matrix_execution.copy_plugins',return_value=[]), \
                     patch('tools.harness.matrix_execution.plugin_set_fingerprint',return_value='sha'):
                    with self.assertRaisesRegex(RuntimeError,'native matrix incomplete'):
                        native_matrix.run(ctx)
                if startup:
                    oracle.assert_not_called()
                else:
                    oracle.assert_called_once()
                self.assertEqual(ctx.client_evidence['matrix']['rows'][0]['status'],expected)
                receipt={'details':{'matrix':ctx.client_evidence['matrix']}}
                native_matrix.record_receipt(receipt,run_root/'result.json')
                summary=yaml.safe_load((run_root/'summary.yaml').read_text())
                self.assertEqual(summary['executed'],0 if startup else 1)
                self.assertEqual(summary['fail'],0 if startup else 1)

    def test_health_rejects_dead_server_errors_missing_metrics_and_disconnect(self):
        from types import SimpleNamespace
        from tools.harness.backends.probe_steps import ProbeSteps
        with tempfile.TemporaryDirectory() as directory:
            log=Path(directory)/'server.log'
            log.write_text('INFO ready\n')
            current=[0]
            tick={'samples':10,'p50_us':49000,'source_tick':2}
            def stats():
                current[0]+=1
                return {'simulation_tick':current[0], 'tick':tick}
            host=SimpleNamespace(running=True,log_path=log,dashboard_full_stats=stats)
            client=SimpleNamespace(call_tool=lambda name,*args:
                {'status':'ok'} if name=='minecraft_wait_ticks' else {'in_play':True})
            lane=SimpleNamespace(host=host,clients=[client])
            def check():
                current[0]=0
                steps=ProbeSteps(client,Path(directory),10,'health')
                native_matrix.check_health(lane,steps)
                return steps.report['assertions']
            self.assertTrue(all(check().values()))
            for message in ('ERROR broken', 'thread panicked'):
                log.write_text(message)
                with self.assertRaisesRegex(RuntimeError,'server_no_error_or_panic'):
                    check()
            log.write_text('')
            host.running=False
            with self.assertRaisesRegex(RuntimeError,'server_alive'):
                check()
            host.running=True
            for invalid in ({'source_tick':2}, {'samples':0,'p50_us':1,'source_tick':2},
                            {'samples':1,'p50_us':50000,'source_tick':2}):
                tick.clear(); tick.update(invalid)
                current[0]=0
                with self.assertRaisesRegex(RuntimeError,'server_mspt_below_50'):
                    check()
            tick.update({'samples':1,'p50_us':1,'source_tick':2})
            current[0]=0
            lane.clients=[]
            with self.assertRaisesRegex(RuntimeError,'clients_present'):
                check()
            lane.clients=[client]
            client.call_tool=lambda *args:{}
            with self.assertRaisesRegex(RuntimeError,'client_0_in_play'):
                check()

    def test_health_rejects_frozen_server_despite_client_ticks(self):
        from types import SimpleNamespace
        from tools.harness.backends.probe_steps import ProbeSteps
        with tempfile.TemporaryDirectory() as directory:
            log=Path(directory)/'server.log'
            log.write_text('INFO ready\n')
            host=SimpleNamespace(running=True,log_path=log,dashboard_full_stats=lambda:{
                'simulation_tick':1,'tick':{'source_tick':1,'samples':2,'p50_us':1}})
            client=SimpleNamespace(call_tool=lambda name,*args:
                {'status':'ok'} if name=='minecraft_wait_ticks' else {'in_play':True})
            lane=SimpleNamespace(host=host,clients=[client])
            steps=ProbeSteps(client,Path(directory),10,'health')
            with patch.object(steps,'remaining',side_effect=[1,1,TimeoutError('frozen')]):
                with self.assertRaisesRegex(TimeoutError,'frozen'):
                    native_matrix.check_health(lane,steps)

    def test_health_interval_requires_live_complete_below_budget_samples_and_clean_log(self):
        baseline = {'latest_tick': 10, 'source_skipped_ticks_total': 0,
                    'instance_started_unix_ns': 42}
        good = {'latest_tick': 12, 'source_skipped_ticks_total': 0, 'after_tick': 10,
                'instance_started_unix_ns': 42, 'overrun': False,
                'samples': [{'tick': 11, 'wall_us': 1000}, {'tick': 12, 'wall_us': 49999}]}
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'server.log'
            log.write_text('INFO server started\n')
            host = SimpleNamespace(running=True, _process=object(), log_path=log)
            with patch.object(native_matrix.HealthInterval, '_snapshot', return_value=baseline | {'latest_tick': None}):
                with self.assertRaisesRegex(MissingCapability, 'missing_load_metrics_baseline'):
                    native_matrix.HealthInterval(host)
            with patch.object(native_matrix.HealthInterval, '_snapshot', side_effect=[baseline, good]):
                health = native_matrix.HealthInterval(host)
                health.finish()
                health.verify_log()
            self.assertEqual(health.segments[0]['samples'], good['samples'])
            for bad in (
                good | {'samples': []},
                good | {'overrun': True},
                good | {'source_skipped_ticks_total': 1},
                good | {'samples': [{'tick': 12, 'wall_us': 1}]},
                good | {'samples': [{'tick': 11, 'wall_us': 1}, {'tick': 13, 'wall_us': 1}]},
                good | {'samples': [{'tick': 11}]},
                good | {'instance_started_unix_ns': 43},
            ):
                with self.subTest(bad=bad), patch.object(native_matrix.HealthInterval, '_snapshot',
                                                         side_effect=[baseline, bad]):
                    health = native_matrix.HealthInterval(host)
                    with self.assertRaises(MissingCapability):
                        health.finish()
            slow = good | {'samples': [{'tick': 11, 'wall_us': 50000}]}
            with patch.object(native_matrix.HealthInterval, '_snapshot', side_effect=[baseline, slow]):
                health = native_matrix.HealthInterval(host)
                with self.assertRaisesRegex(RuntimeError, 'tick_mspt_at_least_50'):
                    health.finish()
            with patch.object(native_matrix.HealthInterval, '_snapshot', return_value=baseline):
                health = native_matrix.HealthInterval(host)
            host.running = False
            with self.assertRaisesRegex(RuntimeError, 'server_not_alive'):
                health.finish()
            host.running = True
            host._process = object()
            with self.assertRaisesRegex(RuntimeError, 'server_not_alive'):
                health.finish()
            for message in ('ERROR plugin failure', 'thread panicked at source'):
                log.write_text(message)
                with self.assertRaisesRegex(RuntimeError, 'server_log_error'):
                    health.verify_log()

    def test_health_checks_before_and_after_restart_and_discloses_gap(self):
        from unittest.mock import Mock, call
        lane = object.__new__(native_matrix.Lane)
        events = Mock()
        lane.health = Mock(restart_boundaries=[])
        lane.host = Mock()
        lane.host.kill9_restart.return_value = {}
        lane._record_server = Mock()
        events.attach_mock(lane.health, 'health')
        events.attach_mock(lane.host, 'host')
        lane.steps = SimpleNamespace(report={}, deadline=0)
        lane.clients, lane.names = [], []
        lane.wal_checkpoint_baseline = None
        lane.timeout = 30
        lane.restart(crash=True)
        self.assertEqual(events.mock_calls, [call.health.finish(), call.health.verify_log(),
                                            call.host.kill9_restart(), call.health.begin()])
        self.assertEqual(lane.health.restart_boundaries, [{
            'kind': 'kill9', 'unobserved_shutdown_gap': 'between final metrics read and server stop'}])
        health = object.__new__(native_matrix.HealthInterval)
        health.restart_boundaries = lane.health.restart_boundaries
        with self.assertRaisesRegex(MissingCapability, 'final_tick_to_server_stop_unobservable'):
            health.require_complete()
        health.restart_boundaries = []
        health.require_complete()
        lane.health.finish.side_effect = RuntimeError('bad health')
        lane.host.kill9_restart.reset_mock()
        with self.assertRaisesRegex(RuntimeError, 'bad health'):
            lane.restart(crash=True)
        lane.host.kill9_restart.assert_not_called()


    def test_restart_has_one_deadline_despite_unrelated_events(self):
        from types import SimpleNamespace
        lane=object.__new__(native_matrix.Lane)
        lane.host=SimpleNamespace(clean_restart=lambda:{})
        lane._record_server=lambda:None
        lane.steps=SimpleNamespace(report={})
        lane.wal_checkpoint_baseline=None
        lane.health=None
        lane.timeout=1
        client=SimpleNamespace(call_tool=lambda *args:{'in_play':True,'state_version':1})
        lane.clients=[client]; lane.names=['test']
        with patch.object(native_matrix.time,'monotonic',side_effect=[0,0.3,0.6,1.0]):
            with self.assertRaisesRegex(TimeoutError,'did not leave Play'):
                lane.restart()

    def test_native_selection_preserves_windows_as_explicit_blockers(self):
        rows = [{"id": "linux", "context": {"client_kind": "native_linux"}},
                {"id": "windows", "context": {"client_kind": "native_windows"}},
                {"id": "java", "context": {"client_kind": "vanilla_java_26_1_2"}}]
        self.assertEqual([row["id"] for row in native_matrix.select(rows)], ["linux", "windows"])

    def test_row_verdicts_distinguish_declared_blockers_and_failures(self):
        details = {'verdicts': {}, 'blocked_rows': [], 'failed_rows': []}
        declared = {'windows': 'windows_mcp_lane', 'windows_b': 'windows_mcp_lane', 'other': None}
        native_matrix.record_outcome(details, {'id': 'ok', 'status': 'pass', 'note': ''}, declared)
        native_matrix.record_outcome(details, {'id': 'windows', 'status': 'blocked',
                                                'note': 'windows_mcp_lane'}, declared)
        native_matrix.record_outcome(details, {'id': 'other', 'status': 'blocked',
                                                'note': 'client startup failed'}, declared)
        native_matrix.record_outcome(details, {'id': 'bad', 'status': 'fail',
                                                'note': 'assertion'}, declared)
        native_matrix.record_outcome(details, {'id': 'windows_b', 'status': 'fail',
                                                'note': 'windows_mcp_lane'}, declared)
        self.assertEqual(details['verdicts'], {'26.3.ok': True, '26.3.windows': False,
                         '26.3.other': False, '26.3.bad': False, '26.3.windows_b': False})
        self.assertEqual(details['blocked_rows'], [{'id': 'windows', 'reason': 'windows_mcp_lane'}])
        self.assertEqual(details['failed_rows'], [{'id': 'other', 'reason': 'client startup failed'},
                         {'id': 'bad', 'reason': 'assertion'},
                         {'id': 'windows_b', 'reason': 'windows_mcp_lane'}])

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

    def test_item_birth_position_is_first_received_and_mobs_get_none(self):
        from tools.harness.backends.native_matrix_client import NativeMatrixClient
        rows=[{'entity_id':1,'entity_uuid':'item','entity_type':'minecraft:item','x':1.,'y':2.,'z':3.},
              {'entity_id':2,'entity_uuid':'cow','entity_type':'minecraft:cow','x':4.,'y':5.,'z':6.}]
        class Mcp:
            def call_tool(self, tool, arguments):
                if tool == 'minecraft_list_entities':
                    return {'entities':[dict(row) for row in rows],'truncated':False}
                return {'in_play':True,'player':{}}
        client=NativeMatrixClient('test',Mcp(),'127.0.0.1:1',[])
        first=client.snapshot()['entities']
        rows[0]['y']=0.; rows[1]['y']=0.
        later=client.snapshot()['entities']
        self.assertEqual(first[0]['spawn_position'],[1.,2.,3.])
        self.assertEqual(later[0]['spawn_position'],[1.,2.,3.])
        self.assertEqual(later[0]['position'],[1.,0.,3.])
        self.assertNotIn('spawn_position',later[1])

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

    def test_native_launch_context_has_every_attribute_start_reads(self):
        from types import SimpleNamespace
        from unittest.mock import MagicMock, patch
        lane=object.__new__(native_matrix.Lane)
        lane.directory=Path(tempfile.mkdtemp())
        lane.processes=[]; lane.handles=[]; lane.contexts=[]
        lane.native=Path('/native'); lane.address='127.0.0.1:1'; lane.timeout=5; lane.display=':99'
        lane.ctx=SimpleNamespace(client_evidence={},commands=[],cwds=[],exits=[],logs=[])
        client=MagicMock()
        seen={}
        def fake_start(ctx,*_,**__):
            # native_client.start reads ctx.scenario_id to pick the window size.
            seen['scenario_id']=ctx.scenario_id
            return MagicMock(),MagicMock(),client
        with patch('tools.harness.audio.private_audio_preflight'), \
             patch.object(native_matrix.native_client,'start',side_effect=fake_start):
            lane.launch('test')
        self.assertIsNone(seen['scenario_id'])

    def test_native_missing_content_is_blocked_not_product_failure(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        from tools.harness import runtime
        from tools.harness.matrix_api import MissingCapability
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / 'client.log'
            log.write_text('krasnokamen-client: missing content file: /private/facts/block_collision_shapes_26_3.bin\n')
            lane = object.__new__(native_matrix.Lane)
            lane.directory, lane.processes = root, []
            lane.ctx = SimpleNamespace(client_evidence={}, commands=[], cwds=[], exits=[], logs=[])
            lane.scenario = {'id': 'matrix:chest'}
            lane.native, lane.address, lane.timeout, lane.display = Path('/client'), 'localhost:1', 30, ':1'
            with patch('tools.harness.audio.private_audio_preflight'), \
                 patch.object(native_matrix.native_client, 'start',
                              side_effect=runtime.ChildExited('startup failed', 1, log)):
                with self.assertRaisesRegex(MissingCapability, 'client:missing_content'):
                    lane.launch('test')

    def test_native_other_startup_crash_remains_failure(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        from tools.harness import runtime
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / 'client.log'
            log.write_text('client panic\n')
            lane = object.__new__(native_matrix.Lane)
            lane.directory, lane.processes = root, []
            lane.ctx = SimpleNamespace(client_evidence={}, commands=[], cwds=[], exits=[], logs=[])
            lane.scenario = {'id': 'matrix:chest'}
            lane.native, lane.address, lane.timeout, lane.display = Path('/client'), 'localhost:1', 30, ':1'
            with patch('tools.harness.audio.private_audio_preflight'), \
                 patch.object(native_matrix.native_client, 'start',
                              side_effect=runtime.ChildExited('startup failed', 1, log)):
                with self.assertRaises(runtime.ChildExited):
                    lane.launch('test')

    def _loaded_lane(self, home):
        from unittest.mock import MagicMock
        lane=object.__new__(native_matrix.Lane)
        lane.timeout=300; lane.chunk_fences=[]; lane.names=['a']
        lane.scenario={'context':{'dimension':'overworld','game_mode':'survival'}}
        lane.commands=[]
        lane.console=lane.commands.append
        client=MagicMock()
        client.call_tool.side_effect=lambda tool,args=None:(
            {'player':home} if tool=='minecraft_observe' else {'loaded':True})
        lane.clients=[client]
        return lane,client

    def test_loaded_block_wait_stays_inside_declared_tool_range(self):
        # minecraft_wait_for_loaded_block accepts at most 120 s; a 300 s run
        # timeout must not be forwarded as the per-call bound.
        lane,client=self._loaded_lane({'x':1.5,'y':100.,'z':1.5})
        lane.ensure_loaded(((0,60,0),(0,70,0)))
        waits=[call.args[1] for call in client.call_tool.call_args_list
               if call.args[0]=='minecraft_wait_for_loaded_block']
        self.assertEqual([wait['timeout_seconds'] for wait in waits],[120])
        self.assertEqual(lane.commands,[])

    def test_fixture_area_outside_view_distance_is_visited_and_player_returned(self):
        # A native client only receives chunks inside its view distance, so a far
        # fixture area is visited as a spectator and the exact position and game
        # mode are restored.
        lane,client=self._loaded_lane({'x':192.5,'y':100.,'z':-447.5})
        lane.ensure_loaded(((-16,199,-16),(16,215,16)))
        self.assertEqual(lane.commands,[
            'gamemode spectator a',
            'execute in minecraft:overworld run tp a 0.5 207.0 0.5',
            'execute in minecraft:overworld run tp a 192.5 100.0 -447.5',
            'gamemode survival a'])
        loaded=[(call.args[1]['x'],call.args[1]['z']) for call in client.call_tool.call_args_list
                if call.args[0]=='minecraft_wait_for_loaded_block']
        self.assertEqual(sorted(loaded),sorted((x*16,z*16) for x in (-1,0,1) for z in (-1,0,1)))

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
