"""Received-state interaction custody probes, composed by native-matrix."""
from __future__ import annotations
import math
from typing import Any
from .probe_steps import item_count
from .native_v2 import _block, _point, _use

SUBJECTS = {'block_interact.jukebox', 'block_interact.shelf', 'block_interact.composter'}


def run(lane: Any, scenario: dict[str, Any]) -> None:
    steps = lane.steps
    if scenario['context']['player_count'] != '1':
        raise NotImplementedError('multi_client_same_target_oracle: interaction custody is single-actor only')
    state = steps.wait(lambda s: s['player']['on_ground'], 'fixture_grounded')
    origin = tuple(math.floor(state['player'][key]) for key in ('x', 'y', 'z'))
    point = _point(origin)
    x, y, z = point.values()
    name = lane.names[0]
    lane.console(f'clear {name}')
    subject = scenario['subject']
    if subject == 'block_interact.jukebox':
        lane.console(f'setblock {x} {y} {z} minecraft:jukebox')
        lane.console(f'setblock {x} {y} {z - 1} minecraft:comparator[facing=north]')
        lane.console(f'setblock {x} {y} {z - 2} minecraft:redstone_wire')
        lane.console(f'give {name} minecraft:music_disc_13 1')
        steps.call('minecraft_select_hotbar_item', {'item_id': 'minecraft:music_disc_13', 'count': 1})
        _use(steps, point)
        _block(steps, point, 'minecraft:jukebox', {'has_record': 'true'})
        steps.wait(lambda s: item_count(s, 'minecraft:music_disc_13') == 0, 'disc_debited_once')
        lane.custody_boundary()
        _block(steps, point, 'minecraft:jukebox', {'has_record': 'true'})
        _block(steps, {'x':x, 'y':y, 'z':z-2}, 'minecraft:redstone_wire', {'power':'1'})
        steps.check('persisted_disc_not_in_inventory', item_count(steps.observe(), 'minecraft:music_disc_13') == 0)
        steps.call('minecraft_set_hotbar_slot', {'slot':8})
        _use(steps, point)
        _block(steps, point, 'minecraft:jukebox', {'has_record':'false'})
        _block(steps, {'x':x, 'y':y, 'z':z-2}, 'minecraft:redstone_wire', {'power':'0'})
        def exactly_one(_: Any) -> bool:
            rows = steps.call('minecraft_list_entities', {'radius':8, 'limit':128})['entities']
            return sum(row.get('item',{}).get('count',0) for row in rows
                       if row.get('item',{}).get('item_id') == 'minecraft:music_disc_13') + item_count(steps.observe(),'minecraft:music_disc_13') == 1
        steps.wait(exactly_one, 'disc_ejected_exactly_once_no_copy')
    elif subject == 'block_interact.shelf':
        lane.console(f'setblock {x} {y} {z} minecraft:oak_shelf[facing=north]')
        lane.console(f'give {name} minecraft:diamond 1')
        steps.call('minecraft_select_hotbar_item', {'item_id':'minecraft:diamond', 'count':1})
        _use(steps, point, 'north')
        steps.wait(lambda s: item_count(s,'minecraft:diamond') == 0, 'shelf_deposit_debited')
        lane.custody_boundary()
        lane.console(f'give {name} minecraft:emerald 1')
        steps.call('minecraft_select_hotbar_item', {'item_id':'minecraft:emerald', 'count':1})
        _use(steps, point, 'north')
        steps.wait(lambda s: item_count(s,'minecraft:diamond') == 1 and item_count(s,'minecraft:emerald') == 0,
                   'persisted_shelf_swap_conserved')
        steps.call('minecraft_set_hotbar_slot', {'slot':8})
        _use(steps, point, 'north')
        steps.wait(lambda s: item_count(s,'minecraft:diamond') == 1 and item_count(s,'minecraft:emerald') == 1,
                   'shelf_returned_one_emerald_no_copy')
        # Repeating a take with an empty hand must not reveal a hidden copy.
        steps.call('minecraft_set_hotbar_slot',{'slot':7})
        steps.wait(lambda s:s['selected_item']['count']==0,'empty_hand_for_shelf_empty_probe')
        _use(steps,point,'north')
        steps.call('minecraft_break_block',{**point,'face':'north',
                   'expected_drop_item_id':'minecraft:oak_shelf','expected_drop_count':1,
                   'timeout_seconds':steps.remaining()})
        _block(steps,point,'minecraft:air')
        state=steps.observe()
        steps.check('destroyed_empty_shelf_has_no_hidden_copy',
                    item_count(state,'minecraft:diamond')==1 and item_count(state,'minecraft:emerald')==1)
        rows=steps.call('minecraft_list_entities',{'radius':16,'limit':512})
        steps.check('shelf_no_ground_custody_copy',not rows['truncated'] and not any(
            row.get('item',{}).get('item_id') in {'minecraft:diamond','minecraft:emerald'}
            for row in rows['entities']))
    else:
        lane.console(f'setblock {x} {y} {z} minecraft:composter')
        for level in range(1,8):
            # Cake is unstackable; each guaranteed compost increment consumes
            # a freshly received single cake, never a nonexistent stack of 7.
            lane.console(f'give {name} minecraft:cake 1')
            steps.call('minecraft_select_hotbar_item', {'item_id':'minecraft:cake', 'count':1})
            _use(steps,point)
            _block(steps,point,'minecraft:composter',{'level':str(level)})
        _block(steps,point,'minecraft:composter',{'level':'8'})
        lane.custody_boundary()
        _block(steps,point,'minecraft:composter',{'level':'8'})
        steps.wait(lambda s:item_count(s,'minecraft:cake') == 0,'seven_compost_inputs_consumed')
        steps.call('minecraft_set_hotbar_slot',{'slot':8})
        _use(steps,point)
        _block(steps,point,'minecraft:composter',{'level':'0'})
        steps.call('minecraft_look',{'yaw_deg':0,'pitch_deg':0})
        steps.call('minecraft_press_inputs',{'keys':['forward'],'ticks':8})
        steps.wait(lambda s:item_count(s,'minecraft:bone_meal') == 1,'bone_meal_collected_once_after_restart')
        rows=steps.call('minecraft_list_entities',{'radius':8,'limit':128})['entities']
        steps.check('no_duplicate_bone_meal',not any(row.get('item',{}).get('item_id') == 'minecraft:bone_meal' for row in rows))
