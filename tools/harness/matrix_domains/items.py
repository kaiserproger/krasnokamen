"""Actual item-use/custody scenarios. Effects/components require real observations."""
from ..matrix_api import MissingCapability
from ._items import (
    adapter, block_is, check, client, command, condition, consume, count, cursor,
    entities, give, health, inventory_slot, menu_slot, number, observe, open_menu,
    player_menu_slot, put, query, require_verbs, select, summon, survival, ticks, verb, WIKI,
)
from .crafting import component


def effects(ctx):
    return {e['id']: e for e in observe(ctx, 'effects')}


def eat(ctx):
    survival(ctx)
    require_verbs(ctx, 'use_item', 'release_use_item')
    # Force hunger through the real effect rather than modifying player NBT.
    for item, food, seconds in (('apple', 4, 32), ('cooked_beef', 8, 32),
                                ('golden_apple', 4, 32), ('chorus_fruit', 4, 32),
                                ('rotten_flesh', 4, 32), ('pufferfish', 1, 32)):
        command(ctx, f'effect give {client(ctx).name} hunger 20 255')
        ticks(ctx, 140)
        command(ctx, f'effect clear {client(ctx).name}')
        before = number(ctx, client(ctx).name, 'foodLevel')
        give(ctx, item)
        position = observe(ctx, 'position')
        consume(ctx, item, seconds)
        check(ctx, item + '_hunger_points', number(ctx, client(ctx).name, 'foodLevel'), min(20, before + food), source=WIKI + 'Food')
        check(ctx, item + '_consumed', count(ctx, item), 0, source=WIKI + 'Food')
        if item == 'golden_apple':
            condition(ctx, 'golden_apple_effects', {'minecraft:regeneration', 'minecraft:absorption'} <= effects(ctx).keys(), effects(ctx), source=WIKI + 'Golden_Apple')
        elif item == 'pufferfish':
            condition(ctx, 'pufferfish_effects', {'minecraft:poison', 'minecraft:hunger', 'minecraft:nausea'} <= effects(ctx).keys(), effects(ctx), source=WIKI + 'Pufferfish')
        elif item == 'chorus_fruit':
            condition(ctx, 'chorus_teleport', observe(ctx, 'position') != position, {'before': position, 'after': observe(ctx, 'position')}, source=WIKI + 'Chorus_Fruit')


def drinks(ctx):
    give(ctx, 'potion[potion_contents={potion:"minecraft:swiftness"}]')
    consume(ctx, 'potion')
    condition(ctx, 'potion_applies_speed', 'minecraft:speed' in effects(ctx), effects(ctx), source=WIKI + 'Potion')
    check(ctx, 'potion_returns_bottle', count(ctx, 'glass_bottle'), 1, source=WIKI + 'Potion')
    give(ctx, 'milk_bucket')
    consume(ctx, 'milk_bucket')
    check(ctx, 'milk_removes_all_effects', effects(ctx), {}, source=WIKI + 'Milk_Bucket')
    check(ctx, 'milk_returns_bucket', count(ctx, 'bucket'), 1, source=WIKI + 'Milk_Bucket')
    command(ctx, f'effect give {client(ctx).name} poison 30 0')
    command(ctx, f'effect give {client(ctx).name} speed 30 0')
    give(ctx, 'honey_bottle')
    bottles = count(ctx, 'glass_bottle')
    consume(ctx, 'honey_bottle', 40)
    check(ctx, 'honey_clears_poison', 'minecraft:poison' in effects(ctx), False, source=WIKI + 'Honey_Bottle')
    check(ctx, 'honey_keeps_other_effects', 'minecraft:speed' in effects(ctx), True, source=WIKI + 'Honey_Bottle')
    check(ctx, 'honey_returns_bottle', count(ctx, 'glass_bottle'), bottles + 1, source=WIKI + 'Honey_Bottle')
    give(ctx, 'ominous_bottle')
    consume(ctx, 'ominous_bottle')
    check(ctx, 'ominous_bottle_bad_omen', 'minecraft:bad_omen' in effects(ctx), True, source=WIKI + 'Ominous_Bottle')


def throwables(ctx):
    require_verbs(ctx, 'use_item')
    for item, entity in (('snowball', 'snowball'), ('egg', 'egg'), ('experience_bottle', 'experience_bottle'),
                         ('splash_potion[potion_contents={potion:"minecraft:poison"}]', 'potion'),
                         ('lingering_potion[potion_contents={potion:"minecraft:poison"}]', 'potion'),
                         ('ender_pearl', 'ender_pearl')):
        base = item.split('[', 1)[0]
        give(ctx, item, 2)
        select(ctx, base)
        client(ctx).look_at(8, 190, 0)
        verb(ctx, 'use_item', hand='main')
        check(ctx, base + '_debit', count(ctx, base), 2 if ctx.factors['game_mode'] == 'creative' else 1, source=WIKI + base.title())
        projectiles = observe(ctx, 'projectile_events')
        condition(ctx, base + '_projectile_spawn', any(e['type'] == 'minecraft:' + entity for e in projectiles), projectiles, source=WIKI + 'Projectile')
        ticks(ctx, 80)
        if base == 'lingering_potion':
            condition(ctx, 'lingering_cloud_created', bool(entities(ctx, 'area_effect_cloud')), entities(ctx, 'area_effect_cloud'), source=WIKI + 'Lingering_Potion')
        elif base == 'experience_bottle':
            condition(ctx, 'xp_bottle_orbs', bool(entities(ctx, 'experience_orb')) or number(ctx, client(ctx).name, 'XpTotal') > 0, observe(ctx, 'entities'), source=WIKI + 'Bottle_o%27_Enchanting')
        elif base == 'ender_pearl':
            condition(ctx, 'pearl_teleport', observe(ctx, 'position')[0] > 1, observe(ctx, 'position'), source=WIKI + 'Ender_Pearl')


def durability(ctx):
    survival(ctx)
    require_verbs(ctx, 'break_block')
    give(ctx, 'diamond_pickaxe')
    select(ctx, 'diamond_pickaxe')
    command(ctx, 'setblock 2 180 0 stone')
    verb(ctx, 'break_block', position=(2, 180, 0), face='up', phase='complete')
    tool = next(s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:diamond_pickaxe')
    check(ctx, 'ordinary_tool_wear_one', component(tool, 'damage'), 1, source=WIKI + 'Durability')
    command(ctx, f'clear {client(ctx).name}')
    give(ctx, 'diamond_pickaxe[enchantments={"minecraft:unbreaking":3,"minecraft:mending":1}]')
    select(ctx, 'diamond_pickaxe')
    for _ in range(64):
        command(ctx, 'setblock 2 180 0 stone')
        verb(ctx, 'break_block', position=(2, 180, 0), face='up', phase='complete')
    tool = next(s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:diamond_pickaxe')
    worn = component(tool, 'damage') or 0
    condition(ctx, 'unbreaking_III_reduces_wear', 0 <= worn < 64, worn, source='local:data/vanilla/data/minecraft/enchantment/unbreaking.json')
    command(ctx, 'summon experience_orb 0.5 180 0.5 {Value:100s}')
    ticks(ctx, 20)
    tool = next(s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:diamond_pickaxe')
    check(ctx, 'mending_repairs_damage', component(tool, 'damage') or 0, 0, source='local:data/vanilla/data/minecraft/enchantment/mending.json')


def shield(ctx):
    from .combat import shield_vs_axe
    shield_vs_axe(ctx)


def bundle(ctx):
    give(ctx, 'bundle')
    give(ctx, 'stone', 63)
    give(ctx, 'dirt', 2)
    open_menu(ctx)
    bundle_slot = player_menu_slot(ctx, inventory_slot(ctx, 'bundle'))
    stone_slot = player_menu_slot(ctx, inventory_slot(ctx, 'stone'))
    verb(ctx, 'click_slot', stone_slot)
    verb(ctx, 'click_slot', bundle_slot, button=1)
    check(ctx, 'bundle_accepts_63', cursor(ctx)['count'], 0, source=WIKI + 'Bundle')
    dirt_slot = player_menu_slot(ctx, inventory_slot(ctx, 'dirt'))
    verb(ctx, 'click_slot', dirt_slot)
    verb(ctx, 'click_slot', bundle_slot, button=1)
    check(ctx, 'bundle_capacity_64', cursor(ctx)['count'], 1, source=WIKI + 'Bundle')
    verb(ctx, 'click_slot', dirt_slot)
    verb(ctx, 'click_slot', bundle_slot, button=1)
    check(ctx, 'bundle_last_inserted_first', cursor(ctx).get('item_id'), 'minecraft:dirt', source=WIKI + 'Bundle')
    check(ctx, 'bundle_last_insert_count', cursor(ctx)['count'], 1, source=WIKI + 'Bundle')


def books(ctx):
    require_verbs(ctx, 'edit_book', 'use_item')
    give(ctx, 'writable_book')
    slot = inventory_slot(ctx, 'writable_book')
    verb(ctx, 'edit_book', slot=slot, pages=['Matrix first page', 'Matrix second page'], title=None)
    book = next(s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:writable_book')
    condition(ctx, 'written_pages_persist', 'Matrix first page' in str(component(book, 'writable_book_content')), book, source=WIKI + 'Book_and_Quill')
    verb(ctx, 'edit_book', slot=slot, pages=['Matrix first page', 'Matrix second page'], title='MatrixBook')
    check(ctx, 'signed_book_conversion', count(ctx, 'written_book'), 1, source=WIKI + 'Written_Book')
    book = next(s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:written_book')
    before = component(book, 'written_book_content')
    check(ctx, 'signed_book_title', before['title'], 'MatrixBook', source=WIKI + 'Written_Book')
    condition(ctx, 'signed_book_pages', all(page in str(before['pages']) for page in ('Matrix first page', 'Matrix second page')), before, source=WIKI + 'Written_Book')
    verb(ctx, 'edit_book', slot=slot, pages=['Tampered'], title='Other')
    after = component(next(s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:written_book'), 'written_book_content')
    check(ctx, 'signed_book_immutable', after, before, source=WIKI + 'Written_Book')
    select(ctx, 'written_book')
    verb(ctx, 'use_item', hand='main')
    check(ctx, 'book_read_screen', observe(ctx, 'screen'), 'book', source=WIKI + 'Written_Book')


def maps(ctx):
    require_verbs(ctx, 'use_item', 'move')
    give(ctx, 'map')
    select(ctx, 'map')
    verb(ctx, 'use_item', hand='main')
    check(ctx, 'empty_map_converted', count(ctx, 'filled_map'), 1, source=WIKI + 'Map')
    before = observe(ctx, 'map_data')
    select(ctx, 'filled_map')
    verb(ctx, 'move', forward=1, strafe=0, ticks=100, sprint=True)
    after = observe(ctx, 'map_data')
    condition(ctx, 'map_updates_pixels', after['pixels'] != before['pixels'], {'before': before, 'after': after}, source=WIKI + 'Map')
    condition(ctx, 'map_has_player_marker', bool(after['decorations']), after['decorations'], source=WIKI + 'Map')
    check(ctx, 'new_map_scale_zero', after['scale'], 0, source=WIKI + 'Map')


def name_tag(ctx):
    require_verbs(ctx, 'rename_item', 'interact_entity', 'close_container')
    give(ctx, 'name_tag')
    command(ctx, f'experience set {client(ctx).name} 10 levels')
    open_menu(ctx, 'anvil')
    put(ctx, 'name_tag', 0)
    verb(ctx, 'rename_item', text='MatrixNamedCow')
    verb(ctx, 'click_slot', 2)
    verb(ctx, 'close_container')
    target_id = summon(ctx, 'cow', pos='3.5 180 0.5')
    select(ctx, 'name_tag')
    verb(ctx, 'interact_entity', entity_id=target_id, hand='main')
    result = query(ctx, 'data get entity @e[type=cow,limit=1] CustomName')
    condition(ctx, 'tag_name_applied', 'MatrixNamedCow' in result, result, source=WIKI + 'Name_Tag')
    check(ctx, 'tag_persistence_required', number(ctx, '@e[type=cow,limit=1]', 'PersistenceRequired'), 1, source=WIKI + 'Name_Tag')


def fishing(ctx):
    require_verbs(ctx, 'use_item')
    command(ctx, 'fill 1 178 -3 9 179 3 water')
    give(ctx, 'fishing_rod')
    select(ctx, 'fishing_rod')
    client(ctx).look_at(5, 179.5, 0)
    verb(ctx, 'use_item', hand='main')
    ticks(ctx, 2)
    verb(ctx, 'use_item', hand='main')
    check(ctx, 'early_reel_no_fish', sum(count(ctx, i) for i in ('cod', 'salmon', 'pufferfish', 'tropical_fish')), 0, source=WIKI + 'Fishing')
    ticks(ctx, 6)
    verb(ctx, 'use_item', hand='main')
    ctx.wait_until(lambda: observe(ctx, 'fishing_bite'), description='server fishing bite event', timeout_seconds=90)
    verb(ctx, 'use_item', hand='main')
    ticks(ctx, 20)
    loot = observe(ctx, 'fishing_loot')
    condition(ctx, 'bite_reel_grants_loot', bool(loot), loot, source='local:data/vanilla/data/minecraft/loot_table/gameplay/fishing.json')
    rod = next(s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:fishing_rod')
    condition(ctx, 'fishing_rod_wear', (component(rod, 'damage') or 0) >= 1, rod, source=WIKI + 'Fishing_Rod')


def spawn_eggs(ctx):
    for item, mob, pos in (('cow_spawn_egg', 'cow', (3, 179, 0)), ('cod_spawn_egg', 'cod', (3, 179, 3))):
        if mob == 'cod':
            command(ctx, 'fill 2 179 2 4 181 4 water')
        give(ctx, item, 2)
        select(ctx, item)
        before = len(entities(ctx, mob))
        verb(ctx, 'use_item_on', pos, face='up')
        ticks(ctx, 2)
        check(ctx, mob + '_spawned', len(entities(ctx, mob)) - before, 1, source=WIKI + 'Spawn_Egg')
        check(ctx, mob + '_egg_debit', count(ctx, item), 2 if ctx.factors['game_mode'] == 'creative' else 1, source=WIKI + 'Spawn_Egg')


def components_roundtrip(ctx):
    require_verbs(ctx, 'close_container')
    give(ctx, 'leather_chestplate[custom_name="Matrix",enchantments={"minecraft:unbreaking":1},dyed_color=11546150,damage=17,lore=["Matrix lore"]]')
    before = component(next(s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:leather_chestplate'), 'custom_name')
    original = next(s['components'] for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:leather_chestplate')
    open_menu(ctx)
    put(ctx, 'leather_chestplate', 0)
    check(ctx, 'container_components', menu_slot(ctx, 0)['components'], original, source=WIKI + 'Data_component_format')
    verb(ctx, 'click_slot', 0, kind='quick_move')
    verb(ctx, 'close_container')
    select(ctx, 'leather_chestplate')
    verb(ctx, 'drop', all=True)
    ticks(ctx, 45)
    command(ctx, f'tp {client(ctx).name} 0.5 180 1.5')
    ticks(ctx, 5)
    ctx.server.save_all()
    ctx.server.kill9_restart()
    client(ctx).reconnect()
    restored = next(s['components'] for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:leather_chestplate')
    check(ctx, 'drop_restart_component_roundtrip', restored, original, source=WIKI + 'Data_component_format')


def despawn(ctx):
    # Start from age 5980 so this tests the actual 6000 threshold without a
    # wall-clock sleep. This is a vanilla summon field, not a test-only timer.
    command(ctx, 'summon item 4.5 180 0.5 {Item:{id:"minecraft:stone",count:1},Age:5980s,PickupDelay:32767s}')
    ticks(ctx, 2)
    condition(ctx, 'item_exists_before_6000', bool(entities(ctx, 'item')), entities(ctx, 'item'), source='local:scenarios/runs/DomItems/facts/ItemEntity.javap')
    ticks(ctx, 22)
    check(ctx, 'item_removed_at_6000', len(entities(ctx, 'item')), 0, source='local:scenarios/runs/DomItems/facts/ItemEntity.javap')


def hazards(ctx):
    for hazard in ('lava', 'fire', 'cactus'):
        command(ctx, 'kill @e[type=item]')
        command(ctx, 'setblock 4 179 0 sand')
        command(ctx, f'setblock 4 180 0 {hazard}')
        for item in ('diamond', 'netherite_ingot'):
            command(ctx, f'summon item 4.5 180 0.5 {{Item:{{id:"minecraft:{item}",count:1}},PickupDelay:32767s}}')
        ticks(ctx, 120)
        survivors = [e['item']['item_id'] for e in entities(ctx, 'item')]
        check(ctx, hazard + '_diamond_destroyed', 'minecraft:diamond' in survivors, False, source=WIKI + 'Item_(entity)')
        check(ctx, hazard + '_netherite_rule', 'minecraft:netherite_ingot' in survivors, hazard != 'cactus', source=WIKI + 'Netherite_Ingot')
        command(ctx, 'setblock 4 180 0 air')
    command(ctx, 'summon item 4.5 180 0.5 {Item:{id:"minecraft:diamond",count:1},PickupDelay:32767s}')
    command(ctx, 'summon tnt 4.5 180 0.5 {fuse:1s}')
    ticks(ctx, 10)
    check(ctx, 'explosion_destroys_diamond', any(e['item']['item_id'] == 'minecraft:diamond' for e in entities(ctx, 'item')), False, source=WIKI + 'Item_(entity)')


def fireworks(ctx):
    require_verbs(ctx, 'use_item', 'press_inputs')
    give(ctx, 'firework_rocket', 2)
    select(ctx, 'firework_rocket')
    verb(ctx, 'use_item_on', (2, 179, 0), face='up')
    condition(ctx, 'ground_rocket_launch', bool(entities(ctx, 'firework_rocket')), entities(ctx, 'firework_rocket'), source=WIKI + 'Firework_Rocket')
    give(ctx, 'elytra')
    select(ctx, 'elytra')
    verb(ctx, 'use_item', hand='main')
    command(ctx, f'tp {client(ctx).name} 0.5 210 0.5')
    verb(ctx, 'press_inputs', jump=True, ticks=2)
    select(ctx, 'firework_rocket')
    before = observe(ctx, 'velocity')
    verb(ctx, 'use_item', hand='main')
    ticks(ctx, 10)
    after = observe(ctx, 'velocity')
    condition(ctx, 'rocket_boost_increases_speed', sum(v * v for v in after) > sum(v * v for v in before), {'before': before, 'after': after}, source=WIKI + 'Firework_Rocket')


def totem(ctx):
    survival(ctx)
    give(ctx, 'totem_of_undying')
    select(ctx, 'totem_of_undying')
    # /kill and the void deliberately bypass totems; harming is a legal trigger.
    command(ctx, f'effect give {client(ctx).name} instant_damage 1 4')
    check(ctx, 'totem_prevents_death', health(ctx), 1, source=WIKI + 'Totem_of_Undying')
    check(ctx, 'totem_consumed', count(ctx, 'totem_of_undying'), 0, source=WIKI + 'Totem_of_Undying')
    actual = effects(ctx)
    for name, amp in (('regeneration', 1), ('absorption', 1), ('fire_resistance', 0)):
        check(ctx, 'totem_' + name, actual.get('minecraft:' + name, {}).get('amplifier'), amp, source=WIKI + 'Totem_of_Undying')


def spyglass_horn(ctx):
    require_verbs(ctx, 'use_item', 'release_use_item')
    give(ctx, 'spyglass')
    select(ctx, 'spyglass')
    verb(ctx, 'use_item', hand='main', ticks=5)
    check(ctx, 'spyglass_zoom_active', observe(ctx, 'spyglass_zoom'), True, source=WIKI + 'Spyglass')
    verb(ctx, 'release_use_item')
    give(ctx, 'goat_horn[instrument="minecraft:ponder_goat_horn"]')
    select(ctx, 'goat_horn')
    verb(ctx, 'use_item', hand='main')
    condition(ctx, 'goat_horn_sound', any('goat_horn' in str(s) for s in observe(ctx, 'sounds')), observe(ctx, 'sounds'), source='local:data/vanilla/data/minecraft/instrument/ponder_goat_horn.json')
    cooldown = observe(ctx, 'cooldowns')
    condition(ctx, 'horn_cooldown', cooldown.get('minecraft:goat_horn', 0) > 0, cooldown, source=WIKI + 'Goat_Horn')


def compasses(ctx):
    require_verbs(ctx, 'respawn')
    command(ctx, f'tp {client(ctx).name} 7.5 180 7.5')
    command(ctx, f'kill {client(ctx).name}')
    verb(ctx, 'respawn')
    give(ctx, 'recovery_compass')
    select(ctx, 'recovery_compass')
    recovery = observe(ctx, 'compass_target')
    check(ctx, 'recovery_compass_last_death', recovery['position'], [7, 180, 7], source=WIKI + 'Recovery_Compass')
    give(ctx, 'compass')
    command(ctx, 'setblock 2 180 0 lodestone')
    select(ctx, 'compass')
    verb(ctx, 'use_item_on', (2, 180, 0), face='up')
    stack = next(s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:compass')
    check(ctx, 'lodestone_tracker_position', component(stack, 'lodestone_tracker')['target']['pos'], [2, 180, 0], source=WIKI + 'Lodestone_Compass')


ADAPTERS = {f'items.{name}': adapter(fn) for name, fn in {
    'eat_food': eat, 'drink_potion_milk_honey': drinks, 'throwables': throwables,
    'tool_durability_unbreaking_mending': durability, 'shield': shield, 'bundle': bundle,
    'books': books, 'maps': maps, 'name_tag': name_tag, 'fishing': fishing,
    'spawn_eggs': spawn_eggs, 'item_components_roundtrip': components_roundtrip,
    'item_despawn': despawn, 'item_hazards': hazards, 'firework_rocket': fireworks,
    'totem_of_undying': totem, 'spyglass_goat_horn': spyglass_horn,
    'recovery_compass_lodestone': compasses,
}.items()}
