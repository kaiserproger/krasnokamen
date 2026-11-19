"""Fail-closed adapter/context contracts; real release probes are separate."""
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

from .matrix_api import MatrixContext, MissingCapability, load_adapter


def context(network='normal', lifecycle='steady'):
    factors={'game_mode':'survival','dimension':'overworld','difficulty':'normal',
             'player_count':'1','permission':'non_op_0','network_condition':network,
             'server_lifecycle':lifecycle}
    return MatrixContext({'subject':'block_place.full_block','context':factors},Mock(),
                         [SimpleNamespace(name='Proto0')],Path('/unused'))


class MatrixApiTests(unittest.TestCase):
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
