"""Station recipes, debits and components; recipe values are local 26.1.2 data."""
from ..matrix_api import MissingCapability
from ._items import (
    adapter, block_is, check, client, command, condition, count, cursor, entities,
    give, menu_slot, number, observe, open_menu, put, query, require_verbs,
    select, ticks, verb, WIKI,
)

RECIPES = 'local:data/vanilla/data/minecraft/recipe/'


def reset(ctx, station='crafting_table'):
    require_verbs(ctx, 'close_container')
    if client(ctx).snapshot().get('menu'):
        verb(ctx, 'close_container')
    command(ctx, f'clear {client(ctx).name}')
    command(ctx, 'setblock 2 180 0 air')
    open_menu(ctx, station)


def grid(ctx, ingredients):
    # ingredients is a concrete ordered list of (slot, item, count).
    for item in dict.fromkeys(item for _, item, _ in ingredients):
        give(ctx, item, sum(n for _, it, n in ingredients if it == item))
    for slot, item, amount in ingredients:
        put(ctx, item.split('[', 1)[0], slot, amount)


def take(ctx, item, amount, source, shift=False):
    before = count(ctx, item)
    result = menu_slot(ctx, 0)
    condition(ctx, 'recipe_result', result.get('item_id') == 'minecraft:' + item,
              result, source=source)
    verb(ctx, 'click_slot', 0, kind='quick_move' if shift else 'pickup')
    if shift:
        check(ctx, 'shift_output_debit', count(ctx, item) - before, amount, source=source)
    else:
        check(ctx, 'cursor_output_count', cursor(ctx)['count'], amount, source=source)
        check(ctx, 'cursor_output_item', cursor(ctx)['item_id'], 'minecraft:' + item, source=source)


def shaped(ctx):
    reset(ctx)
    grid(ctx, [(1, 'oak_planks', 1), (2, 'oak_planks', 1), (3, 'oak_planks', 1),
               (5, 'stick', 1), (8, 'stick', 1)])
    take(ctx, 'wooden_pickaxe', 1, RECIPES + 'wooden_pickaxe.json')
    check(ctx, 'pickaxe_grid_consumed', [menu_slot(ctx, i)['count'] for i in range(1, 10)], [0] * 9, source=RECIPES + 'wooden_pickaxe.json')
    for slots in ((1, 4, 5, 7, 8, 9), (3, 5, 6, 7, 8, 9)):
        reset(ctx)
        grid(ctx, [(i, 'oak_planks', 1) for i in slots])
        take(ctx, 'oak_stairs', 4, RECIPES + 'oak_stairs.json')
        check(ctx, 'stairs_grid_consumed', [menu_slot(ctx, i)['count'] for i in range(1, 10)], [0] * 9, source=RECIPES + 'oak_stairs.json')


def shapeless(ctx):
    for slots in ((1, 2), (7, 3)):
        reset(ctx)
        grid(ctx, [(slots[0], 'red_dye', 1), (slots[1], 'blue_dye', 1)])
        take(ctx, 'purple_dye', 2, RECIPES + 'purple_dye.json')
        check(ctx, 'dye_grid_consumed', [menu_slot(ctx, i)['count'] for i in range(1, 10)], [0] * 9, source=RECIPES + 'purple_dye.json')
    reset(ctx)
    grid(ctx, [(8, 'oak_log', 1)])
    take(ctx, 'oak_planks', 4, RECIPES + 'oak_planks.json')
    check(ctx, 'planks_grid_consumed', [menu_slot(ctx, i)['count'] for i in range(1, 10)], [0] * 9, source=RECIPES + 'oak_planks.json')
    reset(ctx)
    grid(ctx, [(1, 'paper', 1), (4, 'paper', 1), (9, 'paper', 1), (2, 'leather', 1)])
    take(ctx, 'book', 1, RECIPES + 'book.json')
    check(ctx, 'book_grid_consumed', [menu_slot(ctx, i)['count'] for i in range(1, 10)], [0] * 9, source=RECIPES + 'book.json')


def craft_max(ctx):
    reset(ctx)
    grid(ctx, [(1, 'oak_log', 17)])
    take(ctx, 'oak_planks', 68, RECIPES + 'oak_planks.json', shift=True)
    check(ctx, 'shift_craft_consumes_17_logs', menu_slot(ctx, 1)['count'], 0, source=RECIPES + 'oak_planks.json')
    reset(ctx)
    grid(ctx, [(1, 'oak_log', 17)])
    give(ctx, 'dirt', 35 * 64)
    give(ctx, 'oak_planks', 60)
    take(ctx, 'oak_planks', 4, RECIPES + 'oak_planks.json', shift=True)
    check(ctx, 'space_limited_shift_leaves_inputs', menu_slot(ctx, 1)['count'], 16, source=RECIPES + 'oak_planks.json')


def component(stack, key):
    if 'components' not in stack:
        raise MissingCapability('snapshot:stack:components')
    return stack['components'].get('minecraft:' + key)


def registry_name(ctx, registry, value):
    if isinstance(value, str) and value.startswith('minecraft:'):
        return value
    mappings = observe(ctx, 'registries')
    name = mappings.get(registry, {}).get(str(value))
    if name is None:
        raise MissingCapability(f'snapshot:registries:{registry}:{value}')
    return name


def named_enchantments(ctx, stack):
    value = component(stack, 'enchantments')
    if value is None:
        return {}
    entries = value.items() if isinstance(value, dict) else value
    return {registry_name(ctx, 'minecraft:enchantment', key): level for key, level in entries}


def special(ctx):
    reset(ctx)
    grid(ctx, [(1, 'paper', 1), (2, 'gunpowder', 1)])
    take(ctx, 'firework_rocket', 3, RECIPES + 'firework_rocket.json')
    check(ctx, 'rocket_flight_duration', component(cursor(ctx), 'fireworks')['flight_duration'], 1, source=RECIPES + 'firework_rocket.json')
    reset(ctx)
    grid(ctx, [(1, 'gunpowder', 1), (2, 'red_dye', 1)])
    take(ctx, 'firework_star', 1, RECIPES + 'firework_star.json')
    condition(ctx, 'firework_star_has_explosion', bool(component(cursor(ctx), 'firework_explosion')), cursor(ctx), source=WIKI + 'Firework_Star')
    reset(ctx)
    grid(ctx, [(1, 'leather_chestplate', 1), (2, 'red_dye', 1)])
    take(ctx, 'leather_chestplate', 1, WIKI + 'Dye')
    check(ctx, 'red_leather_color', component(cursor(ctx), 'dyed_color'), 11546150, source=WIKI + 'Dye')
    reset(ctx)
    grid(ctx, [(1, 'red_mushroom', 1), (2, 'brown_mushroom', 1), (3, 'bowl', 1), (4, 'dandelion', 1)])
    take(ctx, 'suspicious_stew', 1, WIKI + 'Suspicious_Stew')
    condition(ctx, 'stew_contains_saturation', 'saturation' in str(component(cursor(ctx), 'suspicious_stew_effects')), cursor(ctx), source=WIKI + 'Suspicious_Stew')
    reset(ctx)
    grid(ctx, [(1, 'white_banner[banner_patterns=[{pattern:"minecraft:stripe_bottom",color:"red"}]]', 1), (2, 'white_banner', 1)])
    take(ctx, 'white_banner', 1, WIKI + 'Banner')
    condition(ctx, 'banner_pattern_copied', 'stripe_bottom' in str(component(cursor(ctx), 'banner_patterns')), cursor(ctx), source=WIKI + 'Banner')
    reset(ctx)
    grid(ctx, [(1, 'written_book[written_book_content={title:"Matrix",author:"Matrix",pages:["Page"],generation:0}]', 1), (2, 'writable_book', 1)])
    take(ctx, 'written_book', 1, RECIPES + 'book_cloning.json')
    check(ctx, 'book_generation_increment', component(cursor(ctx), 'written_book_content')['generation'], 1, source=RECIPES + 'book_cloning.json')
    # A map must first be allocated through real use, not a fabricated map id.
    require_verbs(ctx, 'use_item', 'close_container')
    verb(ctx, 'close_container')
    give(ctx, 'map')
    select(ctx, 'map')
    verb(ctx, 'use_item', hand='main')
    open_menu(ctx, 'crafting_table')
    give(ctx, 'paper', 8)
    put(ctx, 'filled_map', 5)
    for slot in (1, 2, 3, 4, 6, 7, 8, 9):
        put(ctx, 'paper', slot, 1)
    take(ctx, 'filled_map', 1, WIKI + 'Map')
    condition(ctx, 'map_extension_component', component(cursor(ctx), 'map_id') is not None, cursor(ctx), source=WIKI + 'Map')


def limited(ctx):
    reset(ctx)
    command(ctx, 'gamerule limited_crafting true')
    command(ctx, f'advancement revoke {client(ctx).name} everything')
    grid(ctx, [(1, 'oak_log', 1)])
    # Recipe knowledge, not an output packet alone, is the relevant precondition.
    known = observe(ctx, 'recipes')
    check(ctx, 'recipe_locked_fixture', 'minecraft:oak_planks' in known, False, source=WIKI + 'Game_rule')
    check(ctx, 'locked_recipe_has_no_result', menu_slot(ctx, 0)['count'], 0, source=WIKI + 'Game_rule')
    command(ctx, 'gamerule limited_crafting false')
    verb(ctx, 'click_slot', 1)
    verb(ctx, 'click_slot', 1)
    check(ctx, 'unlimited_recipe_available', menu_slot(ctx, 0)['count'], 4, source=RECIPES + 'oak_planks.json')


def smelting(ctx):
    for fuel in ('coal', 'lava_bucket', 'oak_planks'):
        reset(ctx, 'furnace')
        give(ctx, 'iron_ore')
        give(ctx, fuel)
        put(ctx, 'iron_ore', 0)
        put(ctx, fuel, 1)
        ticks(ctx, 202)
        check(ctx, fuel + '_smelts_iron', menu_slot(ctx, 2).get('item_id'), 'minecraft:iron_ingot', source=RECIPES + 'iron_ingot_from_smelting_iron_ore.json')
        if fuel == 'lava_bucket':
            check(ctx, 'lava_returns_bucket', menu_slot(ctx, 1).get('item_id'), 'minecraft:bucket', source=WIKI + 'Smelting')
        before = number(ctx, client(ctx).name, 'XpTotal')
        verb(ctx, 'click_slot', 2)
        ticks(ctx, 10)
        xp = number(ctx, client(ctx).name, 'XpTotal') - before
        condition(ctx, 'iron_xp_rounding', xp in (0, 1), xp, source=RECIPES + 'iron_ingot_from_smelting_iron_ore.json')


def campfire(ctx):
    give(ctx, 'beef', 4)
    command(ctx, 'setblock 2 180 0 campfire[lit=true]')
    select(ctx, 'beef')
    for _ in range(4):
        verb(ctx, 'use_item_on', (2, 180, 0), face='up')
    check(ctx, 'four_campfire_inputs_debited', count(ctx, 'beef'), 0 if ctx.factors['game_mode'] != 'creative' else 4, source=RECIPES + 'cooked_beef_from_campfire_cooking.json')
    ticks(ctx, 605)
    cooked = count(ctx, 'cooked_beef') + sum(e['item']['count'] for e in entities(ctx, 'item') if e['item']['item_id'] == 'minecraft:cooked_beef')
    check(ctx, 'four_campfire_outputs', cooked, 4, source=RECIPES + 'cooked_beef_from_campfire_cooking.json')


def stonecutting(ctx):
    require_verbs(ctx, 'container_button')
    reset(ctx, 'stonecutter')
    give(ctx, 'stone', 3)
    put(ctx, 'stone', 0)
    recipes = observe(ctx, 'stonecutter_recipes')
    button = next(i for i, r in enumerate(recipes) if r['result']['item_id'] == 'minecraft:stone_slab')
    verb(ctx, 'container_button', button=button)
    check(ctx, 'stone_slab_result', menu_slot(ctx, 1)['count'], 2, source=RECIPES + 'stone_slab_from_stone_stonecutting.json')
    verb(ctx, 'click_slot', 1)
    check(ctx, 'stonecutting_single_input_debit', menu_slot(ctx, 0)['count'], 2, source=RECIPES + 'stone_slab_from_stone_stonecutting.json')
    check(ctx, 'stonecutting_cursor_item', cursor(ctx)['item_id'], 'minecraft:stone_slab', source=RECIPES + 'stone_slab_from_stone_stonecutting.json')
    check(ctx, 'stonecutting_cursor_count', cursor(ctx)['count'], 2, source=RECIPES + 'stone_slab_from_stone_stonecutting.json')


def smithing(ctx, trim=False):
    reset(ctx, 'smithing_table')
    item = 'diamond_chestplate' if trim else 'diamond_pickaxe'
    template = 'sentry_armor_trim_smithing_template' if trim else 'netherite_upgrade_smithing_template'
    material = 'iron_ingot' if trim else 'netherite_ingot'
    give(ctx, template)
    give(ctx, item + '[damage=17,custom_name="Matrix",enchantments={"minecraft:unbreaking":1}]')
    give(ctx, material)
    put(ctx, template, 0)
    put(ctx, item, 1)
    put(ctx, material, 2)
    verb(ctx, 'click_slot', 3)
    output = cursor(ctx)
    check(ctx, 'smithing_result', output['item_id'], 'minecraft:' + (item if trim else 'netherite_pickaxe'), source=WIKI + 'Smithing')
    check(ctx, 'smithing_retains_damage', component(output, 'damage'), 17, source=WIKI + 'Smithing')
    condition(ctx, 'smithing_retains_name_enchant', 'Matrix' in str(component(output, 'custom_name')) and named_enchantments(ctx, output).get('minecraft:unbreaking') == 1, output, source=WIKI + 'Smithing')
    check(ctx, 'smithing_inputs_consumed', [menu_slot(ctx, i)['count'] for i in range(3)], [0, 0, 0], source=WIKI + 'Smithing')
    if trim:
        check(ctx, 'smithing_trim_component', component(output, 'trim'), {'material': 'minecraft:iron', 'pattern': 'minecraft:sentry'}, source=WIKI + 'Armor_Trim')


def brewing(ctx):
    reset(ctx, 'brewing_stand')
    give(ctx, 'potion[potion_contents={potion:"minecraft:water"}]')
    give(ctx, 'blaze_powder', 2)
    put(ctx, 'potion', 0)
    put(ctx, 'blaze_powder', 4, 1)
    for ingredient, expected, item in (('nether_wart', 'awkward', 'potion'), ('blaze_powder', 'strength', 'potion'), ('gunpowder', 'strength', 'splash_potion'), ('dragon_breath', 'strength', 'lingering_potion')):
        if ingredient != 'blaze_powder':
            give(ctx, ingredient)
        put(ctx, ingredient, 3, 1)
        ticks(ctx, 402)
        stack = menu_slot(ctx, 0)
        check(ctx, 'brewed_' + ingredient, stack.get('item_id'), 'minecraft:' + item, source=WIKI + 'Brewing')
        check(ctx, 'potion_' + expected, registry_name(ctx, 'minecraft:potion', component(stack, 'potion_contents')['potion']), 'minecraft:' + expected, source=WIKI + 'Brewing')
    check(ctx, 'dragon_breath_bottle_return', menu_slot(ctx, 3).get('item_id'), 'minecraft:glass_bottle', source=WIKI + 'Brewing')
    for ingredient, expected in (('redstone', 'long_strength'), ('glowstone_dust', 'strong_strength')):
        reset(ctx, 'brewing_stand')
        give(ctx, 'potion[potion_contents={potion:"minecraft:strength"}]')
        give(ctx, 'blaze_powder')
        give(ctx, ingredient)
        put(ctx, 'potion', 0)
        put(ctx, 'blaze_powder', 4)
        put(ctx, ingredient, 3)
        ticks(ctx, 402)
        check(ctx, 'brewing_modifier_' + ingredient,
              registry_name(ctx, 'minecraft:potion', component(menu_slot(ctx, 0), 'potion_contents')['potion']),
              'minecraft:' + expected, source=WIKI + 'Brewing')


def enchanting(ctx):
    require_verbs(ctx, 'container_button')
    for shelves in (0, 8, 15):
        reset(ctx, 'enchanting_table')
        positions = [(x, 180, z) for x in range(0, 5) for z in range(-2, 3) if max(abs(x - 2), abs(z)) == 2]
        for i, (x, y, z) in enumerate(positions):
            command(ctx, f'setblock {x} {y} {z} ' + ('bookshelf' if i < shelves else 'air'))
        give(ctx, 'diamond_pickaxe')
        give(ctx, 'lapis_lazuli', 3)
        command(ctx, f'experience set {client(ctx).name} 40 levels')
        put(ctx, 'diamond_pickaxe', 0)
        put(ctx, 'lapis_lazuli', 1)
        offers = observe(ctx, 'enchanting_offers')
        condition(ctx, f'enchant_offers_{shelves}_shelves', len(offers) == 3 and all(o['level'] > 0 for o in offers), offers, source=WIKI + 'Enchanting_mechanics')
        if shelves == 15:
            check(ctx, 'max_bookshelf_third_offer_30', offers[2]['level'], 30, source=WIKI + 'Enchanting_mechanics')
        verb(ctx, 'container_button', button=2)
        condition(ctx, 'item_is_enchanted', bool(component(menu_slot(ctx, 0), 'enchantments')), menu_slot(ctx, 0), source=WIKI + 'Enchanting')
        if ctx.factors['game_mode'] != 'creative':
            check(ctx, 'lapis_payment', menu_slot(ctx, 1)['count'], 0, source=WIKI + 'Enchanting')
            check(ctx, 'level_payment', number(ctx, client(ctx).name, 'XpLevel'), 37, source=WIKI + 'Enchanting')


def anvil(ctx):
    require_verbs(ctx, 'rename_item')
    reset(ctx, 'anvil')
    give(ctx, 'diamond_pickaxe[damage=100]')
    command(ctx, f'experience set {client(ctx).name} 40 levels')
    put(ctx, 'diamond_pickaxe', 0)
    verb(ctx, 'rename_item', text='MatrixPick')
    check(ctx, 'rename_cost_one', observe(ctx, 'anvil_cost'), 1, source=WIKI + 'Anvil_mechanics')
    give(ctx, 'diamond')
    put(ctx, 'diamond', 1)
    verb(ctx, 'click_slot', 2)
    check(ctx, 'material_repairs_quarter_durability', component(cursor(ctx), 'damage') or 0, 0, source=WIKI + 'Anvil_mechanics')
    condition(ctx, 'rename_retained', 'MatrixPick' in str(component(cursor(ctx), 'custom_name')), cursor(ctx), source=WIKI + 'Anvil_mechanics')
    condition(ctx, 'prior_work_penalty_increased', component(cursor(ctx), 'repair_cost') > 0, cursor(ctx), source=WIKI + 'Anvil_mechanics')
    reset(ctx, 'anvil')
    give(ctx, 'diamond_pickaxe[enchantments={"minecraft:unbreaking":1}]')
    give(ctx, 'enchanted_book[stored_enchantments={"minecraft:unbreaking":1}]')
    put(ctx, 'diamond_pickaxe', 0)
    put(ctx, 'enchanted_book', 1)
    verb(ctx, 'click_slot', 2)
    check(ctx, 'anvil_combines_equal_enchantment_levels',
          named_enchantments(ctx, cursor(ctx)), {'minecraft:unbreaking': 2},
          source=WIKI + 'Anvil_mechanics')


def grindstone(ctx):
    reset(ctx, 'grindstone')
    give(ctx, 'diamond_chestplate[enchantments={"minecraft:protection":2,"minecraft:binding_curse":1}]')
    put(ctx, 'diamond_chestplate', 0)
    before = number(ctx, client(ctx).name, 'XpTotal')
    verb(ctx, 'click_slot', 2)
    enchantments = named_enchantments(ctx, cursor(ctx))
    check(ctx, 'grindstone_retains_only_curse', enchantments, {'minecraft:binding_curse': 1}, source=WIKI + 'Grindstone')
    ticks(ctx, 10)
    condition(ctx, 'grindstone_awards_xp',
              number(ctx, client(ctx).name, 'XpTotal') > before,
              number(ctx, client(ctx).name, 'XpTotal') - before,
              source=WIKI + 'Grindstone')


def loom(ctx):
    require_verbs(ctx, 'container_button')
    reset(ctx, 'loom')
    give(ctx, 'white_banner')
    give(ctx, 'red_dye')
    put(ctx, 'white_banner', 0)
    put(ctx, 'red_dye', 1)
    without = observe(ctx, 'loom_patterns')
    condition(ctx, 'creeper_requires_pattern_item', 'minecraft:creeper' not in without, without, source=WIKI + 'Loom')
    give(ctx, 'creeper_banner_pattern')
    put(ctx, 'creeper_banner_pattern', 2)
    patterns = observe(ctx, 'loom_patterns')
    check(ctx, 'pattern_item_unlocks_creeper', patterns, ['minecraft:creeper'], source=WIKI + 'Loom')
    verb(ctx, 'container_button', button=0)
    verb(ctx, 'click_slot', 3)
    condition(ctx, 'creeper_banner_output', 'creeper' in str(component(cursor(ctx), 'banner_patterns')), cursor(ctx), source=WIKI + 'Loom')
    check(ctx, 'pattern_item_not_consumed', menu_slot(ctx, 2)['count'], 1, source=WIKI + 'Loom')
    check(ctx, 'loom_banner_and_dye_consumed', [menu_slot(ctx, i)['count'] for i in (0, 1)], [0, 0], source=WIKI + 'Loom')


def cartography(ctx):
    require_verbs(ctx, 'use_item', 'close_container')
    give(ctx, 'map')
    select(ctx, 'map')
    verb(ctx, 'use_item', hand='main')
    for ingredient in ('paper', 'map', 'glass_pane'):
        open_menu(ctx, 'cartography_table')
        give(ctx, ingredient)
        put(ctx, 'filled_map', 0, 1)
        put(ctx, ingredient, 1)
        verb(ctx, 'click_slot', 2)
        check(ctx, 'cartography_output_count_' + ingredient, cursor(ctx)['count'], 2 if ingredient == 'map' else 1, source=WIKI + 'Cartography_Table')
        state = observe(ctx, 'map_data')
        if ingredient == 'paper':
            check(ctx, 'map_scale_increment', state['scale'], 1, source=WIKI + 'Map')
        elif ingredient == 'glass_pane':
            check(ctx, 'map_locked', state['locked'], True, source=WIKI + 'Map')
        verb(ctx, 'close_container')


def crafter(ctx):
    require_verbs(ctx, 'toggle_crafter_slot', 'close_container')
    open_menu(ctx, 'crafter')
    give(ctx, 'oak_log', 2)
    put(ctx, 'oak_log', 0)
    for slot in range(1, 9):
        verb(ctx, 'toggle_crafter_slot', slot=slot, disabled=True)
    verb(ctx, 'close_container')
    command(ctx, 'setblock 2 180 1 comparator[facing=south]')
    command(ctx, 'setblock 2 180 2 redstone_wire')
    ticks(ctx, 4)
    check(ctx, 'crafter_comparator_counts_disabled_and_filled',
          block_is(ctx, (2, 180, 2), 'redstone_wire[power=9]'), True,
          source=WIKI + 'Crafter')
    command(ctx, 'setblock 3 180 0 redstone_block')
    ticks(ctx, 8)
    command(ctx, 'setblock 3 180 0 air')
    ticks(ctx, 8)
    total = sum(e['item']['count'] for e in entities(ctx, 'item') if e['item']['item_id'] == 'minecraft:oak_planks')
    check(ctx, 'single_pulse_single_craft', total, 4, source=WIKI + 'Crafter')
    verb(ctx, 'open_container', (2, 180, 0))
    check(ctx, 'crafter_retains_second_log', menu_slot(ctx, 0)['count'], 1, source=WIKI + 'Crafter')


from functools import partial
ADAPTERS = {f'crafting.{name}': adapter(fn) for name, fn in {
    'shaped': shaped, 'shapeless': shapeless, 'craft_max_shift': craft_max,
    'special_recipes': special, 'limited_crafting': limited, 'smelting_fuel_xp': smelting,
    'campfire_passive': campfire, 'stonecutting': stonecutting,
    'smithing_netherite': smithing, 'smithing_trim': partial(smithing, trim=True),
    'brewing_chain': brewing, 'enchanting_offers': enchanting,
    'anvil_repair_combine_rename': anvil, 'grindstone_disenchant': grindstone,
    'loom_patterns': loom, 'cartography_ops': cartography, 'crafter_automation': crafter,
}.items()}
