"""Container matrix adapters: finite-stock transactions, never command-only passes.

Vanilla 26.1.2 expectations are cited per subject in SOURCES. Optional driver
observations/verbs fail closed, rather than treating an absent field as zero.
Menu slots use protocol indices, including the player suffix exactly once.
"""
from __future__ import annotations

from functools import partial
import re
from typing import Any

from ..matrix_api import Adapter, Check, MatrixContext, MissingCapability

POS = (32, 80, 32)
OTHER = (35, 80, 32)
SOURCES = {
    'player_inventory_2x2': 'https://minecraft.wiki/w/Inventory#Crafting',
    'crafting_table': 'https://minecraft.wiki/w/Crafting_Table#Crafting',
    'chest': 'https://minecraft.wiki/w/Chest#Container',
    'trapped_chest': 'https://minecraft.wiki/w/Trapped_Chest#Redstone_component',
    'double_chest': 'https://minecraft.wiki/w/Chest#Container',
    'barrel': 'https://minecraft.wiki/w/Barrel#Container',
    'copper_chest': 'https://minecraft.wiki/w/Copper_Chest#Usage',
    'ender_chest': 'https://minecraft.wiki/w/Ender_Chest#Usage',
    'shulker_box': 'https://minecraft.wiki/w/Shulker_Box#Usage',
    'hopper': 'https://minecraft.wiki/w/Hopper#Transferring_items',
    'dispenser': 'https://minecraft.wiki/w/Dispenser#Usage',
    'dropper': 'https://minecraft.wiki/w/Dropper#Usage',
    'crafter': 'https://minecraft.wiki/w/Crafter#Usage',
    'furnace': 'https://minecraft.wiki/w/Smelting#Smelting_times',
    'smoker': 'https://minecraft.wiki/w/Smoker#Smelting',
    'blast_furnace': 'https://minecraft.wiki/w/Blast_Furnace#Smelting',
    'brewing_stand': 'https://minecraft.wiki/w/Brewing_Stand#Brewing',
    'enchanting_table': 'https://minecraft.wiki/w/Enchanting_Table#Enchanting',
    'anvil': 'https://minecraft.wiki/w/Anvil_mechanics',
    'grindstone': 'https://minecraft.wiki/w/Grindstone#Usage',
    'smithing_table': 'https://minecraft.wiki/w/Smithing_Table#Smithing',
    'loom': 'https://minecraft.wiki/w/Loom#Usage',
    'stonecutter': 'https://minecraft.wiki/w/Stonecutter#Stonecutting',
    'cartography_table': 'https://minecraft.wiki/w/Cartography_Table#Usage',
    'beacon': 'https://minecraft.wiki/w/Beacon#Powers',
    'merchant_villager': 'https://minecraft.wiki/w/Trading#Mechanics',
    'merchant_wandering_trader': 'https://minecraft.wiki/w/Wandering_Trader#Trading',
    'chest_boat': 'https://minecraft.wiki/w/Boat_with_Chest#Usage',
    'chest_minecart': 'https://minecraft.wiki/w/Minecart_with_Chest#Usage',
    'hopper_minecart': 'https://minecraft.wiki/w/Minecart_with_Hopper#Usage',
    'plugin_chest_menu': 'scenarios/live-plugins/basic-economy/README.md:6-9 (plugin contract, not vanilla)',
    'mount_inventory': 'https://minecraft.wiki/w/Chest#Donkey,_mule,_or_llama_pack',
    'lectern': 'https://minecraft.wiki/w/Lectern#Holding_books',
}
STORAGE = {'chest': 27, 'trapped_chest': 27, 'double_chest': 54,
           'barrel': 27, 'copper_chest': 27, 'ender_chest': 27, 'shulker_box': 27}
ENTITY_TYPES = {'chest_boat': 'oak_chest_boat', 'chest_minecart': 'chest_minecart',
                'hopper_minecart': 'hopper_minecart', 'merchant_villager': 'villager',
                'merchant_wandering_trader': 'wandering_trader', 'mount_inventory': 'donkey'}

# Real partial adapters, not registration stubs. These clauses cannot be inferred
# from a passing transfer; never award a full subject pass without their evidence.
CLAUSE_GAPS = {
    'hopper': ('observation.menu.transition_ticks', 'eight-tick transfer cadence'),
    'brewing_stand': ('observation.menu.transition_ticks', 'exact 400-tick brew interval'),
    'plugin_chest_menu': ('observation.plugin_menu_click', 'click delivered to owning plugin'),
}


def _call(obj: Any, method: str, *args: Any, **kwargs: Any) -> Any:
    fn = getattr(obj, method, None)
    if not callable(fn):
        raise MissingCapability('client:' + method)
    return fn(*args, **kwargs)


def _field(value: dict, key: str, capability: str) -> Any:
    if key not in value or value[key] is None:
        raise MissingCapability(capability)
    return value[key]


def _snapshot(ctx: MatrixContext, who: int = 0) -> dict:
    return ctx.clients[who].snapshot()


def _menu(ctx: MatrixContext, who: int = 0) -> dict:
    return _field(_snapshot(ctx, who), 'menu', 'observation.menu')


def _slots(ctx: MatrixContext, who: int = 0) -> list[dict]:
    return _field(_menu(ctx, who), 'slots', 'observation.menu.slots')


def _slot(ctx: MatrixContext, index: int, who: int = 0) -> dict:
    slots = _slots(ctx, who)
    for stack in slots:
        if 'slot' not in stack:
            raise MissingCapability('observation.menu.slots.slot')
        if stack['slot'] == index:
            return stack
    return {'slot': index, 'item_id': 'minecraft:air', 'count': 0, 'components': {}}


def _count(stacks: list[dict], item: str) -> int:
    return sum(s['count'] for s in stacks if s['item_id'] == 'minecraft:' + item)


def _inventory(ctx: MatrixContext, who: int = 0) -> list[dict]:
    return _field(_snapshot(ctx, who), 'inventory', 'observation.inventory')


def _record(ctx: MatrixContext, name: str, actual: Any, expected: Any) -> None:
    ctx.state['checks'].append({'name': name, 'passed': actual == expected, 'detail': {
        'actual': actual, 'expected': expected, 'version': '26.1.2',
        'source': SOURCES[ctx.state['subject']]}})


def _predicate(ctx: MatrixContext, name: str, passed: bool, detail: Any) -> None:
    ctx.state['checks'].append({'name': name, 'passed': bool(passed), 'detail': {
        'observation': detail, 'version': '26.1.2', 'source': SOURCES[ctx.state['subject']]}})


def _command(ctx: MatrixContext, text: str) -> None:
    # Console completion is not a success assertion. Each setup has state evidence.
    ctx.server.command(text)


def _block(ctx: MatrixContext, block: str, pos: tuple = POS) -> None:
    _command(ctx, f'setblock {pos[0]} {pos[1]} {pos[2]} {block}')


def _give(ctx: MatrixContext, item: str, count: int = 1, who: int = 0) -> None:
    plain = item.split('[', 1)[0].removeprefix('minecraft:')
    before = _count(_inventory(ctx, who), plain)
    _command(ctx, f'give {ctx.clients[who].name} {item} {count}')
    ctx.wait_until(lambda: _count(_inventory(ctx, who), plain) == before + count,
                   description=f'received finite fixture {plain} +{count}')


def _open(ctx: MatrixContext, pos: tuple = POS, who: int = 0) -> None:
    ctx.clients[who].open_container(pos)
    ctx.wait_until(lambda: _snapshot(ctx, who).get('menu') is not None,
                   description='received container contents')

def _use_on_ack(ctx: MatrixContext, pos: tuple = POS) -> None:
    token = ctx.clients[0].use_item_on(pos)
    if type(token) is not int:
        raise MissingCapability('block_ack_sequence')
    def admitted():
        snapshot = _snapshot(ctx)
        if 'block_ack_sequence' not in snapshot:
            raise MissingCapability('block_ack_sequence')
        observed = snapshot['block_ack_sequence']
        return type(observed) is int and observed >= token
    ctx.wait_until(admitted, description='exact block interaction admission acknowledgement')



def _close(ctx: MatrixContext, who: int = 0) -> None:
    _call(ctx.clients[who], 'close_container')
    _ticks(ctx, 2)


def _size(ctx: MatrixContext, size: int) -> None:
    actual = _field(_menu(ctx), 'slot_count', 'observation.menu.slot_count')
    _record(ctx, 'menu_slot_count', actual, size)


def _put(ctx: MatrixContext, item: str, destination: int, player_start: int,
         *, accepted: bool = True) -> None:
    source = next((s for s in _slots(ctx) if s['slot'] >= player_start
                   and s['item_id'] == 'minecraft:' + item), None)
    if source is None:
        raise AssertionError(f'fixture item {item} absent from received menu')
    actor = ctx.clients[0]
    actor.click_slot(source['slot'])
    ctx.wait_until(lambda: (_snapshot(ctx).get('cursor') or {}).get('count') == source['count'],
                   description='received pickup cursor')
    actor.click_slot(destination)
    if accepted:
        ctx.wait_until(lambda: _slot(ctx, destination)['item_id'] == 'minecraft:' + item,
                       description='received placed input stack')
    else:
        _ticks(ctx, 2)


def _take(ctx: MatrixContext, slot: int) -> None:
    before = _slot(ctx, slot)
    ctx.clients[0].click_slot(slot, kind='quick_move')
    ctx.wait_until(lambda: _slot(ctx, slot) != before, description='received quick-move result')


def _distribute(ctx: MatrixContext, item: str, destinations: list[tuple[int, int]],
                player_start: int) -> None:
    for destination, count in destinations:
        source = next((s for s in _slots(ctx) if s['slot'] >= player_start
                       and s['item_id'] == 'minecraft:' + item), None)
        if source is None or source['count'] < count:
            raise AssertionError(f'not enough finite {item} for slot {destination}')
        actor = ctx.clients[0]
        actor.click_slot(source['slot'])
        ctx.wait_until(lambda: (_snapshot(ctx).get('cursor') or {}).get('count') == source['count'],
                       description='received distribution cursor')
        for expected in range(1, count + 1):
            actor.click_slot(destination, button=1)
            ctx.wait_until(lambda expected=expected: _slot(ctx, destination)['count'] == expected,
                           description='received single-item placement')
        if source['count'] > count:
            actor.click_slot(source['slot'])
            ctx.wait_until(lambda: _snapshot(ctx).get('cursor') is None,
                           description='received returned remainder')


def _ticks(ctx: MatrixContext, count: int) -> None:
    def tick():
        stats = ctx.server.dashboard_stats()
        return _field(stats, 'simulation_tick', 'dashboard.simulation_tick')
    first = tick()
    ctx.wait_until(lambda: tick() >= first + count,
                   description=f'dashboard advances {count} simulation ticks',
                   timeout_seconds=max(30.0, count / 10))


def _block_state(ctx: MatrixContext, pos: tuple = POS) -> dict:
    return _call(ctx.clients[0], 'read_block_state', pos)


def _property(ctx: MatrixContext, key: str) -> Any:
    properties = _field(_menu(ctx), 'properties', 'observation.menu.properties')
    if key in properties:
        return properties[key]
    # Native sources: inventory/furnace.rs:51-59, brewing.rs:27-28,
    # inventory/stations.rs:72-76,91-92, crafter.rs:512-523; lectern.rs page property 0.
    def raw(index):
        def received():
            current = _field(_menu(ctx), 'properties', 'observation.menu.properties')
            return index in current or str(index) in current
        if not received():
            ctx.wait_until(received, description=f'received menu property {key}[{index}]')
        current = _menu(ctx)['properties']
        return current[index] if index in current else current[str(index)]
    if key == 'enchant_costs':
        return [raw(index) for index in range(3)]
    if key == 'disabled_slots':
        return [index for index in range(9) if raw(index) == 1]
    indices = {'cook_total': 3, 'cook_progress': 2, 'burn_remaining': 0,
               'burn_total': 1, 'brew_time': 0, 'fuel': 1, 'enchant_seed': 3, 'level_cost': 0,
               'page': 0}
    if key in indices:
        return raw(indices[key])
    raise MissingCapability('observation.menu.properties.' + key)


def _components(stack: dict) -> dict:
    return _field(stack, 'components', 'observation.stack.components')


def _registry_name(ctx: MatrixContext, registry: str, identifier: int) -> str:
    registries = _field(_snapshot(ctx), 'registries', 'observation.registries')
    names = _field(registries, registry, 'observation.registries.' + registry)
    return _field(names, str(identifier), 'observation.registries.' + registry + '.' + str(identifier))


def _enchantments(ctx: MatrixContext, stack: dict) -> dict:
    pairs = _components(stack).get('minecraft:enchantments', [])
    # Native MCP exposes actual names; protocol exposes IDs plus its registry.
    if isinstance(pairs, dict):
        if not all(isinstance(name, str) and ':' in name and type(level) is int
                   for name, level in pairs.items()):
            raise MissingCapability('observation.stack.components.enchantments')
        return pairs
    return {_registry_name(ctx, 'minecraft:enchantment', identifier): level
            for identifier, level in pairs}


def _potion(ctx: MatrixContext, stack: dict) -> str | None:
    contents = _field(_components(stack), 'minecraft:potion_contents',
                      'observation.stack.components.minecraft:potion_contents')
    identifier = contents.get('potion')
    if isinstance(identifier, str) and ':' in identifier:
        return identifier
    return None if identifier is None else _registry_name(ctx, 'minecraft:potion', identifier)


def _data_number(ctx: MatrixContext, target: str, path: str, capability: str) -> int:
    reply = ctx.server.query(f'data get {target} {path} 1')
    if ('Unknown or incomplete command' in reply
            or 'Data editing currently supports command blocks only' in reply):
        raise MissingCapability(capability)
    match = re.search(r' after scale factor of 1\.00 is (-?\d+)(?:\s|$)', reply)
    if match is None:
        raise AssertionError(f'authoritative numeric query failed: {reply}')
    return int(match.group(1))


def _boundary(ctx: MatrixContext) -> None:
    """Driver owns crash/network injection; reconnect does not recreate menu grids."""
    subject = ctx.state['subject']
    transient = {'player_inventory_2x2': (9, [1, 2, 3, 4]),
                 'crafting_table': (10, list(range(1, 10))),
                 'enchanting_table': (2, [0, 1]), 'anvil': (3, [0, 1]),
                 'grindstone': (3, [0, 1]), 'smithing_table': (4, [0, 1, 2]),
                 'loom': (4, [0, 1, 2]), 'stonecutter': (2, [0]),
                 'cartography_table': (3, [0, 1]), 'beacon': (1, [0]),
                 'merchant_villager': (3, [0, 1]), 'merchant_wandering_trader': (3, [0, 1])}
    saved = []
    if subject in transient and ctx.factors['game_mode'] != 'spectator':
        saved = [_slot(ctx, slot) for slot in transient[subject][1]]
    ctx.boundary()
    disconnected = (ctx.factors['network_condition'] in {'disconnect_during_menu', 'reconnect_mid_action', 'duplicate_login'}
                    or ctx.factors['server_lifecycle'] in {
                        'clean_restart_mid_action', 'crash_kill9_wal_replay', 'pause_when_empty_resume'})
    if not disconnected:
        return
    if subject in ENTITY_TYPES:
        entity = _entity(ctx, ENTITY_TYPES[subject])
        _entity_menu(ctx, entity['id'], secondary=subject not in {
            'merchant_villager', 'merchant_wandering_trader'})
    elif subject == 'player_inventory_2x2':
        _call(ctx.clients[0], 'open_inventory')
    elif subject == 'plugin_chest_menu':
        ctx.clients[0].command('economy shop')
    else:
        _open(ctx)
    for stack in saved:
        if stack['count']:
            _distribute(ctx, stack['item_id'].removeprefix('minecraft:'),
                        [(stack['slot'], stack['count'])], transient[subject][0])
    if subject in {'enchanting_table', 'anvil', 'stonecutter', 'merchant_villager', 'merchant_wandering_trader'}:
        ctx.state['menu_reopened'] = True


def _entity(ctx: MatrixContext, kind: str) -> dict:
    entities = _field(_snapshot(ctx), 'entities', 'observation.entities')
    found = [e for e in entities if e.get('type') == 'minecraft:' + kind]
    if len(found) != 1:
        raise AssertionError(f'expected exactly one fixture {kind}, received {found}')
    return found[0]


def _entity_menu(ctx: MatrixContext, entity_id: int, *, secondary: bool = True) -> None:
    _call(ctx.clients[0], 'interact_entity', entity_id, secondary=secondary)
    ctx.wait_until(lambda: (_snapshot(ctx).get('menu') or {}).get('id', 0) != 0,
                   description='received entity container contents')


def _world_stock(ctx: MatrixContext, item: str) -> int:
    entities = _field(_snapshot(ctx), 'entities', 'observation.entities')
    drops = [_field(e, 'item', 'observation.entities.item')
             for e in entities if e['type'] == 'minecraft:item']
    return _count(_inventory(ctx), item) + _count(drops, item)


def _xp(ctx: MatrixContext, key: str) -> int:
    snapshot = _snapshot(ctx)
    if 'xp' in snapshot:
        return _field(snapshot['xp'], key, 'observation.xp.' + key)
    return _field(snapshot, 'experience_' + key, 'observation.experience_' + key)


def _effects(ctx: MatrixContext) -> list[dict]:
    effects = _field(_snapshot(ctx), 'effects', 'observation.effects')
    if isinstance(effects, dict):
        return [dict(value, id=_registry_name(ctx, 'minecraft:mob_effect', int(key)))
                for key, value in effects.items()]
    return effects


def setup(ctx: MatrixContext, subject: str) -> None:
    ctx.state.update(subject=subject, checks=[])
    origin = ctx.row.get('origin', '')
    # These matrix origins are extra actions, not ordinary network/lifecycle factors.
    # No driver currently exposes a coordinated dual-click/destruction receipt.
    if 'two_players_same_slot_same_tick' in origin:
        raise MissingCapability('same_tick_admission_fence')
    if 'break_container_while_open' in origin:
        raise MissingCapability('container_break_while_open_custody')
    # Keep fixture clear of basic-economy's automatic zone (-8..8).
    _command(ctx, 'forceload add 26 26 40 40')
    for who, client in enumerate(ctx.clients):
        client.teleport_to(31.5 + who % 2, 80, 30.5)
    # A teleport packet is not a chunk-loaded receipt. No retrying failed fills.
    for x in (26, 40):
        for z in (26, 40):
            _call(ctx.clients[0], 'wait_for_chunk', (x, 80, z))
    _command(ctx, 'fill 26 75 26 40 79 40 stone')
    _command(ctx, 'fill 26 80 26 40 86 40 air')
    for who, client in enumerate(ctx.clients):
        client.teleport_to(31.5 + who % 2, 80, 30.5)
        _command(ctx, f'clear {client.name}')
        ctx.wait_until(lambda who=who: not _inventory(ctx, who),
                       description='received cleared inventory fixture')
    if subject in ENTITY_TYPES:
        entity_type = ENTITY_TYPES[subject]
        if entity_type == 'chest_minecart':
            for x in range(32, 36):
                _block(ctx, 'powered_rail[shape=east_west,powered=false]', (x, 80, 32))
            for x in (31, 36):
                _block(ctx, 'stone', (x, 80, 32))
        elif entity_type == 'hopper_minecart':
            _block(ctx, 'redstone_block', (32, 79, 32))
            _block(ctx, 'activator_rail[shape=east_west,powered=true]')
        nbt = ' {Tame:1b}' if subject == 'mount_inventory' else ''
        if subject == 'merchant_villager':
            nbt = ' {VillagerData:{profession:"minecraft:farmer",level:1,type:"minecraft:plains"}}'
            _block(ctx, 'composter', OTHER)
        _command(ctx, f'summon {entity_type} 32.5 80 32.5{nbt}')
        ctx.wait_until(lambda: any(e.get('type') == 'minecraft:' + entity_type
                                  for e in _field(_snapshot(ctx), 'entities', 'observation.entities')),
                       description='received fixture entity spawn')
    elif subject == 'double_chest':
        _block(ctx, 'chest[facing=north,type=left]')
        _block(ctx, 'chest[facing=north,type=right]', (33, 80, 32))
    elif subject not in {'player_inventory_2x2', 'plugin_chest_menu'}:
        _block(ctx, subject)
    if (subject in STORAGE or subject in {'chest_boat', 'chest_minecart'}
            or ctx.factors['game_mode'] == 'spectator'):
        _give(ctx, 'stone', 16)
    if subject == 'trapped_chest':
        _block(ctx, 'redstone_wire', (32, 80, 33))
    if subject == 'ender_chest':
        _block(ctx, 'ender_chest', OTHER)
    if subject == 'beacon':
        for depth in range(1, 5):
            _command(ctx, f'fill {32-depth} {80-depth} {32-depth} '
                          f'{32+depth} {80-depth} {32+depth} iron_block')
        _command(ctx, 'fill 32 81 32 32 100 32 air')
    if subject == 'enchanting_table':
        # 13 lower shelves plus two upper shelves, all with a one-block gap.
        shelves = [(x, 80, 34) for x in range(30, 35)]
        shelves += [(x, 80, z) for x in (30, 34) for z in range(30, 34)]
        shelves += [(30, 81, 32), (34, 81, 32)]
        for position in shelves:
            _block(ctx, 'bookshelf', position)


def _spectator(ctx: MatrixContext, size: int | None = None) -> bool:
    if ctx.factors['game_mode'] != 'spectator':
        return False
    before = _inventory(ctx)
    if ctx.state['subject'] == 'player_inventory_2x2':
        _call(ctx.clients[0], 'open_inventory')
    else:
        _use_on_ack(ctx)
    _ticks(ctx, 2)
    menu = _snapshot(ctx).get('menu')
    if ctx.state['subject'] != 'player_inventory_2x2' and menu is not None and menu.get('id') == 0:
        menu = None
    if size is None:
        _record(ctx, 'spectator_cannot_open_workstation', menu, None)
    if size is not None:
        _predicate(ctx, 'spectator_can_view_storage', menu is not None, menu)
        if menu is not None:
            _size(ctx, size)
    if menu is not None:
        before_slots = _slots(ctx)
        source = next((s['slot'] for s in before_slots
                       if s['item_id'] == 'minecraft:stone'), None)
        if source is None:
            raise AssertionError('spectator fixture stone absent from menu')
        ctx.clients[0].click_slot(source, kind='quick_move')
        _ticks(ctx, 2)
        _record(ctx, 'spectator_menu_read_only', _slots(ctx), before_slots)
    _record(ctx, 'spectator_inventory_read_only', _inventory(ctx), before)
    _boundary(ctx)
    return True


def storage(ctx: MatrixContext) -> None:
    subject = ctx.state['subject']
    capacity = STORAGE[subject]
    if _spectator(ctx, capacity + 36):
        return
    _open(ctx)
    _size(ctx, capacity + 36)
    _put(ctx, 'stone', 0, capacity)
    _record(ctx, 'deposit_16', _slot(ctx, 0)['count'], 16)
    if subject not in {'ender_chest', 'double_chest'}:
        _record(ctx, 'authoritative_deposit_count',
                _data_number(ctx, 'block 32 80 32', 'Items[0].count', 'data_get_block'), 16)
    for who in range(1, len(ctx.clients)):
        _open(ctx, who=who)
        expected = 0 if subject == 'ender_chest' else 16
        _record(ctx, f'viewer_{who}_storage', _slot(ctx, 0, who)['count'], expected)
    if subject == 'barrel':
        _record(ctx, 'barrel_open', _block_state(ctx).get('properties', {}).get('open'), 'true')
    if subject == 'trapped_chest':
        power = _block_state(ctx, (32, 80, 33)).get('properties', {}).get('power')
        _record(ctx, 'power_equals_viewers', power, str(min(15, len(ctx.clients))))
    _boundary(ctx)
    _close(ctx)
    target = (33, 80, 32) if subject == 'double_chest' else OTHER if subject == 'ender_chest' else POS
    _open(ctx, target)
    _record(ctx, 'reopened_storage_preserves_16', _slot(ctx, 0)['count'], 16)
    _take(ctx, 0)
    _record(ctx, 'withdraw_empties_storage', _slot(ctx, 0)['count'], 0)
    _record(ctx, 'finite_stock_conserved', _count(_slots(ctx), 'stone'), 16)
    for who in range(1, len(ctx.clients)):
        if subject != 'ender_chest':
            ctx.wait_until(lambda who=who: _slot(ctx, 0, who)['count'] == 0,
                           description='viewer receives withdrawal')
        _close(ctx, who)
    _close(ctx)
    if subject == 'barrel':
        _record(ctx, 'barrel_closed', _block_state(ctx).get('properties', {}).get('open'), 'false')
    if subject == 'trapped_chest':
        _ticks(ctx, 2)
        _record(ctx, 'power_zero_after_close',
                _block_state(ctx, (32, 80, 33)).get('properties', {}).get('power'), '0')
    if subject == 'shulker_box':
        _give(ctx, 'shulker_box')
        _open(ctx)
        _put(ctx, 'shulker_box', 0, 27, accepted=False)
        _record(ctx, 'nested_shulker_refused', _slot(ctx, 0)['count'], 0)
        cursor = _field(_snapshot(ctx), 'cursor', 'observation.cursor')
        _record(ctx, 'refused_box_stays_on_cursor', cursor['item_id'], 'minecraft:shulker_box')
        _close(ctx)
    if subject == 'copper_chest':
        _open(ctx)
        _put(ctx, 'stone', 0, 27)
        _close(ctx)
        _block(ctx, 'chest', OTHER)
        _open(ctx, OTHER)
        _record(ctx, 'sorting_destination_initially_empty',
                _count([s for s in _slots(ctx) if s['slot'] < 27], 'stone'), 0)
        _command(ctx, 'summon copper_golem 33.5 80 32.5')
        ctx.wait_until(lambda: any(e['type'] == 'minecraft:copper_golem'
                       for e in _field(_snapshot(ctx), 'entities', 'observation.entities')),
                       description='received sorting golem')
        ctx.wait_until(lambda: _count([s for s in _slots(ctx) if s['slot'] < 27], 'stone') == 16,
                       description='golem moves finite stock from copper chest into wooden chest',
                       timeout_seconds=60)
        _record(ctx, 'golem_sorts_16_into_wooden_chest',
                _count([s for s in _slots(ctx) if s['slot'] < 27], 'stone'), 16)
        _close(ctx)
        _open(ctx)
        _record(ctx, 'golem_empties_copper_source',
                _count([s for s in _slots(ctx) if s['slot'] < 27], 'stone'), 0)


def crafting(ctx: MatrixContext) -> None:
    personal = ctx.state['subject'] == 'player_inventory_2x2'
    if _spectator(ctx, 46 if personal else None):
        return
    _give(ctx, 'oak_log' if personal else 'oak_planks', 2 if personal else 16)
    if personal:
        _call(ctx.clients[0], 'open_inventory')
    else:
        _open(ctx)
    _size(ctx, 46)
    if personal:
        _record(ctx, 'personal_inventory_window_zero', _field(_menu(ctx), 'id', 'observation.menu.id'), 0)
        _put(ctx, 'oak_log', 1, 9)
    else:
        _distribute(ctx, 'oak_planks', [(slot, 2) for slot in (1, 2, 3, 4, 6, 7, 8, 9)], 10)
    _record(ctx, 'recipe_result', (_slot(ctx, 0)['item_id'], _slot(ctx, 0)['count']),
            ('minecraft:oak_planks', 4) if personal else ('minecraft:chest', 1))
    if not personal:
        ctx.clients[0].click_slot(1)
        ctx.wait_until(lambda: _slot(ctx, 0)['count'] == 0,
                       description='incomplete ring removes crafting result')
        _record(ctx, 'recipe_recomputed_after_removal', _slot(ctx, 0)['count'], 0)
        ctx.clients[0].click_slot(1)
        ctx.wait_until(lambda: _slot(ctx, 0)['item_id'] == 'minecraft:chest',
                       description='restored ring recomputes crafting result')
        _record(ctx, 'recipe_recomputed_after_restore', _slot(ctx, 0)['count'], 1)
    _boundary(ctx)
    _take(ctx, 0)
    _record(ctx, 'shift_craft_inputs_consumed', _slot(ctx, 1)['count'], 0)
    _record(ctx, 'shift_craft_maximum_output',
            _count(_slots(ctx), 'oak_planks' if personal else 'chest'), 8 if personal else 2)
    _close(ctx)
    if personal:
        _give(ctx, 'iron_helmet')
        _call(ctx.clients[0], 'open_inventory')
        _put(ctx, 'iron_helmet', 5, 9)
        _record(ctx, 'matching_head_slot', _slot(ctx, 5)['item_id'], 'minecraft:iron_helmet')
        _put(ctx, 'oak_planks', 45, 9)
        _record(ctx, 'offhand_accepts_stack', _slot(ctx, 45)['count'], 8)
        _take(ctx, 45)
        _record(ctx, 'offhand_returns_stack', _slot(ctx, 45)['count'], 0)
        _put(ctx, 'oak_planks', 6, 9, accepted=False)
        _record(ctx, 'nonarmor_refused', _slot(ctx, 6)['count'], 0)
        _close(ctx)
    else:
        _give(ctx, 'stick')
        _open(ctx)
        _put(ctx, 'stick', 1, 10)
        _close(ctx)
        _record(ctx, 'grid_returned_on_close', _count(_inventory(ctx), 'stick'), 1)


def cooking(ctx: MatrixContext) -> None:
    if _spectator(ctx):
        return
    subject = ctx.state['subject']
    raw, output, duration = ('beef', 'cooked_beef', 100) if subject == 'smoker' else (
        'iron_ore', 'iron_ingot', 200 if subject == 'furnace' else 100)
    _give(ctx, raw)
    _give(ctx, 'coal')
    _open(ctx)
    _size(ctx, 39)
    # Fuel stays observable while input is empty; inserting it last may burn it
    # in the same tick before the client predicate sees the intermediate stack.
    _put(ctx, 'coal', 1, 3)
    _put(ctx, raw, 0, 3)
    ctx.wait_until(lambda: _property(ctx, 'cook_total') > 0 and _property(ctx, 'burn_remaining') > 0,
                   description='received active furnace properties')
    _record(ctx, 'recipe_duration_ticks', _property(ctx, 'cook_total'), duration)
    burn = _property(ctx, 'burn_remaining')
    progress = _property(ctx, 'cook_progress')
    _predicate(ctx, 'fuel_timer_positive', 0 < burn <= _property(ctx, 'burn_total'), burn)
    ctx.wait_until(lambda: _property(ctx, 'burn_remaining') < burn,
                   description='received fuel timer decrement')
    _predicate(ctx, 'fuel_property_ticks', _property(ctx, 'burn_remaining') < burn,
               {'before': burn, 'after': _property(ctx, 'burn_remaining')})
    ctx.wait_until(lambda: _property(ctx, 'cook_progress') > progress or _slot(ctx, 2)['count'] > 0,
                   description='received cooking progress or completed output')
    _predicate(ctx, 'progress_property_ticks',
               _property(ctx, 'cook_progress') > progress or _slot(ctx, 2)['count'] == 1,
               {'before': progress, 'after': _property(ctx, 'cook_progress'),
                'output': _slot(ctx, 2)['count']})
    _boundary(ctx)
    ctx.wait_until(lambda: _slot(ctx, 2)['item_id'] == 'minecraft:' + output,
                   description=f'{subject} produces {output}')
    _record(ctx, 'input_consumed', _slot(ctx, 0)['count'], 0)
    _record(ctx, 'fuel_consumed', _slot(ctx, 1)['count'], 0)
    _record(ctx, 'output_exactly_one', _slot(ctx, 2)['count'], 1)
    xp = _xp(ctx, 'total')
    _take(ctx, 2)
    _record(ctx, 'output_take_debits_slot', _slot(ctx, 2)['count'], 0)
    _record(ctx, 'output_in_inventory', _count(_slots(ctx), output), 1)
    # XP can round down (0.7 iron / 0.35 beef); require integer bounded award, not random success.
    after = _xp(ctx, 'total')
    _predicate(ctx, 'xp_rounding_bounds', 0 <= after - xp <= 1, {'before': xp, 'after': after})
    _close(ctx)
    if subject != 'furnace':
        invalid = 'iron_ore' if subject == 'smoker' else 'beef'
        _give(ctx, invalid)
        _open(ctx)
        _put(ctx, invalid, 0, 3)
        _ticks(ctx, duration + 2)
        _record(ctx, 'ineligible_recipe_no_output', _slot(ctx, 2)['count'], 0)
        _record(ctx, 'ineligible_input_retained', _count(_slots(ctx), invalid), 1)


def hopper(ctx: MatrixContext) -> None:
    if _spectator(ctx, 41):
        return
    _block(ctx, 'hopper[facing=east]')
    _block(ctx, 'chest', (33, 80, 32))
    _block(ctx, 'redstone_block', (32, 81, 32))
    _give(ctx, 'stone', 3)
    _open(ctx)
    _size(ctx, 41)
    _put(ctx, 'stone', 0, 5)
    _ticks(ctx, 16)
    _record(ctx, 'powered_hopper_locked', _slot(ctx, 0)['count'], 3)
    _boundary(ctx)
    _block(ctx, 'air', (32, 81, 32))
    ctx.wait_until(lambda: _slot(ctx, 0)['count'] == 0, description='three hopper transfers')
    _close(ctx)
    _open(ctx, (33, 80, 32))
    _record(ctx, 'destination_received_all_three', _count(_slots(ctx), 'stone'), 3)


def redstone_output(ctx: MatrixContext) -> None:
    if _spectator(ctx, 45):
        return
    dropper = ctx.state['subject'] == 'dropper'
    _block(ctx, ('dropper' if dropper else 'dispenser') + '[facing=east]')
    if dropper:
        _block(ctx, 'chest', (33, 80, 32))
    _give(ctx, 'stone' if dropper else 'arrow', 2)
    _open(ctx)
    _size(ctx, 45)
    _put(ctx, 'stone' if dropper else 'arrow', 0, 9)
    _boundary(ctx)
    _block(ctx, 'redstone_block', (32, 81, 32))
    _ticks(ctx, 8)
    _record(ctx, 'one_item_per_rising_edge', _slot(ctx, 0)['count'], 1)
    _ticks(ctx, 8)
    _record(ctx, 'held_signal_does_not_repeat', _slot(ctx, 0)['count'], 1)
    if dropper:
        _close(ctx)
        _open(ctx, (33, 80, 32))
        _record(ctx, 'dropper_transferred_one', _slot(ctx, 0)['count'], 1)
    else:
        entities = _field(_snapshot(ctx), 'entities', 'observation.entities')
        _record(ctx, 'arrow_projectile_spawned',
                sum(e.get('type') == 'minecraft:arrow' for e in entities), 1)


def crafter(ctx: MatrixContext) -> None:
    if _spectator(ctx, 46):
        return
    _block(ctx, 'crafter[orientation=east_up]')
    _block(ctx, 'chest', (33, 80, 32))
    _give(ctx, 'oak_log', 2)
    _open(ctx)
    _size(ctx, 46)
    for slot in range(1, 9):
        _call(ctx.clients[0], 'toggle_crafter_slot', slot, True)
    ctx.wait_until(lambda: _property(ctx, 'disabled_slots') == list(range(1, 9)),
                   description='received disabled crafter slots')
    _record(ctx, 'disabled_slots', _property(ctx, 'disabled_slots'), list(range(1, 9)))
    _put(ctx, 'oak_log', 0, 9)
    _boundary(ctx)
    _block(ctx, 'redstone_block', (32, 81, 32))
    _ticks(ctx, 8)
    _record(ctx, 'one_craft_consumes_one_log', _slot(ctx, 0)['count'], 1)
    _ticks(ctx, 8)
    _record(ctx, 'held_signal_no_second_craft', _slot(ctx, 0)['count'], 1)
    _close(ctx)
    _open(ctx, (33, 80, 32))
    _record(ctx, 'crafted_four_planks', _count(_slots(ctx), 'oak_planks'), 4)


def brewing(ctx: MatrixContext) -> None:
    if _spectator(ctx, 41):
        return
    _give(ctx, 'potion[potion_contents={potion:"minecraft:water"}]')
    _give(ctx, 'nether_wart')
    _give(ctx, 'blaze_powder')
    _open(ctx)
    _size(ctx, 41)
    _put(ctx, 'potion', 0, 5)
    _put(ctx, 'nether_wart', 3, 5)
    _put(ctx, 'blaze_powder', 4, 5)
    _predicate(ctx, 'brew_progress_400_tick_range', 0 < _property(ctx, 'brew_time') <= 400,
               _property(ctx, 'brew_time'))
    _boundary(ctx)
    ctx.wait_until(lambda: _potion(ctx, _slot(ctx, 0)) == 'minecraft:awkward',
                   description='awkward potion brewing completion')
    _record(ctx, 'ingredient_debited', _slot(ctx, 3)['count'], 0)
    _record(ctx, 'powder_debited', _slot(ctx, 4)['count'], 0)
    _record(ctx, 'nineteen_brews_remaining', _property(ctx, 'fuel'), 19)
    _record(ctx, 'one_bottle_preserved', _slot(ctx, 0)['count'], 1)


def enchanting(ctx: MatrixContext) -> None:
    if _spectator(ctx):
        return
    _command(ctx, f'xp set {ctx.clients[0].name} 30 levels')
    _give(ctx, 'diamond_sword')
    _give(ctx, 'lapis_lazuli', 3)
    _open(ctx)
    _put(ctx, 'diamond_sword', 0, 2)
    _put(ctx, 'lapis_lazuli', 1, 2)
    _record(ctx, 'third_offer_requires_30', _property(ctx, 'enchant_costs')[2], 30)
    costs = _property(ctx, 'enchant_costs')
    _predicate(ctx, 'three_offers_with_fifteen_shelves',
               len(costs) == 3 and 2 <= costs[0] <= 10 and 6 <= costs[1] <= 21 and costs[2] == 30,
               {'costs': costs, 'source': 'https://minecraft.wiki/w/Enchanting_table_mechanics#Experience_cost'})
    seed = _property(ctx, 'enchant_seed')
    _close(ctx)
    _open(ctx)
    _put(ctx, 'diamond_sword', 0, 2)
    _put(ctx, 'lapis_lazuli', 1, 2)
    _record(ctx, 'same_seed_same_offers', _property(ctx, 'enchant_costs'), costs)
    _record(ctx, 'viewing_does_not_reroll_seed', _property(ctx, 'enchant_seed'), seed)
    _boundary(ctx)
    _call(ctx.clients[0], 'container_button', 2)
    ctx.wait_until(lambda: bool(_enchantments(ctx, _slot(ctx, 0))),
                   description='received enchanted item components')
    enchantments = _enchantments(ctx, _slot(ctx, 0))
    _predicate(ctx, 'sword_is_enchanted', bool(enchantments), enchantments)
    creative = ctx.factors['game_mode'] == 'creative'
    _record(ctx, 'lapis_cost', _slot(ctx, 1)['count'], 3 if creative else 0)
    _record(ctx, 'level_cost', _xp(ctx, 'level'),
            30 if creative else 27)
    _predicate(ctx, 'enchant_seed_rerolled', _property(ctx, 'enchant_seed') != seed,
               {'before': seed, 'after': _property(ctx, 'enchant_seed')})


def anvil(ctx: MatrixContext) -> None:
    if _spectator(ctx):
        return
    _command(ctx, f'xp set {ctx.clients[0].name} 50 levels')
    _give(ctx, 'diamond_sword')
    _open(ctx)
    _put(ctx, 'diamond_sword', 0, 3)
    _call(ctx.clients[0], 'rename_item', 'Matrix blade')
    ctx.wait_until(lambda: _property(ctx, 'level_cost') == 1, description='received rename cost')
    _record(ctx, 'fresh_rename_cost', _property(ctx, 'level_cost'), 1)
    _boundary(ctx)
    if ctx.state.get('menu_reopened'):
        _call(ctx.clients[0], 'rename_item', 'Matrix blade')
        ctx.wait_until(lambda: _property(ctx, 'level_cost') == 1, description='received reopened rename cost')
    _take(ctx, 2)
    sword = next(s for s in _slots(ctx) if s['item_id'] == 'minecraft:diamond_sword')
    _record(ctx, 'rename_component', _components(sword).get('minecraft:custom_name'), 'Matrix blade')
    _record(ctx, 'rename_level_debit', _xp(ctx, 'level'),
            50 if ctx.factors['game_mode'] == 'creative' else 49)
    _close(ctx)
    block = _field(_block_state(ctx), 'block_id', 'observation.block_id')
    allowed = {'minecraft:anvil'} if ctx.factors['game_mode'] == 'creative' else {
        'minecraft:anvil', 'minecraft:chipped_anvil'}
    _predicate(ctx, 'anvil_degrades_at_most_one_stage', block in allowed,
               {'block': block, 'allowed': sorted(allowed), 'source': 'https://minecraft.wiki/w/Anvil#Damaging_the_anvil'})
    for item, material in [('iron_shovel', 'iron_ingot'), ('iron_pickaxe', 'iron_pickaxe')]:
        _block(ctx, 'anvil')
        _give(ctx, item + '[damage=1]')
        _give(ctx, material + ('[damage=1]' if material == item else ''))
        _open(ctx)
        _put(ctx, item, 0, 3)
        _put(ctx, material, 1, 3)
        _record(ctx, item + '_repair_result', _slot(ctx, 2)['item_id'], 'minecraft:' + item)
        _record(ctx, item + '_repair_full_durability',
                _components(_slot(ctx, 2)).get('minecraft:damage', 0), 0)
        cost = _property(ctx, 'level_cost')
        _predicate(ctx, item + '_repair_cost_positive', cost > 0, cost)
        before_level = _xp(ctx, 'level')
        _take(ctx, 2)
        _record(ctx, item + '_repair_inputs_consumed', [_slot(ctx, s)['count'] for s in (0, 1)], [0, 0])
        _record(ctx, item + '_repair_level_debit',
                _xp(ctx, 'level'),
                before_level if ctx.factors['game_mode'] == 'creative' else before_level - cost)
        _close(ctx)
    _block(ctx, 'anvil')
    _give(ctx, 'diamond_sword[damage=100,repair_cost=39]')
    _give(ctx, 'diamond')
    _open(ctx)
    # The named sword is deliberately already returned; select the damaged fixture by components.
    source = next(s for s in _slots(ctx) if s['slot'] >= 3 and s['item_id'] == 'minecraft:diamond_sword'
                  and _components(s).get('minecraft:damage') == 100)
    ctx.clients[0].click_slot(source['slot'])
    ctx.wait_until(lambda: (_snapshot(ctx).get('cursor') or {}).get('item_id') == 'minecraft:diamond_sword',
                   description='received damaged sword cursor')
    ctx.clients[0].click_slot(0)
    ctx.wait_until(lambda: _slot(ctx, 0)['item_id'] == 'minecraft:diamond_sword',
                   description='received damaged sword input')
    _put(ctx, 'diamond', 1, 3)
    _predicate(ctx, 'repair_cost_at_least_40', _property(ctx, 'level_cost') >= 40,
               _property(ctx, 'level_cost'))
    if ctx.factors['game_mode'] != 'creative':
        _record(ctx, 'too_expensive_result_empty', _slot(ctx, 2)['count'], 0)


def grindstone(ctx: MatrixContext) -> None:
    if _spectator(ctx):
        return
    _give(ctx, 'diamond_sword[enchantments={"minecraft:sharpness":1,"minecraft:vanishing_curse":1}]')
    _open(ctx)
    _put(ctx, 'diamond_sword', 0, 3)
    ench = _enchantments(ctx, _slot(ctx, 2))
    _record(ctx, 'curse_only_in_result', ench, {'minecraft:vanishing_curse': 1})
    before = _xp(ctx, 'total')
    _boundary(ctx)
    _take(ctx, 2)
    _record(ctx, 'disenchant_input_consumed', _slot(ctx, 0)['count'], 0)
    _close(ctx)
    ctx.wait_until(lambda: _xp(ctx, 'total') > before,
                   description='grindstone XP pickup')
    _predicate(ctx, 'disenchant_awards_xp',
               _xp(ctx, 'total') > before, _xp(ctx, 'total'))
    _give(ctx, 'iron_pickaxe[damage=1]', 2)
    _open(ctx)
    _put(ctx, 'iron_pickaxe', 0, 3)
    _put(ctx, 'iron_pickaxe', 1, 3)
    _record(ctx, 'repair_combines_matching_tools', _slot(ctx, 2)['item_id'], 'minecraft:iron_pickaxe')
    # Two nearly-full tools exceed maximum durability even before the 5% bonus.
    _record(ctx, 'repair_capped_at_full_durability',
            _components(_slot(ctx, 2)).get('minecraft:damage', 0), 0)
    _take(ctx, 2)
    _record(ctx, 'repair_consumes_both_tools', [_slot(ctx, s)['count'] for s in (0, 1)], [0, 0])
    _record(ctx, 'repair_returns_one_tool', _count(_slots(ctx), 'iron_pickaxe'), 1)


def smithing(ctx: MatrixContext) -> None:
    if _spectator(ctx):
        return
    _give(ctx, 'netherite_upgrade_smithing_template')
    _give(ctx, 'diamond_chestplate[damage=10,custom_name="Matrix armor"]')
    _give(ctx, 'netherite_ingot')
    _open(ctx)
    for item, slot in [('netherite_upgrade_smithing_template', 0), ('diamond_chestplate', 1), ('netherite_ingot', 2)]:
        _put(ctx, item, slot, 4)
    result = _slot(ctx, 3)
    _record(ctx, 'upgrade_result', result['item_id'], 'minecraft:netherite_chestplate')
    _record(ctx, 'upgrade_damage_preserved', _components(result).get('minecraft:damage'), 10)
    _record(ctx, 'upgrade_name_preserved', _components(result).get('minecraft:custom_name'), 'Matrix armor')
    _boundary(ctx)
    _take(ctx, 3)
    _record(ctx, 'three_upgrade_inputs_consumed', [_slot(ctx, s)['count'] for s in range(3)], [0, 0, 0])
    _close(ctx)
    _give(ctx, 'sentry_armor_trim_smithing_template')
    _give(ctx, 'redstone')
    _open(ctx)
    for item, slot in [('sentry_armor_trim_smithing_template', 0), ('netherite_chestplate', 1), ('redstone', 2)]:
        _put(ctx, item, slot, 4)
    _record(ctx, 'trim_component', _components(_slot(ctx, 3)).get('minecraft:trim'),
            {'pattern': 'minecraft:sentry', 'material': 'minecraft:redstone'})
    _take(ctx, 3)
    _record(ctx, 'three_trim_inputs_consumed', [_slot(ctx, s)['count'] for s in range(3)], [0, 0, 0])


def loom(ctx: MatrixContext) -> None:
    if _spectator(ctx):
        return
    _give(ctx, 'white_banner')
    _give(ctx, 'red_dye')
    _give(ctx, 'creeper_banner_pattern')
    _open(ctx)
    for item, slot in [('white_banner', 0), ('red_dye', 1), ('creeper_banner_pattern', 2)]:
        _put(ctx, item, slot, 4)
    _record(ctx, 'banner_pattern_result', _components(_slot(ctx, 3)).get('minecraft:banner_patterns'),
            [{'pattern': 'minecraft:creeper', 'color': 'red'}])
    _boundary(ctx)
    _take(ctx, 3)
    _record(ctx, 'banner_and_dye_consumed', [_slot(ctx, s)['count'] for s in (0, 1)], [0, 0])
    _record(ctx, 'pattern_item_reusable', _slot(ctx, 2)['count'], 1)


def stonecutter(ctx: MatrixContext) -> None:
    if _spectator(ctx):
        return
    _give(ctx, 'stone', 3)
    _open(ctx)
    _put(ctx, 'stone', 0, 2)
    recipes = _field(_menu(ctx), 'recipes', 'observation.menu.recipes')
    recipe = next((r for r in recipes if r['result']['item_id'] == 'minecraft:stone_slab'), None)
    if recipe is None:
        raise AssertionError('stone_slab missing from stonecutter recipes')
    _call(ctx.clients[0], 'container_button', recipe['index'])
    ctx.wait_until(lambda: _slot(ctx, 1)['item_id'] == 'minecraft:stone_slab',
                   description='received stonecutter selection result')
    _record(ctx, 'two_slabs_per_stone', _slot(ctx, 1)['count'], 2)
    _boundary(ctx)
    if ctx.state.get('menu_reopened'):
        _call(ctx.clients[0], 'container_button', recipe['index'])
        ctx.wait_until(lambda: _slot(ctx, 1)['item_id'] == 'minecraft:stone_slab',
                       description='received reopened stonecutter result')
    _take(ctx, 1)
    _record(ctx, 'all_three_inputs_consumed', _slot(ctx, 0)['count'], 0)
    _record(ctx, 'six_slabs_received', _count(_slots(ctx), 'stone_slab'), 6)


def cartography(ctx: MatrixContext) -> None:
    if _spectator(ctx):
        return
    _give(ctx, 'map', 2)
    _call(ctx.clients[0], 'use_item')
    ctx.wait_until(lambda: _count(_inventory(ctx), 'filled_map') == 1,
                   description='empty map initialized through use')
    _give(ctx, 'paper')
    _give(ctx, 'glass_pane')
    _open(ctx)
    _put(ctx, 'filled_map', 0, 3)
    _put(ctx, 'paper', 1, 3)
    _boundary(ctx)
    _take(ctx, 2)
    _record(ctx, 'zoom_consumed_paper', _count(_slots(ctx), 'paper'), 0)
    _close(ctx)
    maps = _field(_snapshot(ctx), 'maps', 'observation.maps')
    _record(ctx, 'zoom_scale_one', max(m['scale'] for m in maps), 1)
    _open(ctx)
    _put(ctx, 'filled_map', 0, 3)
    _put(ctx, 'map', 1, 3)
    _take(ctx, 2)
    _record(ctx, 'clone_two_maps', _count(_slots(ctx), 'filled_map'), 2)
    _close(ctx)
    _open(ctx)
    _put(ctx, 'filled_map', 0, 3)
    _put(ctx, 'glass_pane', 1, 3)
    _take(ctx, 2)
    _record(ctx, 'lock_consumed_glass', _count(_slots(ctx), 'glass_pane'), 0)
    _close(ctx)
    _predicate(ctx, 'locked_map_exists', any(m.get('locked') for m in
               _field(_snapshot(ctx), 'maps', 'observation.maps')), _snapshot(ctx).get('maps'))


def beacon(ctx: MatrixContext) -> None:
    if _spectator(ctx):
        return
    _give(ctx, 'iron_ingot')
    _open(ctx)
    _size(ctx, 37)
    _put(ctx, 'iron_ingot', 0, 1)
    _boundary(ctx)
    _call(ctx.clients[0], 'set_beacon', 'minecraft:speed', 'minecraft:regeneration')
    _close(ctx)
    ctx.wait_until(lambda: 'effects' in _snapshot(ctx) and {'minecraft:speed', 'minecraft:regeneration'} <= {
        e['id'] for e in _effects(ctx)}, description='tier-four beacon effects received')
    _record(ctx, 'beacon_payment_consumed', _count(_inventory(ctx), 'iron_ingot'), 0)
    effects = _effects(ctx)
    _predicate(ctx, 'primary_and_secondary_applied', all(any(
        e['id'] == name and e['amplifier'] == 0 for e in effects)
        for name in ('minecraft:speed', 'minecraft:regeneration')), effects)
    ctx.clients[0].teleport_to(110.5, 80, 30.5)
    ctx.wait_until(lambda: not any(e['id'] in {'minecraft:speed', 'minecraft:regeneration'}
                   for e in _effects(ctx)),
                   description='beacon effects expire outside tier-four range', timeout_seconds=30)
    _record(ctx, 'outside_beacon_range_no_effects',
            [e['id'] for e in _effects(ctx)
             if e['id'] in {'minecraft:speed', 'minecraft:regeneration'}], [])


def _merchant_costs(offer: dict) -> list[dict]:
    costs = [_field(offer, 'input1', 'observation.menu.offers.input1')]
    if offer.get('input2') is not None:
        costs.append(offer['input2'])
    if offer['special_price'] != 0 or offer['demand'] != 0:
        raise MissingCapability('observation.menu.offers.effective_costs')
    return costs


def _merchant_open(ctx: MatrixContext, entity_id: int) -> None:
    _call(ctx.clients[0], 'interact_entity', entity_id)
    ctx.wait_until(lambda: (_snapshot(ctx).get('menu') or {}).get('offers') is not None,
                   description='received merchant offers')


def _merchant_fund(ctx: MatrixContext, entity_id: int, costs: list[dict]) -> None:
    _close(ctx)
    for stack in costs:
        _give(ctx, stack['item_id'], stack['count'])
    _merchant_open(ctx, entity_id)
    _call(ctx.clients[0], 'select_trade', 0)
    for slot, stack in enumerate(costs):
        _put(ctx, stack['item_id'].removeprefix('minecraft:'), slot, 3)


def merchant(ctx: MatrixContext) -> None:
    kind = ENTITY_TYPES[ctx.state['subject']]
    entity = _entity(ctx, kind)
    _call(ctx.clients[0], 'interact_entity', entity['id'])
    if ctx.factors['game_mode'] == 'spectator':
        _ticks(ctx, 2)
        _record(ctx, 'spectator_cannot_trade', _snapshot(ctx).get('menu'), None)
        _boundary(ctx)
        return
    ctx.wait_until(lambda: (_snapshot(ctx).get('menu') or {}).get('offers') is not None,
                   description='received initial merchant offers')
    offers = _field(_menu(ctx), 'offers', 'observation.menu.offers')
    _predicate(ctx, 'merchant_has_offers', bool(offers), offers)
    if not offers:
        return
    # Choose an actual received offer; avoid RNG assumptions about its item/cost.
    offer = offers[0]
    costs = _merchant_costs(offer)
    merchant_xp = _field(_menu(ctx), 'xp', 'observation.menu.xp')
    _merchant_fund(ctx, entity['id'], costs)
    _record(ctx, 'trade_result_matches_offer',
            (_slot(ctx, 2)['item_id'], _slot(ctx, 2)['count']),
            (offer['output']['item_id'], offer['output']['count']))
    _boundary(ctx)
    if ctx.state.get('menu_reopened'):
        _call(ctx.clients[0], 'select_trade', 0)
        ctx.wait_until(lambda: _slot(ctx, 2)['count'] > 0, description='received reopened trade result')
    _take(ctx, 2)
    ctx.wait_until(lambda: _field(_menu(ctx), 'offers', 'observation.menu.offers')[0]['uses']
                   == offer['uses'] + 1, description='received trade use increment')
    after = _field(_menu(ctx), 'offers', 'observation.menu.offers')[0]
    _record(ctx, 'trade_use_incremented_once', after['uses'], offer['uses'] + 1)
    _record(ctx, 'payments_consumed', [_slot(ctx, s)['count'] for s in range(len(costs))],
            [0] * len(costs))
    if kind == 'villager':
        _predicate(ctx, 'villager_experience_increased',
                   _field(_menu(ctx), 'xp', 'observation.menu.xp') > merchant_xp, _menu(ctx)['xp'])
        _close(ctx)
        _command(ctx, 'time set 2000')
        selector = '@e[type=minecraft:villager,x=32,y=80,z=32,distance=..8,limit=1]'
        ctx.wait_until(lambda: _data_number(ctx, 'entity ' + selector, 'Offers.Recipes[0].uses',
                                            'data_get_entity') == 0,
                       description='villager restocks at fixture composter', timeout_seconds=120)
        _merchant_open(ctx, entity['id'])
        _record(ctx, 'villager_restock_resets_uses', _menu(ctx)['offers'][0]['uses'], 0)
    else:
        _record(ctx, 'trader_does_not_level',
                _field(_menu(ctx), 'show_progress', 'observation.menu.show_progress'), False)
        for uses in range(after['uses'], after['max_uses']):
            _merchant_fund(ctx, entity['id'], costs)
            _take(ctx, 2)
            ctx.wait_until(lambda uses=uses: _menu(ctx)['offers'][0]['uses'] == uses + 1,
                           description='received remaining trade consumption')
        exhausted = _menu(ctx)['offers'][0]
        _record(ctx, 'trader_reaches_max_uses', exhausted['uses'], exhausted['max_uses'])
        _record(ctx, 'trader_offer_locks', exhausted['out_of_stock'], True)
        _merchant_fund(ctx, entity['id'], costs)
        _record(ctx, 'exhausted_trade_has_no_result', _slot(ctx, 2)['count'], 0)
        _record(ctx, 'exhausted_trade_retains_payments',
                [_slot(ctx, s)['count'] for s in range(len(costs))], [s['count'] for s in costs])


def vehicle(ctx: MatrixContext) -> None:
    kind = ENTITY_TYPES[ctx.state['subject']]
    entity = _entity(ctx, kind)
    _entity_menu(ctx, entity['id'])
    capacity = 5 if kind == 'hopper_minecart' else 27
    _size(ctx, capacity + 36)
    if ctx.factors['game_mode'] == 'spectator':
        _record(ctx, 'spectator_storage_unchanged', _count(_slots(ctx), 'stone'), 16)
        _boundary(ctx)
        return
    if kind == 'hopper_minecart':
        _close(ctx)
        _ticks(ctx, 2)
        _command(ctx, 'summon item 32.5 81 32.5 {Item:{id:"minecraft:stone",count:3},PickupDelay:32767}')
        _ticks(ctx, 20)
        _entity_menu(ctx, entity['id'])
        _record(ctx, 'powered_activator_disables_pull', _count(_slots(ctx), 'stone'), 0)
        _record(ctx, 'disabled_cart_preserves_dropped_stock', _world_stock(ctx, 'stone'), 3)
        _block(ctx, 'stone', (32, 79, 32))
        ctx.wait_until(lambda: _count(_slots(ctx), 'stone') == 3,
                       description='unpowered activator reenables hopper cart')
        _record(ctx, 'hopper_cart_collects_items', _count(_slots(ctx), 'stone'), 3)
        _record(ctx, 'collected_items_removed_from_world', _world_stock(ctx, 'stone'), 0)
    else:
        _put(ctx, 'stone', 0, capacity)
        _record(ctx, 'vehicle_deposit', _slot(ctx, 0)['count'], 16)
    _boundary(ctx)
    _close(ctx)
    entity = _entity(ctx, kind)  # Reconnect/restart can change the wire entity ID.
    if kind == 'chest_minecart':
        before = entity['position']
        _block(ctx, 'redstone_block', (32, 79, 32))
        ctx.wait_until(lambda: _entity(ctx, kind)['position'][0] > before[0] + 1,
                       description='received powered minecart movement')
        entity = _entity(ctx, kind)
        _predicate(ctx, 'loaded_cart_moves', entity['position'][0] > before[0] + 1,
                   {'before': before, 'after': entity['position']})
        ctx.clients[0].teleport_to(entity['position'][0], 80, 30.5)
    _entity_menu(ctx, entity['id'])
    _record(ctx, 'vehicle_storage_retained', _slot(ctx, 0)['count'], 3 if capacity == 5 else 16)
    if kind == 'oak_chest_boat':
        _close(ctx)
        _give(ctx, 'iron_axe')
        axe = next(s for s in _inventory(ctx) if s['item_id'] == 'minecraft:iron_axe')
        if not 36 <= axe['slot'] <= 44:
            raise MissingCapability('fixture_hotbar_item:iron_axe')
        ctx.clients[0].select_hotbar(axe['slot'] - 36)
        _ticks(ctx, 25)
        _call(ctx.clients[0], 'attack', entity['id'])
        ctx.wait_until(lambda: not any(e['id'] == entity['id']
                       for e in _field(_snapshot(ctx), 'entities', 'observation.entities')),
                       description='destroyed chest boat removed')
        _ticks(ctx, 2)
        _record(ctx, 'destroyed_boat_drops_all_16', _world_stock(ctx, 'stone'), 16)
        _ticks(ctx, 20)
        _record(ctx, 'destroyed_boat_does_not_duplicate_contents', _world_stock(ctx, 'stone'), 16)


def plugin_menu(ctx: MatrixContext) -> None:
    before = _inventory(ctx)
    ctx.clients[0].command('economy shop')
    ctx.wait_until(lambda: _snapshot(ctx).get('menu') is not None, description='plugin shop opened')
    slots = _slots(ctx)
    _predicate(ctx, 'plugin_menu_has_stock', bool(slots), slots)
    _boundary(ctx)
    # Outside-click cannot extract server-owned display stock into the cursor.
    if slots:
        ctx.clients[0].click_slot(slots[0]['slot'], kind='quick_move')
    _close(ctx)
    _record(ctx, 'display_stock_not_leaked', _inventory(ctx), before)


def mount(ctx: MatrixContext) -> None:
    if ctx.factors['game_mode'] == 'spectator':
        _boundary(ctx)
        raise MissingCapability('spectator.mount_inventory_observation')
    _give(ctx, 'chest')
    inv = next(s for s in _inventory(ctx) if s['item_id'] == 'minecraft:chest')
    if not 36 <= inv['slot'] <= 44:
        raise MissingCapability('fixture_hotbar_item:chest')
    ctx.clients[0].select_hotbar(inv['slot'] - 36)
    entity = _entity(ctx, 'donkey')
    _call(ctx.clients[0], 'interact_entity', entity['id'])
    ctx.wait_until(lambda: _count(_inventory(ctx), 'chest') == 0,
                   description='received mount chest attachment debit')
    _give(ctx, 'stone', 16)
    _entity_menu(ctx, entity['id'])
    _size(ctx, 53)  # saddle + equipment + 15 chest + 36 player slots
    _record(ctx, 'mount_chest_consumed', _count(_inventory(ctx), 'chest'), 0)
    _put(ctx, 'stone', 2, 17)
    _record(ctx, 'mount_cargo_deposit_16', _slot(ctx, 2)['count'], 16)
    _boundary(ctx)
    _record(ctx, 'mount_cargo_retained_16', _slot(ctx, 2)['count'], 16)
    _take(ctx, 2)
    _record(ctx, 'mount_cargo_withdrawn', _slot(ctx, 2)['count'], 0)
    _record(ctx, 'mount_finite_stock_conserved', _count(_inventory(ctx), 'stone'), 16)
    _close(ctx)


def lectern(ctx: MatrixContext) -> None:
    if _spectator(ctx):
        return
    _give(ctx, 'writable_book[writable_book_content={pages:["first","second"]}]')
    inv = next(s for s in _inventory(ctx) if s['item_id'] == 'minecraft:writable_book')
    if not 36 <= inv['slot'] <= 44:
        raise MissingCapability('fixture_hotbar_item:writable_book')
    ctx.clients[0].select_hotbar(inv['slot'] - 36)
    _use_on_ack(ctx)
    _open(ctx)
    _record(ctx, 'lectern_initial_page', _property(ctx, 'page'), 0)
    _call(ctx.clients[0], 'container_button', 2)
    ctx.wait_until(lambda: _property(ctx, 'page') == 1,
                   description='received lectern next page')
    _record(ctx, 'lectern_next_page', _property(ctx, 'page'), 1)
    _boundary(ctx)
    _call(ctx.clients[0], 'container_button', 3)
    ctx.wait_until(lambda: _count(_inventory(ctx), 'writable_book') == 1,
                   description='received lectern book return')
    _record(ctx, 'lectern_take_returns_book', _count(_inventory(ctx), 'writable_book'), 1)


def oracle(ctx: MatrixContext) -> list[Check]:
    checks = ctx.state['checks']
    if not checks:
        raise AssertionError('container action produced no observations')
    if ctx.state['subject'] in CLAUSE_GAPS and all(check['passed'] for check in checks):
        capability, clause = CLAUSE_GAPS[ctx.state['subject']]
        ctx.state['unproved_oracle_clause'] = clause
        raise MissingCapability(capability)
    if len(ctx.clients) > 1 and all(check['passed'] for check in checks):
        names = {check['name'] for check in checks}
        if ctx.state['subject'] not in STORAGE or any(
                f'viewer_{who}_storage' not in names for who in range(1, len(ctx.clients))):
            raise MissingCapability('observer_subject_receipt')
    return [Check(**check) for check in checks]


_ACTIONS = {**{s: storage for s in STORAGE},
    'player_inventory_2x2': crafting, 'crafting_table': crafting,
    'hopper': hopper, 'dispenser': redstone_output, 'dropper': redstone_output,
    'crafter': crafter, 'furnace': cooking, 'smoker': cooking, 'blast_furnace': cooking,
    'brewing_stand': brewing, 'enchanting_table': enchanting, 'anvil': anvil,
    'grindstone': grindstone, 'smithing_table': smithing, 'loom': loom,
    'stonecutter': stonecutter, 'cartography_table': cartography, 'beacon': beacon,
    'merchant_villager': merchant, 'merchant_wandering_trader': merchant,
    'chest_boat': vehicle, 'chest_minecart': vehicle, 'hopper_minecart': vehicle,
    'plugin_chest_menu': plugin_menu, 'mount_inventory': mount, 'lectern': lectern}
ADAPTERS = {'containers.' + subject: Adapter(partial(setup, subject=subject), action, oracle)
            for subject, action in _ACTIONS.items()}
