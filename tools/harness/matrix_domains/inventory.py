"""Vanilla 26.1.2 inventory interactions, including intermediate custody checks."""
from ._items import (
    adapter, block_is, check, client, command, condition, count, cursor, entities,
    give, inventory_slot, menu, menu_slot, number, observe, open_menu, player_menu_slot,
    put, query, require_verbs, select, setup, summon, ticks, verb, WIKI,
)

SOURCE = WIKI + 'Inventory'


def pickup(ctx):
    give(ctx, 'stone', 9)
    give(ctx, 'dirt', 3)
    open_menu(ctx)
    put(ctx, 'stone', 0)
    verb(ctx, 'click_slot', 0, button=1)
    check(ctx, 'right_pickup_ceiling_half', cursor(ctx)['count'], 5, source=SOURCE)
    check(ctx, 'right_pickup_remainder', menu_slot(ctx, 0)['count'], 4, source=SOURCE)
    verb(ctx, 'click_slot', 1, button=1)
    check(ctx, 'right_place_one', menu_slot(ctx, 1)['count'], 1, source=SOURCE)
    verb(ctx, 'click_slot', 0)
    check(ctx, 'merge_same_item', menu_slot(ctx, 0)['count'], 8, source=SOURCE)
    put(ctx, 'dirt', 2)
    verb(ctx, 'click_slot', 0)
    verb(ctx, 'click_slot', 2)
    check(ctx, 'swap_cursor_different_item', cursor(ctx)['item_id'], 'minecraft:dirt', source=SOURCE)
    check(ctx, 'swap_slot_different_item', menu_slot(ctx, 2)['count'], 8, source=SOURCE)
    verb(ctx, 'click_slot', 3)
    check(ctx, 'total_stone_conserved', sum(menu_slot(ctx, i).get('count', 0) for i in (1, 2)), 9, source=SOURCE)
    check(ctx, 'total_dirt_conserved', menu_slot(ctx, 3)['count'], 3, source=SOURCE)


def quick_move(ctx):
    give(ctx, 'stone', 64)
    open_menu(ctx)
    src = player_menu_slot(ctx, inventory_slot(ctx, 'stone'))
    verb(ctx, 'click_slot', src, kind='quick_move')
    check(ctx, 'first_container_slot', menu_slot(ctx, 0)['count'], 64, source=SOURCE)
    verb(ctx, 'click_slot', 0, kind='quick_move')
    check(ctx, 'withdraw_exact', count(ctx, 'stone'), 64, source=SOURCE)
    # Saturate the target, then prove a second shift-click leaves its source.
    for slot in range(27):
        give(ctx, 'dirt', 64)
        put(ctx, 'dirt', slot)
    src = player_menu_slot(ctx, inventory_slot(ctx, 'stone'))
    verb(ctx, 'click_slot', src, kind='quick_move')
    check(ctx, 'full_destination_keeps_source', count(ctx, 'stone'), 64, source=SOURCE)
    check(ctx, 'full_destination_unchanged', sum(menu_slot(ctx, i)['count'] for i in range(27)), 1728, source=SOURCE)


def swap(ctx):
    give(ctx, 'stone', 9)
    give(ctx, 'dirt', 3)
    open_menu(ctx)
    put(ctx, 'stone', 0)
    hotbar = inventory_slot(ctx, 'dirt') - 36
    verb(ctx, 'click_slot', 0, kind='swap', button=hotbar)
    check(ctx, 'number_key_container', menu_slot(ctx, 0)['item_id'], 'minecraft:dirt', source=SOURCE)
    check(ctx, 'number_key_inventory', count(ctx, 'stone'), 9, source=SOURCE)
    verb(ctx, 'click_slot', 0, kind='swap', button=40)
    check(ctx, 'offhand_swap_destination', next(s['count'] for s in observe(ctx, 'inventory') if s['slot'] == 45), 3, source=SOURCE)
    check(ctx, 'offhand_swap_origin_empty', menu_slot(ctx, 0)['count'], 0, source=SOURCE)


def clone(ctx):
    give(ctx, 'stone', 3)
    open_menu(ctx)
    put(ctx, 'stone', 0)
    verb(ctx, 'click_slot', 0, kind='clone', button=2)
    creative = ctx.factors['game_mode'] == 'creative'
    check(ctx, 'clone_cursor_by_mode', cursor(ctx)['count'], 64 if creative else 0, source=SOURCE)
    check(ctx, 'clone_does_not_debit', menu_slot(ctx, 0)['count'], 3, source=SOURCE)


def throw(ctx):
    give(ctx, 'stone', 9)
    open_menu(ctx)
    put(ctx, 'stone', 0)
    verb(ctx, 'click_slot', 0, kind='throw', button=0)
    check(ctx, 'throw_one_debit', menu_slot(ctx, 0)['count'], 8, source=SOURCE)
    verb(ctx, 'click_slot', 0, kind='throw', button=1)
    check(ctx, 'throw_stack_debit', menu_slot(ctx, 0)['count'], 0, source=SOURCE)
    command(ctx, f'tp {client(ctx).name} -5 180 -5')
    ticks(ctx, 2)
    check(ctx, 'thrown_entities_total', sum(e['item']['count'] for e in entities(ctx, 'item') if e['item']['item_id'] == 'minecraft:stone'), 9, source=WIKI + 'Item_(entity)')


def quick_craft(ctx):
    give(ctx, 'stone', 10)
    open_menu(ctx)
    src = player_menu_slot(ctx, inventory_slot(ctx, 'stone'))
    verb(ctx, 'click_slot', src)
    verb(ctx, 'click_slot', -999, kind='quick_craft', button=0)
    for slot in (0, 1, 2):
        verb(ctx, 'click_slot', slot, kind='quick_craft', button=1)
    verb(ctx, 'click_slot', -999, kind='quick_craft', button=2)
    check(ctx, 'left_drag_even_split', [menu_slot(ctx, i)['count'] for i in range(3)], [3, 3, 3], source=SOURCE)
    check(ctx, 'left_drag_cursor_remainder', cursor(ctx)['count'], 1, source=SOURCE)
    verb(ctx, 'click_slot', 0)
    verb(ctx, 'click_slot', 0)
    verb(ctx, 'click_slot', -999, kind='quick_craft', button=4)
    for slot in (3, 4):
        verb(ctx, 'click_slot', slot, kind='quick_craft', button=5)
    verb(ctx, 'click_slot', -999, kind='quick_craft', button=6)
    check(ctx, 'right_drag_one_each', [menu_slot(ctx, i)['count'] for i in (3, 4)], [1, 1], source=SOURCE)
    check(ctx, 'right_drag_cursor_remainder', cursor(ctx)['count'], 2, source=SOURCE)
    if ctx.factors['game_mode'] == 'creative':
        verb(ctx, 'click_slot', -999, kind='quick_craft', button=8)
        for slot in (5, 6):
            verb(ctx, 'click_slot', slot, kind='quick_craft', button=9)
        verb(ctx, 'click_slot', -999, kind='quick_craft', button=10)
        check(ctx, 'creative_middle_drag_fill', [menu_slot(ctx, i)['count'] for i in (5, 6)], [64, 64], source=SOURCE)


def pickup_all(ctx):
    give(ctx, 'stone', 70)
    open_menu(ctx)
    put(ctx, 'stone', 0, 20)
    put(ctx, 'stone', 1, 20)
    verb(ctx, 'click_slot', 0)
    verb(ctx, 'click_slot', 0, kind='pickup_all')
    check(ctx, 'double_click_stack_cap', cursor(ctx)['count'], 64, source=SOURCE)
    total = sum(s.get('count', 0) for s in menu(ctx)['slots'] if s and s.get('item_id') == 'minecraft:stone')
    check(ctx, 'double_click_remainder', total, 6, source=SOURCE)


def stale(ctx):
    require_verbs(ctx, 'stale_click')
    give(ctx, 'stone', 9)
    open_menu(ctx)
    old = menu(ctx)['state_id']
    put(ctx, 'stone', 0)
    verb(ctx, 'stale_click', slot=0, kind='pickup', button=0, state_id=old)
    # Mojang 26.1.2 handleContainerClick executes then broadcastsFullState.
    source = 'local:scenarios/runs/DomItems/facts/ServerGamePacketListenerImpl.javap:5717-5775'
    check(ctx, 'stale_click_applied_once', cursor(ctx)['count'], 9, source=source)
    check(ctx, 'stale_click_source_empty', menu_slot(ctx, 0)['count'], 0, source=source)
    events = observe(ctx, 'events')
    condition(ctx, 'stale_click_full_resync', 'menu_full_sync' in events, events, source=source)


def cursor_close(ctx):
    require_verbs(ctx, 'close_container')
    give(ctx, 'stone', 9)
    open_menu(ctx)
    put(ctx, 'stone', 0)
    verb(ctx, 'click_slot', 0)
    verb(ctx, 'close_container')
    check(ctx, 'close_returns_cursor', count(ctx, 'stone'), 9, source=SOURCE)
    check(ctx, 'closed_cursor_empty', cursor(ctx)['count'], 0, source=SOURCE)
    open_menu(ctx)
    put(ctx, 'stone', 0)
    # Fill all 36 inventory slots while the nine stones are in the chest.
    give(ctx, 'dirt', 36 * 64)
    verb(ctx, 'click_slot', 0)
    verb(ctx, 'close_container')
    command(ctx, f'tp {client(ctx).name} -5 180 -5')
    ticks(ctx, 2)
    check(ctx, 'full_inventory_close_drops_exact', sum(e['item']['count'] for e in entities(ctx, 'item') if e['item']['item_id'] == 'minecraft:stone'), 9, source=SOURCE)


def hotbar(ctx):
    items = ('stone', 'dirt', 'glass', 'oak_planks', 'cobblestone', 'bricks',
             'sandstone', 'red_wool', 'blue_wool')
    for item in items:
        give(ctx, item, 2)
    for i, item in enumerate(items):
        verb(ctx, 'select_hotbar', i)
        if len(ctx.clients) > 1:
            equipment = observe(ctx, 'equipment', observer=1)
            check(ctx, f'observer_mainhand_{i}', equipment[client(ctx).name]['mainhand']['item_id'], 'minecraft:' + item, source=WIKI + 'Hotbar')
        command(ctx, 'setblock 2 180 0 air')
        client(ctx).look_at(2.5, 179.5, .5)
        verb(ctx, 'use_item_on', (2, 179, 0), face='up')
        check(ctx, f'selected_slot_{i}_used_by_server', block_is(ctx, (2, 180, 0), item), True, source=WIKI + 'Hotbar')
        check(ctx, f'selected_slot_{i}_debit', count(ctx, item), 2 if ctx.factors['game_mode'] == 'creative' else 1, source=WIKI + 'Hotbar')


def swap_hands(ctx):
    require_verbs(ctx, 'swap_hands')
    give(ctx, 'stone', 9)
    give(ctx, 'dirt', 3)
    select(ctx, 'stone')
    verb(ctx, 'swap_hands')
    main_slot = inventory_slot(ctx, 'dirt')
    select(ctx, 'dirt')
    verb(ctx, 'swap_hands')
    check(ctx, 'offhand_dirt', next(s['count'] for s in observe(ctx, 'inventory') if s['slot'] == 45), 3, source=SOURCE)
    check(ctx, 'hands_custody', (count(ctx, 'stone'), count(ctx, 'dirt')), (9, 3), source=SOURCE)
    # The first check invokes the lifecycle boundary. Assert slot identities
    # again afterward: conserved counts alone cannot detect a reverted swap.
    slots = {s['slot']: s for s in observe(ctx, 'inventory')}
    check(ctx, 'post_boundary_offhand_identity', slots[45]['item_id'], 'minecraft:dirt', source=SOURCE)
    check(ctx, 'post_boundary_mainhand_identity', slots[main_slot]['item_id'], 'minecraft:stone', source=SOURCE)


def drop(ctx):
    give(ctx, 'stone', 9)
    select(ctx, 'stone')
    verb(ctx, 'drop', )
    check(ctx, 'Q_debits_one', count(ctx, 'stone'), 8, source=WIKI + 'Item_(entity)')
    verb(ctx, 'drop', all=True)
    check(ctx, 'ctrl_Q_debits_remainder', count(ctx, 'stone'), 0, source=WIKI + 'Item_(entity)')
    command(ctx, f'tp {client(ctx).name} -5 180 -5')
    ticks(ctx, 2)
    check(ctx, 'drop_custody', sum(e['item']['count'] for e in entities(ctx, 'item') if e['item']['item_id'] == 'minecraft:stone'), 9, source=WIKI + 'Item_(entity)')


def pickup_entity(ctx):
    give(ctx, 'stone', 63)
    give(ctx, 'dirt', 35 * 64)
    command(ctx, 'summon item 0.5 180 0.5 {Item:{id:"minecraft:stone",count:5},PickupDelay:40s}')
    ticks(ctx, 2)
    check(ctx, 'pickup_delay_holds', count(ctx, 'stone'), 63, source=WIKI + 'Item_(entity)')
    ticks(ctx, 42)
    check(ctx, 'partial_pickup_fills_existing', count(ctx, 'stone'), 64, source=WIKI + 'Item_(entity)')
    check(ctx, 'partial_pickup_remainder', sum(e['item']['count'] for e in entities(ctx, 'item') if e['item']['item_id'] == 'minecraft:stone'), 4, source=WIKI + 'Item_(entity)')


def creative(ctx):
    require_verbs(ctx, 'set_creative_slot')
    verb(ctx, 'set_creative_slot', slot=36, item_id='minecraft:diamond', count=17, components={})
    expected = 17 if ctx.factors['game_mode'] == 'creative' else 0
    check(ctx, 'creative_slot_admission_by_mode', count(ctx, 'diamond'), expected, source=WIKI + 'Creative')
    verb(ctx, 'set_creative_slot', slot=36, item_id='minecraft:air', count=0, components={})
    check(ctx, 'creative_delete', count(ctx, 'diamond'), 0, source=WIKI + 'Creative')


def armor(ctx):
    require_verbs(ctx, 'use_item')
    for item, slot in (('iron_boots', 8), ('iron_leggings', 7), ('iron_chestplate', 6), ('iron_helmet', 5)):
        give(ctx, item)
        select(ctx, item)
        verb(ctx, 'use_item', hand='main')
        check(ctx, item + '_matching_slot', next((s['item_id'] for s in observe(ctx, 'inventory') if s['slot'] == slot), None), 'minecraft:' + item, source=WIKI + 'Armor')
    check(ctx, 'post_boundary_armor_slots',
          {s['slot']: s['item_id'] for s in observe(ctx, 'inventory') if 5 <= s['slot'] <= 8},
          {8: 'minecraft:iron_boots', 7: 'minecraft:iron_leggings',
           6: 'minecraft:iron_chestplate', 5: 'minecraft:iron_helmet'}, source=WIKI + 'Armor')
    give(ctx, 'diamond_chestplate')
    select(ctx, 'diamond_chestplate')
    verb(ctx, 'use_item', hand='main')
    equipped = next(s for s in observe(ctx, 'inventory') if s['slot'] == 6)
    check(ctx, 'use_swaps_existing_chestplate', equipped['item_id'], 'minecraft:diamond_chestplate', source=WIKI + 'Armor')
    check(ctx, 'swapped_chestplate_retained_once', count(ctx, 'iron_chestplate'), 1, source=WIKI + 'Armor')
    # Binding is a separate removal condition, not inferred from successful equip.
    command(ctx, f'clear {client(ctx).name}')
    give(ctx, 'iron_chestplate[enchantments={"minecraft:binding_curse":1}]')
    select(ctx, 'iron_chestplate')
    verb(ctx, 'use_item', hand='main')
    verb(ctx, 'click_slot', 6, kind='quick_move')
    expected = ctx.factors['game_mode'] != 'creative'
    check(ctx, 'binding_prevents_survival_removal', any(s['slot'] == 6 for s in observe(ctx, 'inventory')), expected, source=WIKI + 'Curse_of_Binding')


def recipe_book(ctx):
    require_verbs(ctx, 'place_recipe')
    give(ctx, 'oak_log', 3)
    open_menu(ctx, 'crafting_table')
    verb(ctx, 'place_recipe', recipe_id='minecraft:oak_planks', all=False)
    check(ctx, 'make_one_grid', menu_slot(ctx, 1)['count'], 1, source=WIKI + 'Recipe_book')
    check(ctx, 'make_one_output', menu_slot(ctx, 0)['count'], 4, source=WIKI + 'Recipe_book')
    verb(ctx, 'place_recipe', recipe_id='minecraft:oak_planks', all=True)
    check(ctx, 'make_all_grid', menu_slot(ctx, 1)['count'], 3, source=WIKI + 'Recipe_book')
    verb(ctx, 'place_recipe', recipe_id='minecraft:diamond_pickaxe', all=False)
    check(ctx, 'missing_ingredients_no_result', menu_slot(ctx, 0)['count'], 0, source=WIKI + 'Recipe_book')
    grid_logs = sum(menu_slot(ctx, slot)['count'] for slot in range(1, 10)
                    if menu_slot(ctx, slot).get('item_id') == 'minecraft:oak_log')
    check(ctx, 'recipe_placement_conserves_ingredients', count(ctx, 'oak_log') + grid_logs, 3, source=WIKI + 'Recipe_book')
    ghost = observe(ctx, 'recipe_ghost')
    check(ctx, 'missing_recipe_ghost', ghost['recipe_id'], 'minecraft:diamond_pickaxe', source=WIKI + 'Recipe_book')


def merge(ctx):
    command(ctx, 'summon item 4.5 180 0.5 {Item:{id:"minecraft:stone",count:40},PickupDelay:32767s}')
    command(ctx, 'summon item 4.6 180 0.5 {Item:{id:"minecraft:stone",count:30},PickupDelay:32767s}')
    ticks(ctx, 80)
    counts = sorted(e['item']['count'] for e in entities(ctx, 'item') if e['item']['item_id'] == 'minecraft:stone')
    check(ctx, 'merged_total_conserved', sum(counts), 70, source=WIKI + 'Item_(entity)')
    condition(ctx, 'merge_never_exceeds_stack_limit', all(0 < n <= 64 for n in counts), counts, source=WIKI + 'Item_(entity)')
    # Two stacks whose sum exceeds 64 need not merge; test a mergeable pair too.
    command(ctx, 'kill @e[type=item]')
    for n in (10, 20):
        command(ctx, f'summon item 4.5 180 0.5 {{Item:{{id:"minecraft:stone",count:{n}}},PickupDelay:32767s}}')
    ticks(ctx, 80)
    check(ctx, 'mergeable_pair_coalesces', [e['item']['count'] for e in entities(ctx, 'item') if e['item']['item_id'] == 'minecraft:stone'], [30], source=WIKI + 'Item_(entity)')


def xp(ctx):
    give(ctx, 'diamond_pickaxe[damage=20,enchantments={"minecraft:mending":1}]')
    select(ctx, 'diamond_pickaxe')
    command(ctx, f'experience set {client(ctx).name} 0 points')
    command(ctx, 'summon experience_orb 0.5 180 0.5 {Value:5s}')
    ticks(ctx, 20)
    tools = [s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:diamond_pickaxe']
    if 'components' not in tools[0]:
        from ..matrix_api import MissingCapability
        raise MissingCapability('snapshot:stack:components')
    check(ctx, 'mending_two_durability_per_xp', tools[0]['components'].get('minecraft:damage', 0), 10, source=WIKI + 'Mending')
    check(ctx, 'repair_consumes_xp', number(ctx, client(ctx).name, 'XpTotal'), 0, source=WIKI + 'Mending')
    command(ctx, f'clear {client(ctx).name}')
    before = number(ctx, client(ctx).name, 'XpTotal')
    command(ctx, 'summon experience_orb 0.5 180 0.5 {Value:5s}')
    ticks(ctx, 20)
    check(ctx, 'unspent_orb_adds_xp', number(ctx, client(ctx).name, 'XpTotal') - before, 5, source=WIKI + 'Experience')
    command(ctx, 'kill @e[type=experience_orb]')
    # Vanilla merges only matching entity-id residues modulo 40; 41 equal
    # stationary orbs guarantee a mergeable pair, unlike a random small sample.
    for _ in range(41):
        command(ctx, 'summon experience_orb 7.5 180 0.5 {Value:1s,Motion:[0d,0d,0d]}')
    ticks(ctx, 40)
    orbs = entities(ctx, 'experience_orb')
    if any('value' not in orb or 'count' not in orb for orb in orbs):
        from ..matrix_api import MissingCapability
        raise MissingCapability('snapshot:experience_orb:value_count')
    check(ctx, 'merged_orb_xp_conserved', sum(orb['value'] * orb['count'] for orb in orbs), 41, source=WIKI + 'Experience')
    condition(ctx, 'equal_value_orbs_merge', len(orbs) < 41, orbs, source='local:scenarios/runs/DomItems/facts/ExperienceOrb.javap:651-670')


ADAPTERS = {f'inventory.{name}': adapter(fn) for name, fn in {
    'click_pickup': pickup, 'click_quick_move': quick_move, 'click_swap': swap,
    'click_clone': clone, 'click_throw': throw, 'click_quick_craft': quick_craft,
    'click_pickup_all': pickup_all, 'stale_state_id': stale, 'cursor_on_close': cursor_close,
    'hotbar_select': hotbar, 'swap_hands': swap_hands, 'drop_item': drop,
    'pickup_item_entity': pickup_entity, 'creative_inventory': creative, 'armor_equip': armor,
    'recipe_book_place': recipe_book, 'item_entity_merge': merge, 'xp_orb_pickup': xp,
}.items()}
