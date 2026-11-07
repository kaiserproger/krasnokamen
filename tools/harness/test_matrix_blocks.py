"""Adapter-contract unit tests, not substitutes for live vanilla gameplay proof."""
import importlib
import json
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from .matrix_api import MatrixContext, MissingCapability, load_adapter
from .matrix_domains import _blocks as b

DOMAINS = {'block_interact':29, 'block_break':24, 'block_place':24,
           'special_blocks':8, 'falling_blocks':6, 'fire':5, 'explosions':11}


class BlocksContractTests(unittest.TestCase):
    def context(self, reply='', snapshot=None):
        server=SimpleNamespace(query=lambda text:reply, command=lambda text:reply)
        client=SimpleNamespace(name='MatrixA',snapshot=lambda:snapshot or {})
        row={'subject':'block_interact.note_block_tune','context':{
            'dimension':'overworld','game_mode':'survival','network_condition':'normal',
            'server_lifecycle':'steady','permission':'non_op_0','player_count':'1'}}
        return MatrixContext(row,server,[client],Path('/run/media/kaiserroman/games/solaris-work/scenarios/runs/DomBlocks'))

    def test_all_subjects_export_real_adapters(self):
        total=0
        for domain,expected in DOMAINS.items():
            adapters=importlib.import_module('tools.harness.matrix_domains.'+domain).ADAPTERS
            self.assertEqual(len(adapters),expected,domain)
            for subject,adapter in adapters.items():
                self.assertIs(load_adapter(subject),adapter)
                self.assertTrue(callable(adapter.setup))
                self.assertTrue(callable(adapter.act))
                self.assertTrue(callable(adapter.oracle))
            total+=len(adapters)
        self.assertEqual(total,107)

    def test_missing_observation_never_becomes_empty_inventory(self):
        with self.assertRaisesRegex(MissingCapability,'snapshot:inventory'):
            b.count(self.context(),'diamond')

    def test_sparse_inventory_counts_stack_quantities(self):
        ctx=self.context(snapshot={'inventory':[{'slot':36,'item_id':'minecraft:diamond','count':7},
                                               {'slot':9,'item_id':'minecraft:diamond','count':3},
                                               {'slot':10,'item_id':'minecraft:stone','count':64}]})
        self.assertEqual(b.count(ctx,'diamond'),10)

    def test_data_missing_commands_are_blocked(self):
        for reply in ('Unknown or incomplete command',
                      'Supported: data get|merge block <x> <y> <z> [compound] (command blocks only)',
                      'Data editing currently supports command blocks only'):
            with self.subTest(reply=reply):
                with self.assertRaisesRegex(MissingCapability,'data_get_entity'):
                    b.scalar(self.context(reply),'entity MatrixA','Health')
                with self.assertRaisesRegex(MissingCapability,'data_get_block'):
                    b.data(self.context(reply),'block 1 80 0')

    def test_data_path_failure_is_not_zero(self):
        with self.assertRaises(RuntimeError):
            b.scalar(self.context('Found no elements matching Inventory[0]'),'entity MatrixA','Inventory[0].count')

    def test_scaled_number_including_negative(self):
        reply='Health on MatrixA after scale factor of 1.00 is -2'
        self.assertEqual(b.scalar(self.context(reply),'entity MatrixA','Health'),-2)

    def test_root_snbt_not_command_echo(self):
        reply='1, 80, 0 has the following block data: {item:{id:"minecraft:diamond",count:1}}'
        self.assertEqual(b.data(self.context(reply),'block 1 80 0'),'{item:{id:"minecraft:diamond",count:1}}')
        with self.assertRaises(RuntimeError):
            b.data(self.context('data get block 1 80 0'),'block 1 80 0')

    def test_boundary_runs_once_across_multiple_action_helpers(self):
        ctx=self.context(); calls=[]
        ctx.boundary_hook=lambda context:calls.append(context.row['subject'])
        b.boundary(ctx); b.boundary(ctx)
        self.assertEqual(calls,[ctx.row['subject']])

    def test_unknown_mode_is_blocked_not_silently_changed(self):
        ctx=self.context(); ctx.row['context']['game_mode']='spectator'
        with self.assertRaisesRegex(MissingCapability,'context_oracle:'):
            b.require_modes(ctx,'survival')
        self.assertEqual(ctx.factors['game_mode'],'spectator')

    def test_multiplayer_not_credited_as_single_actor(self):
        ctx=self.context(); ctx.clients.append(ctx.clients[0])
        ctx.row['origin']='triple:multiplayer_x_same_block'
        with self.assertRaisesRegex(MissingCapability,'scenario_concurrent_actor_schedule:'):
            b.fixture(ctx)

    def test_fixture_loads_chunks_before_any_fill(self):
        ctx=self.context(); commands=[]
        ctx.server.command=lambda text: commands.append(text) or ''
        ctx.server.query=lambda text:text.split('run say ')[-1]
        ctx.clients[0].teleport_to=lambda *pos:commands.append(('tp',pos))
        ctx.clients[0].close_container=lambda:commands.append(('close',))
        def loaded(area):
            self.assertFalse(any(isinstance(text,str) and 'run fill ' in text for text in commands))
            self.assertGreater(area[0][1],100)
            commands.append(('loaded',area))
        ctx.server.ensure_loaded=loaded
        b.fixture(ctx)
        self.assertEqual(commands[0],('tp',(.5,160,2.5)))
        self.assertEqual(commands[1][0],'loaded')
        self.assertTrue(any(isinstance(text,str) and 'run fill ' in text for text in commands))

    def test_checks_remain_json_serializable(self):
        ctx=self.context(); ctx.state['block_checks']=[]
        b.check(ctx,'block_changed',True,{'position':b.POS})
        json.dumps(ctx.state,allow_nan=False)
        self.assertTrue(b.oracle(ctx)[0].passed)

    def test_use_waits_exact_received_ack_before_boundary(self):
        snap={'block_ack_sequence':None, 'revision':1}
        ctx=self.context(snapshot=snap); calls=[]
        client=ctx.clients[0]
        client.look_at=lambda *pos:None
        client.use_item_on=lambda *args,**kwargs:calls.append('submitted') or 9
        ctx.boundary_hook=lambda context:calls.append('boundary')
        def wait(predicate,**kwargs):
            self.assertEqual(calls,['submitted'])
            self.assertFalse(predicate())
            snap['revision']+=1; snap['block_ack_sequence']=8
            self.assertFalse(predicate())
            snap['block_ack_sequence']=9
            self.assertTrue(predicate())
        client.wait_until=wait
        b.use(ctx)
        self.assertEqual(calls,['submitted','boundary'])

    def test_data_parser_ignores_console_fence_suffix(self):
        reply='Health on MatrixA after scale factor of 1.00 is 20\nchat.type.announcement: Server, MatrixConsoleFence1\n'
        self.assertEqual(b.scalar(self.context(reply),'entity MatrixA','Health'),20)
        reply='MatrixA has the following entity data: {Health:20.0f}\nchat.type.announcement: Server, MatrixConsoleFence2\n'
        self.assertEqual(b.data(self.context(reply),'entity MatrixA'),'{Health:20.0f}')

    def test_setblock_waits_received_fixture_not_console_only(self):
        ctx=self.context(); state={'block_id':'minecraft:air','properties':{}}
        ctx.server.query=lambda text:text.split('run say ')[-1]
        ctx.clients[0].read_block_state=lambda pos:state
        def wait(predicate,**kwargs):
            self.assertFalse(predicate())
            state.update(block_id='minecraft:oak_log',properties={'axis':'z'})
            self.assertFalse(predicate())
            state['properties']['axis']='x'
            self.assertTrue(predicate())
        ctx.clients[0].wait_until=wait
        b.setblock(ctx,'oak_log[axis=x]')

    def test_missing_optional_verb_or_argument_is_blocked(self):
        client=SimpleNamespace(use_item_on=lambda pos,face='up':None)
        with self.assertRaisesRegex(MissingCapability,'client:break_block'):
            b.call(client,'break_block',(0,0,0))
        with self.assertRaisesRegex(MissingCapability,'client:use_item_on:cursor'):
            b.call(client,'use_item_on',(0,0,0),cursor=(.5,.25,.5))

    def test_empty_check_list_is_not_pass(self):
        ctx=self.context(); ctx.state['block_checks']=[]
        with self.assertRaises(RuntimeError): b.oracle(ctx)

    def test_spectator_interactions_check_unchanged_states(self):
        from .matrix_domains import block_interact as module
        ctx=self.context(); ctx.row['context']['game_mode']='spectator'
        states=[]
        def observe(context,pos,state,*args):
            states.append(state)
        noop=lambda *args,**kwargs:None
        with patch.multiple(module,equip=noop,setblock=noop,use=noop,ticks=noop,block=observe):
            module.door_trapdoor_gate_toggle(ctx)
            self.assertTrue(states and all('open=false' in value for value in states))
            states.clear(); module.note_block_tune(ctx)
            self.assertEqual(len(states),125)
            self.assertTrue(all('note=0,' in value for value in states))
            states.clear(); module.repeater_comparator_daylight(ctx)
            self.assertEqual(set(states),{'repeater[delay=1]','comparator[mode=compare]',
                                         'daylight_detector[inverted=false]'})
            states.clear(); module.lever_button(ctx)
            self.assertTrue(states and all('powered=false' in value for value in states))

    def test_slab_checks_each_item_debit(self):
        from .matrix_domains import block_place_extra as module
        ctx=self.context(); ctx.state['block_checks']=[]
        counts=iter((2,1,2,1)); noop=lambda *args,**kwargs:None
        with patch.multiple(module,equip=noop,setblock=noop,use=noop,block=noop,
                            count=lambda *args:next(counts)):
            module.slab_double(ctx)
        checks=b.oracle(ctx)
        self.assertEqual(len(checks),4)
        self.assertTrue(all(check.passed for check in checks))

    def test_adventure_and_spectator_refuse_tools_and_repeater_edits(self):
        from .matrix_domains import block_interact as module
        ctx=self.context(); ctx.state['block_checks']=[]
        states=[]; noop=lambda *args,**kwargs:None
        for mode in ('adventure','spectator'):
            ctx.row['context']['game_mode']=mode
            with patch.multiple(module,equip=noop,setblock=noop,use=noop,
                                block=lambda context,pos,state:states.append(state),
                                drops=lambda *args:0,count=lambda *args:0):
                states.clear(); module.axe_strip_scrape_unwax(ctx)
                self.assertEqual(states,['oak_log[axis=x]','oxidized_copper','waxed_weathered_copper'])
                states.clear(); module.shovel_path(ctx)
                self.assertEqual(states,['grass_block','dirt','campfire[lit=true]'])
                states.clear(); module.hoe_till(ctx)
                self.assertEqual(states,['dirt','grass_block','coarse_dirt','rooted_dirt'])
                states.clear(); module.repeater_comparator_daylight(ctx)
                self.assertEqual(set(states),{'repeater[delay=1]','comparator[mode=compare]',
                                             'daylight_detector[inverted=false]'})
        self.assertTrue(all(check.passed for check in b.oracle(ctx)))

    def test_observer_must_receive_matching_block_state(self):
        ctx=self.context(); ctx.state['block_checks']=[]
        state={'block_id':'minecraft:repeater','properties':{'delay':'1'}}
        observer=SimpleNamespace(name='Watcher',read_block_state=lambda pos:state)
        def wait(predicate,**kwargs):
            self.assertFalse(predicate())
            state['properties']['delay']='2'
            self.assertTrue(predicate())
        observer.wait_until=wait
        ctx.clients.append(observer)
        ctx.server.query=lambda text:text.split('run say ')[-1]
        b.block(ctx,b.POS,'repeater[delay=2]')
        self.assertEqual(len(b.oracle(ctx)),2)
        self.assertTrue(b.oracle(ctx)[1].name.startswith('observer:Watcher:'))

    def test_not_received_yet_waits_instead_of_fabricating_block(self):
        ready=[False]
        def read(pos):
            if not ready[0]:
                raise MissingCapability('snapshot:block_state_not_received:'+str(pos))
            return {'block_id':'minecraft:air','properties':{}}
        def wait(predicate,**kwargs):
            self.assertFalse(predicate()); ready[0]=True; self.assertTrue(predicate())
        b.wait_block(SimpleNamespace(read_block_state=read,wait_until=wait),b.POS,'air','fixture')

    def test_received_xp_orb_health_and_effect_fields(self):
        ctx=self.context(snapshot={
            'xp':{'total':17,'level':1,'progress':.5},
            'entities':[{'type':'minecraft:experience_orb','xp_value':3},
                        {'type':'minecraft:iron_golem','health':87.0}],
            'registries':{'minecraft:mob_effect':{'1':'minecraft:speed'}},
            'effects':{'1':{'amplifier':0,'duration_ticks':200}}})
        self.assertEqual(b.experience(ctx),17)
        self.assertEqual(b.orb_experience(ctx),3)
        self.assertEqual(b.entity_health(ctx,'iron_golem'),87.0)
        self.assertEqual(b.effects(ctx)['minecraft:speed']['duration_ticks'],200)

    def test_missing_entity_metadata_never_becomes_zero(self):
        ctx=self.context(snapshot={'entities':[{'type':'minecraft:experience_orb'}]})
        with self.assertRaisesRegex(MissingCapability,'snapshot:entities:xp_value'):
            b.orb_experience(ctx)
        ctx=self.context(snapshot={'entities':[{'type':'minecraft:iron_golem'}]})
        with self.assertRaisesRegex(MissingCapability,'snapshot:entities:health'):
            b.entity_health(ctx,'iron_golem')
        with self.assertRaisesRegex(MissingCapability,'snapshot:xp'):
            b.experience(ctx)


if __name__=='__main__':
    unittest.main()
