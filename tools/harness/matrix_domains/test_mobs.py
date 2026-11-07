"""Focused pure-oracle regressions; no fake server/client receipts.

Run with the pinned26.1.2 data installed by canonical content import.
"""
import json
import os
from pathlib import Path
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from ..matrix_api import MissingCapability
from ._mob_contracts import CONTRACTS
from ._mob_loot import _condition, bounds
from . import mobs, spawning, villagers, settlements

VANILLA=Path(os.environ.get('SOLARIS_VANILLA_DATA',
    '/run/media/kaiserroman/games/solaris-work/solaris/data/vanilla'))


class MobOracleTests(unittest.TestCase):
    def test_exact_version_and_all_species_have_real_loot_sources(self):
        self.assertEqual(json.loads((VANILLA/'gameplay_version.json').read_text())['id'],'26.1.2')
        self.assertEqual(len(CONTRACTS),91)
        self.assertEqual(set(mobs.ADAPTERS),{'mobs.'+name for name in CONTRACTS})
        for name in CONTRACTS:
            self.assertTrue((VANILLA/'data/minecraft/loot_table/entities'/(name+'.json')).is_file(),name)

    def test_all_pinned_loot_conditions_are_understood_for_authored_fixture(self):
        for name in CONTRACTS:
            for killed_by_player in (False,True):
                with self.subTest(name=name,player_kill=killed_by_player):
                    limits=bounds(VANILLA,'entities/'+name,killed_by_player)
                    self.assertTrue(all(0<=low<=high for low,high in limits.values()))

    def test_cow_exact_unenchanted_bounds_from_data(self):
        self.assertEqual(bounds(VANILLA,'entities/cow',True),
                         {'minecraft:leather':(0,2),'minecraft:beef':(1,3)})

    def test_player_kill_condition_is_not_silently_assumed(self):
        condition={'condition':'minecraft:killed_by_player'}
        self.assertEqual(_condition(condition,False),(False,False))
        self.assertEqual(_condition(condition,True),(True,True))
        self.assertEqual(bounds(VANILLA,'entities/blaze',False),{})
        self.assertIn('minecraft:blaze_rod',bounds(VANILLA,'entities/blaze',True))

    def test_unknown_predicate_fails_closed(self):
        with self.assertRaises(MissingCapability):
            _condition({'condition':'minecraft:unrecognized_future_condition'},True)

    def test_giant_does_not_inherit_monster_pursuit(self):
        # Exact26.1.2 Giant.javap has attributes but no registerGoals override.
        self.assertEqual(CONTRACTS['giant'],('Immobile','None'))

    def test_domains_have_only_shared_adapter_objects(self):
        from ..matrix_api import Adapter
        for module in (mobs,spawning,villagers,settlements):
            self.assertTrue(module.ADAPTERS)
            for subject,adapter in module.ADAPTERS.items():
                self.assertIsInstance(adapter,Adapter,subject)
                self.assertTrue(callable(adapter.setup) and callable(adapter.act) and callable(adapter.oracle))

    def test_lightning_oracle_requires_named_type_change_not_uuid_deletion(self):
        ctx=SimpleNamespace(factors={'difficulty':'normal'},state={'witches_before':0,'observer_witches_before':[]},clients=[object()])
        def present(_ctx,selector):
            return selector=='@e[type=witch,name=MatrixLightning]'
        with patch.object(villagers,'exists',side_effect=present), patch.object(villagers,'count',return_value=1):
            self.assertTrue(all(check.passed for check in villagers.lightning_oracle(ctx)))
        with patch.object(villagers,'exists',return_value=False), patch.object(villagers,'count',return_value=1):
            self.assertFalse(all(check.passed for check in villagers.lightning_oracle(ctx)))

    def test_lightning_peaceful_preserves_villager_without_witch(self):
        ctx=SimpleNamespace(factors={'difficulty':'peaceful'},state={'witches_before':0,'observer_witches_before':[]},clients=[object()])
        def present(_ctx,selector):
            return selector=='@e[type=villager,name=MatrixLightning]'
        with patch.object(villagers,'exists',side_effect=present), patch.object(villagers,'count',return_value=0):
            self.assertTrue(all(check.passed for check in villagers.lightning_oracle(ctx)))

    def test_console_fences_do_not_change_settlement_state(self):
        reply='Settlements: MatrixTown.\nchat.type.announcement: Server, MatrixConsoleFence00000015\n'
        ctx=SimpleNamespace(server=SimpleNamespace(query=lambda _:reply))
        self.assertEqual(settlements.query(ctx,'settlement list'),'Settlements: MatrixTown.')

    def test_data_queries_accept_transport_fence_after_value(self):
        from ._mob_support import number,string
        reply='Health on Cow after scale factor of 1000.00 is 10000\nchat.type.announcement: Server, MatrixConsoleFence00000016\n'
        ctx=SimpleNamespace(server=SimpleNamespace(query=lambda _:reply))
        self.assertEqual(number(ctx,'cow','Health'),10)
        reply='Cow has the following entity data: "minecraft:cow"\nchat.type.announcement: Server, MatrixConsoleFence00000017\n'
        self.assertEqual(string(ctx,'cow','id'),'minecraft:cow')

    def test_mob_observation_state_is_json_serializable(self):
        snapshot={'entities':[{'type':'minecraft:cow','uuid':mobs.UUID,'position':[1,200,2]}],'health':20}
        ctx=SimpleNamespace(clients=[SimpleNamespace(snapshot=lambda:snapshot)],
                            state={'name':'cow','seen_types':[],'samples':[],'minimum_health':20,'checks':[]})
        mobs.sample(ctx)
        mobs.sample(ctx)
        self.assertEqual(ctx.state['seen_types'],['minecraft:cow'])
        json.dumps(ctx.state,allow_nan=False)

    def test_mob_health_uses_received_metadata_without_data_command(self):
        row={'uuid':mobs.UUID,'id':1,'type':'minecraft:cow','position':[1,200,2],'health':10.0}
        ctx=SimpleNamespace(clients=[SimpleNamespace(snapshot=lambda:{'entities':[row]})])
        self.assertEqual(mobs.observed_health(ctx,mobs.UUID),10.0)
        del row['health']
        with self.assertRaisesRegex(MissingCapability,'snapshot:entities.health'):
            mobs.observed_health(ctx,mobs.UUID)

    def test_settlement_oracle_rejects_restart_state_loss(self):
        state={'action_reply':'Settlements: MatrixTown.','pattern':'MatrixTown','read':'settlement list',
               'after_action':'MatrixTown','after_boundary':'MatrixTown','after_restart':'empty',
               'observer_replies':[],'observer_restart_replies':[]}
        ctx=SimpleNamespace(state=state,row={'subject':'settlements.list'},clients=[object()])
        with patch.object(settlements,'query',return_value='MatrixTown'):
            checks=settlements.oracle(ctx)
        self.assertFalse(next(c.passed for c in checks if c.name=='explicit_restart_state_preserved'))

    def test_settlement_observer_cannot_be_replaced_by_server_query(self):
        state={'action_reply':'MatrixTown','pattern':'MatrixTown','read':'settlement list',
               'after_action':'MatrixTown','after_boundary':'MatrixTown','after_restart':'MatrixTown',
               'observer_replies':[],'observer_restart_replies':[]}
        ctx=SimpleNamespace(state=state,row={'subject':'settlements.list'},clients=[object(),object()])
        with patch.object(settlements,'query',return_value='MatrixTown'):
            checks=settlements.oracle(ctx)
            self.assertFalse(next(c.passed for c in checks if c.name=='observers_receive_authoritative_result'))
            state['observer_replies']=['MatrixTown'];state['observer_restart_replies']=['MatrixTown']
            self.assertTrue(all(c.passed for c in settlements.oracle(ctx)))

    def test_spawner_oracle_checks_birth_range_and_delay(self):
        state={'empty_before':None,'configured':'minecraft:zombie','spawned':1,'inactive_count':0,
               'paused_delay':250,'inactive_delay':250,'spawn_positions':[[4.5,200,0.5]],
               'reset_delay':199,'delay_query_ticks':1,'spawner_observer_receipts':[]}
        ctx=SimpleNamespace(state=state,factors={'game_mode':'survival','difficulty':'normal'},clients=[object()])
        self.assertTrue(all(c.passed for c in spawning.spawner_oracle(ctx)))
        state['spawn_positions']=[[9,200,0.5]]
        self.assertFalse(next(c.passed for c in spawning.spawner_oracle(ctx)
                              if c.name=='spawn_inside_four_block_horizontal_range'))
        state['spawn_positions']=[[4.5,200,0.5]]
        state['reset_delay']=900
        self.assertFalse(next(c.passed for c in spawning.spawner_oracle(ctx)
                              if c.name=='reset_delay_200_to_800_ticks'))

    def test_settlement_family_cannot_pass_from_empty_status(self):
        ctx=SimpleNamespace(row={'subject':'settlements.production'},server=SimpleNamespace(command=lambda _:None))
        with patch.object(settlements,'arena'):
            with self.assertRaisesRegex(MissingCapability,'settlement_economy_snapshot'):
                settlements.setup(ctx)

    def test_mob_loot_excludes_remote_births_not_unexpected_nearby_items(self):
        rows=[
            {'type':'minecraft:item','spawn_position':[6,200,7],'position':[6,60,7],
             'item':{'item_id':'minecraft:beef','count':3}},
            {'type':'minecraft:item','spawn_position':[6,60,7],
             'item':{'item_id':'minecraft:kelp','count':39}},
            {'type':'minecraft:item','spawn_position':[7,200,7],
             'item':{'item_id':'minecraft:kelp','count':1}},
        ]
        ctx=SimpleNamespace(clients=[SimpleNamespace(snapshot=lambda:{'entities':rows})],
                            state={'inventory_before':{},'death_position':[6,200,7]})
        with patch.object(mobs,'inventory',return_value={}):
            self.assertEqual(mobs.drop_total(ctx),{'minecraft:beef':3,'minecraft:kelp':1})
            rows[1]['item']['count']=46
            self.assertEqual(mobs.drop_total(ctx),{'minecraft:beef':3,'minecraft:kelp':1})

    def test_mob_loot_requires_received_birth_position(self):
        rows=[{'type':'minecraft:item','item':{'item_id':'minecraft:beef','count':3}}]
        ctx=SimpleNamespace(clients=[SimpleNamespace(snapshot=lambda:{'entities':rows})],
                            state={'inventory_before':{},'death_position':[6,200,7]})
        with patch.object(mobs,'inventory',return_value={}):
            with self.assertRaisesRegex(MissingCapability,'snapshot:entities.spawn_position'):
                mobs.drop_total(ctx)


if __name__=='__main__':
    unittest.main()
