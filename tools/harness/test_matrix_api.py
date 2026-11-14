"""Fail-closed adapter/context contracts; real release probes are separate."""
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

from .matrix_api import Adapter, Check, MatrixContext, MissingCapability, execute_adapter, load_adapter
from .matrix_execution import apply_factors, offline_uuid, prepare_access


def context(network='normal', lifecycle='steady'):
    factors={'game_mode':'survival','dimension':'overworld','difficulty':'normal',
             'player_count':'1','permission':'non_op_0','network_condition':network,
             'server_lifecycle':lifecycle}
    return MatrixContext({'subject':'block_place.full_block','context':factors},Mock(),
                         [SimpleNamespace(name='Proto0')],Path('/unused'))


class MatrixApiTests(unittest.TestCase):
    def test_empty_or_malformed_oracle_fails_closed(self):
        for result in ([],None,[True],[Check('truthy',1)]):
            with self.subTest(result=result), self.assertRaises((RuntimeError,TypeError)):
                execute_adapter(Adapter(lambda c:None,lambda c:None,lambda c:result),context())

    def test_nonsteady_adapter_without_semantic_boundary_is_blocked(self):
        with self.assertRaisesRegex(MissingCapability,'adapter_action_boundary'):
            execute_adapter(Adapter(lambda c:None,lambda c:None,lambda c:[Check('ok',True)]),
                            context(lifecycle='clean_restart_mid_action'))

    def test_boundary_requires_real_hook_and_is_single_use(self):
        ctx=context(network='reconnect_mid_action')
        with self.assertRaisesRegex(MissingCapability,'action_boundary'):
            ctx.boundary()
        ctx=context(); ctx.boundary()
        with self.assertRaisesRegex(RuntimeError,'more than once'):
            ctx.boundary()

    def test_boundary_hook_keeps_exact_context(self):
        ctx=context(lifecycle='crash_kill9_wal_replay'); seen=[]
        ctx.boundary_hook=seen.append
        execute_adapter(Adapter(lambda c:None,lambda c:c.boundary(),lambda c:[Check('ok',True)]),ctx)
        self.assertEqual(seen,[ctx])

    def test_access_fixture_has_exact_levels_and_offline_identity(self):
        with tempfile.TemporaryDirectory() as temp:
            prepare_access(Path(temp),['Proto0'],'op_2')
            entry=json.loads((Path(temp)/'ops.json').read_text())[0]
            self.assertEqual(entry['level'],2)
            self.assertEqual(entry['uuid'],str(offline_uuid('Proto0')))
            prepare_access(Path(temp),['Proto0'],'non_op_0')
            self.assertEqual(json.loads((Path(temp)/'ops.json').read_text()),[])

    def test_factors_reject_wrong_client_count_not_silently_scale_down(self):
        ctx=context(); ctx.row['context']['player_count']='N8'
        with self.assertRaisesRegex(RuntimeError,'requires 8'):
            apply_factors(ctx)
        ctx.server.command.assert_not_called()

    def test_unknown_domain_does_not_import_arbitrary_paths(self):
        with self.assertRaises(ValueError):
            load_adapter('../os.subject')
        with self.assertRaisesRegex(MissingCapability,'domain_adapter'):
            load_adapter('missing_domain.no_subject')

    def test_sixface_fixture_teleports_before_world_commands(self):
        from .matrix_domains import block_place
        events=[]
        class Client:
            name='Proto0'
            def teleport_to(self,*position): events.append(('teleport',position))
            def snapshot(self): return {'inventory':[{'slot':36,'item_id':'minecraft:stone','count':16}]}
            def select_hotbar(self,slot): events.append(('select',slot))
            def wait_until(self,predicate,**kwargs): return predicate()
        ctx=context(); ctx.clients=[Client()]
        ctx.server.command.side_effect=lambda command:events.append(('command',command)) or ''
        ctx.server.query.return_value='MatrixPlacementFixtureReady'
        block_place.setup(ctx)
        self.assertEqual(events[0],('teleport',(4.5,80,8.5)))
        self.assertEqual(len(block_place.FACES),6)
        self.assertEqual(len(set(position for _,position in block_place.FACES)),6)


if __name__=='__main__':
    unittest.main()
