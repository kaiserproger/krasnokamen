"""Adapter-contract unit tests, not substitutes for live vanilla gameplay proof."""
import importlib
import pkgutil
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from .matrix_api import MatrixContext, MissingCapability
from .matrix_domains import _blocks as b


class BlocksContractTests(unittest.TestCase):
    def test_tnt_adapter_blocks_received_lifecycle_without_tick_evidence(self):
        import threading
        from .matrix_domains import explosions
        from .protocol_entities import ProtocolEntities
        from .protocol_matrix_client import ProtocolMatrixClient
        from .test_protocol_entities import add_body,varint
        tracker=ProtocolEntities()
        client=object.__new__(ProtocolMatrixClient)
        client.entity_types={25:'minecraft:tnt'}; client.items={}
        client.observer=SimpleNamespace(condition=threading.RLock(),position=None,events=[],health=None,
            food=None,revision=1,chunks=set(),chat_components=[],chat_errors=[],menu_errors={},
            merchant_errors={},menus={},current_window=0,entities=tracker)
        def fire(_ctx):
            tracker.consume(0x01,add_body(entity_id=42,entity_type=25))
            tracker.consume(0x4d,varint(1)+varint(42))
        ctx=SimpleNamespace(clients=[client])
        with patch.object(explosions,'require_modes'),patch.object(explosions,'setblock'), \
             patch.object(explosions,'equip'),patch.object(explosions,'use',side_effect=fire), \
             patch.object(explosions,'block'),patch.object(explosions,'ticks'), \
             patch.object(explosions,'entities',return_value=[{'id':42}]), \
             patch.object(explosions,'check'):
            with self.assertRaisesRegex(MissingCapability,'authoritative_tick'):
                explosions.tnt_prime_chain(ctx)

    def context(self, reply='', snapshot=None):
        server=SimpleNamespace(query=lambda text:reply, command=lambda text:reply)
        client=SimpleNamespace(name='MatrixA',snapshot=lambda:snapshot or {})
        row={'subject':'block_interact.note_block_tune','context':{
            'dimension':'overworld','game_mode':'survival','network_condition':'normal',
            'server_lifecycle':'steady','permission':'non_op_0','player_count':'1'}}
        return MatrixContext(row,server,[client],Path('/run/media/kaiserroman/games/solaris-work/scenarios/runs/DomBlocks'))

    def test_all_matrix_domains_import_and_health_requires_received_value(self):
        from . import matrix_domains
        missing_health = False
        for module in pkgutil.iter_modules(matrix_domains.__path__):
            if not module.name.startswith('test_'):
                with self.subTest(module=module.name):
                    try:
                        importlib.import_module(matrix_domains.__name__ + '.' + module.name)
                    except ImportError as error:
                        if (error.name != matrix_domains.__name__ + '._blocks'
                                or not error.msg.startswith("cannot import name 'entity_health' from ")):
                            raise
                        missing_health = True
                        self.fail(f'{module.name}: consumer import contract broken: {error}')
        if missing_health:
            return
        ctx=self.context(snapshot={'entities':[{'type':'minecraft:iron_golem','health':42.0}]})
        self.assertEqual(b.entity_health(ctx,'iron_golem'),42.0)
        ctx=self.context(snapshot={'entities':[{'type':'minecraft:iron_golem'}]})
        with self.assertRaisesRegex(MissingCapability,'health'):
            b.entity_health(ctx,'iron_golem')

    def test_missing_health_import_is_the_only_classified_contract_failure(self):
        cases = (
            (ImportError("cannot import name 'entity_health' from '_blocks'",
                         name='tools.harness.matrix_domains._blocks'), True),
            (ModuleNotFoundError("No module named 'optional_dependency'",
                                 name='optional_dependency'), False),
            (ImportError("cannot import name 'other' from '_blocks'",
                         name='tools.harness.matrix_domains._blocks'), False),
            (ImportError("cannot import name 'entity_health' from 'other'",
                         name='other'), False),
        )
        for error, expected_failure in cases:
            with self.subTest(error=str(error)), \
                    patch.object(pkgutil, 'iter_modules', return_value=[SimpleNamespace(name='fixture')]), \
                    patch.object(importlib, 'import_module', side_effect=error):
                result = unittest.TestResult()
                self.__class__('test_all_matrix_domains_import_and_health_requires_received_value').run(result)
                self.assertEqual(len(result.failures), int(expected_failure))
                self.assertEqual(len(result.errors), int(not expected_failure))

    def test_every_matrix_domain_imports(self):
        import pkgutil
        from . import matrix_domains
        modules = [entry.name for entry in pkgutil.iter_modules(matrix_domains.__path__)
                   if not entry.ispkg and not entry.name.startswith('test_')]
        self.assertIn('falling_blocks', modules)
        for name in modules:
            with self.subTest(module=name):
                importlib.import_module('tools.harness.matrix_domains.' + name)

    def test_falling_blocks_health_uses_received_metadata(self):
        falling = importlib.import_module('tools.harness.matrix_domains.falling_blocks')
        self.assertIs(falling.entity_health, b.entity_health)
        ctx = self.context(snapshot={'entities': [
            {'type': 'minecraft:iron_golem', 'health': 72.0},
            {'type': 'minecraft:cow', 'health': 10.0},
        ]})
        self.assertEqual(falling.entity_health(ctx, 'iron_golem'), 72.0)
        missing = self.context(snapshot={'entities': [{'type': 'minecraft:iron_golem'}]})
        with self.assertRaisesRegex(MissingCapability, 'snapshot:entities:health'):
            falling.entity_health(missing, 'iron_golem')

    def test_missing_observation_never_becomes_empty_inventory(self):
        with self.assertRaisesRegex(MissingCapability,'snapshot:inventory'):
            b.count(self.context(),'diamond')

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

    def test_root_snbt_not_command_echo(self):
        reply='1, 80, 0 has the following block data: {item:{id:"minecraft:diamond",count:1}}'
        self.assertEqual(b.data(self.context(reply),'block 1 80 0'),'{item:{id:"minecraft:diamond",count:1}}')
        with self.assertRaises(RuntimeError):
            b.data(self.context('data get block 1 80 0'),'block 1 80 0')

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
    def test_full_block_fixture_repositions_after_floor_is_built(self):
        from .matrix_domains import block_place
        ctx = self.context()
        events = []
        client = ctx.clients[0]
        client.teleport_to = lambda *pos: events.append(('teleport', pos))
        client.select_hotbar = lambda slot: None
        client.snapshot = lambda: {'inventory': [{'slot': 36, 'item_id': 'minecraft:stone', 'count': 16}]}
        ctx.server.ensure_loaded = lambda area: events.append(('loaded', area))
        ctx.server.command = lambda text: events.append(('command', text))
        ctx.server.query = lambda text: events.append(('fixture_ready', text)) or 'MatrixPlacementFixtureReady'
        ctx.wait_until = lambda predicate, **kwargs: self.assertTrue(predicate())
        block_place.setup(ctx)
        ready = next(i for i, event in enumerate(events) if event[0] == 'fixture_ready')
        self.assertEqual(events[ready + 1], ('teleport', (4.5, 80, 8.5)))


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
