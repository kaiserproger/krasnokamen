"""Farming 26.1.2 checks. Sources are attached to every receipt check.
Random growth uses a bounded high-random-tick sample, not an exact RNG oracle.
"""
from ._world import (adapter, block, check, command, condition, data, entity_field,
                     exists, give, growth, interact, inventory, observe, plant,
                     ticks, use, verb)


def wheat_carrot_potato_beetroot(ctx):
    command(ctx, 'time set noon')
    command(ctx, 'fill 6 99 6 10 99 10 farmland[moisture=7]')
    command(ctx, 'setblock 6 99 7 water')
    for crop, maximum in [('wheat', 7), ('carrots', 7), ('potatoes', 7), ('beetroots', 3)]:
        plant(ctx, 'farmland[moisture=7]', crop + '[age=0]')
        matured = growth(ctx, f'block 8 100 8 {crop}[age={maximum}]')
        check(ctx, crop + ':matures', matured, matured, True, 'Tutorial:Crop_farming')
        # An opaque sealed box has block/sky light zero at the crop.
        command(ctx, 'fill 6 100 6 10 103 10 stone')
        command(ctx, 'fill 7 100 7 9 102 9 air')
        plant(ctx, 'farmland[moisture=7]', crop + '[age=0]')
        growth(ctx, f'block 8 100 8 {crop}[age=0]')
        block(ctx, (8, 100, 8), crop + '[age=0]', source='Tutorial:Crop_farming')
        command(ctx, 'fill 6 100 6 10 103 10 air')


def stem_crops(ctx):
    for fruit in ('melon', 'pumpkin'):
        command(ctx, 'fill 7 99 7 9 99 9 dirt')
        plant(ctx, 'farmland[moisture=7]', fruit + '_stem[age=7]')
        grew = growth(ctx, f'block 8 100 8 attached_{fruit}_stem')
        check(ctx, fruit + ':attached', grew, grew, True, 'Tutorial:Pumpkin_and_melon_farming')
        adjacent = any(condition(ctx, f'block {x} 100 {z} {fruit}')
                       for x, z in [(7, 8), (9, 8), (8, 7), (8, 9)])
        check(ctx, fruit + ':adjacent-fruit', adjacent, adjacent, True, 'Tutorial:Pumpkin_and_melon_farming')
        command(ctx, 'fill 7 100 7 9 100 9 air')


def nether_wart(ctx):
    plant(ctx, 'soul_sand', 'nether_wart[age=0]')
    mature = growth(ctx, 'block 8 100 8 nether_wart[age=3]')
    check(ctx, 'nether-wart-age-three', mature, mature, True, 'Nether_Wart')


def sugar_cane_cactus_bamboo(ctx):
    for crop, support, low, high in [('sugar_cane', 'sand', 3, 3), ('cactus', 'sand', 3, 3), ('bamboo', 'dirt', 12, 16)]:
        command(ctx, 'fill 7 100 7 9 120 9 air')
        command(ctx, 'setblock 7 99 8 water')
        plant(ctx, support, crop)
        growth(ctx, f'block 8 {99+low} 8 {crop}', 1200)
        block(ctx, (8, 99 + low, 8), crop, source=crop.title().replace('_', '_'))
        block(ctx, (8, 100 + high, 8), 'air', source=crop.title())
        if crop == 'cactus':
            command(ctx, 'setblock 9 100 8 stone')
            ticks(ctx, 2)
            block(ctx, (8, 100, 8), 'air', source='Cactus')
            command(ctx, 'setblock 9 100 8 air')


def kelp_seagrass(ctx):
    command(ctx, 'fill 7 100 7 9 126 9 water')
    plant(ctx, 'sand', 'kelp[age=24]')
    growth(ctx, 'block 8 101 8 kelp[age=25]')
    block(ctx, (8, 101, 8), 'kelp[age=25]', source='Kelp')
    ticks(ctx, 100)
    block(ctx, (8, 102, 8), 'water', source='Kelp')
    ctx.clients[0].teleport_to(2.5, 100, 13.5)
    command(ctx, 'fill 3 100 16 3 101 16 water')
    plant(ctx, 'sand', 'seagrass', (3, 100, 16))
    use(ctx, (3, 100, 16), 'bone_meal')
    block(ctx, (3, 100, 16), 'tall_seagrass[half=lower]' if ctx.factors['game_mode'] != 'spectator' else 'seagrass', source='Seagrass')


def sweet_berries_cocoa_glow_berries(ctx):
    ctx.clients[0].teleport_to(2.5, 100, 13.5)
    plant(ctx, 'dirt', 'sweet_berry_bush[age=0]', (3, 100, 16))
    matured = growth(ctx, 'block 3 100 16 sweet_berry_bush[age=3]')
    check(ctx, 'sweet-berry-matures', matured, matured, True, 'Sweet_Berries')
    give(ctx, 'stick')
    use(ctx, (3, 100, 16))
    block(ctx, (3, 100, 16), 'sweet_berry_bush[age=1]' if ctx.factors['game_mode'] != 'spectator' else 'sweet_berry_bush[age=3]', source='Sweet_Berries')
    command(ctx, 'setblock 4 100 16 jungle_log')
    command(ctx, 'setblock 3 100 16 cocoa[age=0,facing=east]')
    matured = growth(ctx, 'block 3 100 16 cocoa[age=2]')
    check(ctx, 'cocoa-matures', matured, matured, True, 'Cocoa_Beans')
    command(ctx, 'setblock 3 103 16 stone')
    command(ctx, 'setblock 3 102 16 cave_vines[berries=true]')
    use(ctx, (3, 102, 16))
    block(ctx, (3, 102, 16), 'cave_vines[berries=false]' if ctx.factors['game_mode'] != 'spectator' else 'cave_vines[berries=true]', source='Glow_Berries')


def chorus(ctx):
    plant(ctx, 'end_stone', 'chorus_flower[age=0]')
    grew = growth(ctx, 'block 8 100 8 chorus_plant', 1200)
    check(ctx, 'chorus-branches-from-flower', grew, grew, True, 'Chorus_Flower')


def mushrooms_vines_spread(ctx):
    command(ctx, 'fill 5 99 5 11 99 11 mycelium')
    command(ctx, 'fill 5 104 5 11 104 11 stone')
    plant(ctx, 'mycelium', 'brown_mushroom')
    growth(ctx, 'block 8 100 8 brown_mushroom', 1200)
    spread = sum(condition(ctx, f'block {x} 100 {z} brown_mushroom')
                 for x in range(5, 12) for z in range(5, 12))
    check(ctx, 'mushroom-spread-density', 1 < spread <= 5, spread, '2..5', 'Mushroom')
    command(ctx, 'fill 15 100 8 15 110 8 stone')
    command(ctx, 'setblock 14 108 8 vine[east=true]')
    grew = growth(ctx, 'block 14 107 8 vine', 1200)
    check(ctx, 'vine-spreads-down', grew, grew, True, 'Vines')
    for vine, support, y, next_y in [('twisting_vines', 'warped_nylium', 100, 101), ('weeping_vines', 'netherrack', 108, 107)]:
        command(ctx, f'setblock 20 {y-1 if next_y>y else y+1} 8 {support}')
        command(ctx, f'setblock 20 {y} 8 {vine}[age=0]')
        grew = growth(ctx, f'block 20 {next_y} 8 {vine}', 1200)
        check(ctx, vine + ':grows', grew, grew, True, vine.title())


def saplings_trees(ctx):
    for sapling, log, square in [('oak', 'oak_log', False), ('spruce', 'spruce_log', True), ('jungle', 'jungle_log', True), ('dark_oak', 'dark_oak_log', True)]:
        command(ctx, 'fill 6 100 6 20 130 20 air')
        command(ctx, 'fill 9 99 9 12 99 12 dirt')
        command(ctx, f'setblock 10 100 10 {sapling}_sapling[stage=1]')
        if square:
            command(ctx, f'fill 10 100 10 11 100 11 {sapling}_sapling[stage=1]')
        grew = growth(ctx, f'block 10 100 10 {log}', 1200)
        check(ctx, sapling + ':tree-trunk', grew, grew, True, 'Sapling')


def grass_mycelium_spread(ctx):
    command(ctx, 'time set noon')
    for surface in ('grass_block', 'mycelium'):
        command(ctx, 'fill 7 99 7 9 99 9 dirt')
        command(ctx, f'setblock 8 99 8 {surface}')
        growth(ctx, f'block 9 99 8 {surface}', 1200)
        block(ctx, (9, 99, 8), surface, source='Grass_Block' if surface == 'grass_block' else 'Mycelium')
        command(ctx, 'setblock 8 100 8 stone')
        growth(ctx, 'block 8 99 8 dirt')
        block(ctx, (8, 99, 8), 'dirt', source='Grass_Block')
        command(ctx, 'setblock 8 100 8 air')


def farmland_trample_dry(ctx):
    command(ctx, 'weather clear')
    plant(ctx, 'farmland[moisture=7]', 'air')
    dried = growth(ctx, 'block 8 99 8 dirt', 1200)
    check(ctx, 'unplanted-farmland-dries', dried, dried, True, 'Farmland')
    command(ctx, 'setblock 8 99 8 farmland[moisture=7]')
    command(ctx, f'tp {ctx.clients[0].name} 8.5 104 8.5')
    verb(ctx, 'move', forward=0, strafe=0, ticks=80)
    block(ctx, (8, 99, 8), 'farmland' if ctx.factors['game_mode'] == 'spectator' else 'dirt', source='Farmland')


def random_tick_speed(ctx):
    plant(ctx, 'farmland[moisture=7]', 'wheat[age=0]')
    command(ctx, 'gamerule random_tick_speed 0')
    ticks(ctx, 200)
    block(ctx, (8, 100, 8), 'wheat[age=0]', source='Tick#Random_tick')
    grew = growth(ctx, 'block 8 100 8 wheat[age=7]')
    check(ctx, 'positive-random-ticks-enable-growth', grew, grew, True, 'Tick#Random_tick')


def shear_sheep_wool_regrow(ctx):
    command(ctx, 'fill 1 99 1 5 99 5 grass_block')
    command(ctx, 'summon sheep 3 100 3 {Color:0b}')
    interact(ctx, 'sheep', 'shears')
    ticks(ctx, 5)
    drops = [e for e in observe(ctx, 'entities') if e.get('type') == 'minecraft:item']
    if any(e.get('item') is None for e in drops):
        from ..matrix_api import MissingCapability
        raise MissingCapability('snapshot:entities.item')
    wool = inventory(ctx, 'white_wool') + sum(e['item']['count'] for e in drops
                                            if e['item']['item_id'] == 'minecraft:white_wool')
    permitted = ctx.factors['game_mode'] != 'spectator'
    check(ctx, 'shearing-wool-count', 1 <= wool <= 3 if permitted else wool == 0,
          wool, '1..3' if permitted else 0, 'Sheep#Shearing')
    actual = entity_field(ctx, 'sheep', 'Sheared')
    check(ctx, 'sheared', actual is permitted, actual, permitted, 'Sheep')
    if not permitted:
        return
    ctx.wait_until(lambda: entity_field(ctx, 'sheep', 'Sheared') is False,
                   description='sheep eats grass and regrows wool', timeout_seconds=300)
    check(ctx, 'wool-regrown', not entity_field(ctx, 'sheep', 'Sheared'), False, False, 'Sheep')


def milk_cow(ctx):
    for kind, item, result in [('cow', 'bucket', 'milk_bucket'), ('mooshroom', 'bucket', 'milk_bucket'), ('mooshroom', 'bowl', 'mushroom_stew')]:
        command(ctx, f'summon {kind} 3 100 3')
        interact(ctx, kind, item)
        got = inventory(ctx, result)
        check(ctx, kind + ':' + result, got == (0 if ctx.factors['game_mode'] == 'spectator' else 1), got, 0 if ctx.factors['game_mode'] == 'spectator' else 1, 'Mooshroom' if kind == 'mooshroom' else 'Cow')
        command(ctx, f'kill @e[type=minecraft:{kind}]')


def chicken_eggs(ctx):
    command(ctx, 'summon chicken 8 100 8')
    ticks(ctx, 12000)
    exists(ctx, '@e[type=minecraft:item,distance=..32]', source='Chicken')
    items = [e for e in observe(ctx, 'entities') if e.get('type') == 'minecraft:item']
    if any('item' not in item or item['item'] is None for item in items):
        from ..matrix_api import MissingCapability
        raise MissingCapability('snapshot:entities.item')
    eggs = [e for e in items if e['item']['item_id'] == 'minecraft:egg']
    check(ctx, 'chicken-laid-egg-within-6000-12000-ticks', bool(eggs), eggs, 'at least one egg', 'Chicken')


def bees_pollinate(ctx):
    command(ctx, 'time set noon')
    command(ctx, 'weather clear')
    command(ctx, 'gamerule random_tick_speed 0')
    command(ctx, 'setblock 10 102 8 beehive[honey_level=0]')
    plant(ctx, 'dirt', 'dandelion', (6, 100, 8))
    command(ctx, 'fill 7 99 7 9 99 9 farmland[moisture=7]')
    command(ctx, 'fill 7 100 7 9 100 9 wheat[age=0]')
    command(ctx, 'summon bee 6 101 8')
    ticks(ctx, 6000)
    increased = any(condition(ctx, f'block 10 102 8 beehive[honey_level={n}]') for n in range(1, 6))
    check(ctx, 'bee-hive-honey', increased, increased, True, 'Bee')
    fertilized = any(condition(ctx, f'block {x} 100 {z} wheat')
                     and not condition(ctx, f'block {x} 100 {z} wheat[age=0]')
                     for x in range(7, 10) for z in range(7, 10))
    check(ctx, 'bee-fertilizes-crop-with-random-ticks-disabled',
          fertilized, fertilized, 'at least one wheat age >0', 'Bee#Pollinating')
    command(ctx, 'gamerule random_tick_speed 3')


def taming(ctx):
    actor_uuid = data(ctx, ctx.clients[0].name, 'UUID')
    for kind, food in [('wolf', 'bone'), ('cat', 'cod'), ('parrot', 'wheat_seeds')]:
        command(ctx, f'summon {kind} 3 100 3')
        for _ in range(64):
            interact(ctx, kind, food)
            if entity_field(ctx, kind, 'Owner'):
                break
        owner = entity_field(ctx, kind, 'Owner')
        check(ctx, kind + ':owner', owner == actor_uuid, owner, actor_uuid, 'Taming')
        command(ctx, f'clear {ctx.clients[0].name}')
        if not entity_field(ctx, kind, 'Sitting'):
            interact(ctx, kind)
        sitting = entity_field(ctx, kind, 'Sitting')
        check(ctx, kind + ':sitting', sitting is True or sitting == 1,
              sitting, True, 'Taming')
        interact(ctx, kind)
        sitting = entity_field(ctx, kind, 'Sitting')
        check(ctx, kind + ':standing', sitting is False or sitting == 0,
              sitting, False, 'Taming')
        command(ctx, f'tp {ctx.clients[0].name} 18.5 100 3.5')
        ticks(ctx, 200)
        position = entity_field(ctx, kind, 'Pos')
        distance_squared = sum((v - target) ** 2 for v, target
                               in zip(position, (18.5, 100, 3.5)))
        check(ctx, kind + ':follows-owner', distance_squared <= 25,
              distance_squared, '<=25 squared blocks', 'Taming')
        command(ctx, f'tp {ctx.clients[0].name} 2.5 100 3.5')
        command(ctx, f'kill @e[type=minecraft:{kind}]')
    # Horse/llama taming and fox trust must be driven, not inferred from wolf.
    for kind in ('horse', 'llama'):
        command(ctx, f'summon {kind} 3 100 3')
        for _ in range(64):
            interact(ctx, kind)
            ticks(ctx, 100)
            if entity_field(ctx, kind, 'Tame'):
                break
        check(ctx, kind + ':tamed', entity_field(ctx, kind, 'Tame') is True, entity_field(ctx, kind, 'Tame'), True, 'Taming')
        owner = entity_field(ctx, kind, 'Owner')
        check(ctx, kind + ':owner', owner == actor_uuid, owner, actor_uuid, 'Taming')
        command(ctx, f'kill @e[type=minecraft:{kind}]')
    command(ctx, 'summon fox 3 100 3')
    command(ctx, 'summon fox 4 100 3')
    for fox in [e for e in observe(ctx, 'entities') if e.get('type') == 'minecraft:fox']:
        give(ctx, 'sweet_berries')
        verb(ctx, 'interact_entity', fox['id'])
    ticks(ctx, 100)
    foxes = [e for e in observe(ctx, 'entities') if e.get('type') == 'minecraft:fox']
    if any('baby' not in fox for fox in foxes):
        from ..matrix_api import MissingCapability
        raise MissingCapability('snapshot:entities.baby')
    babies = [fox for fox in foxes if fox['baby']]
    check(ctx, 'fox-breeding-offspring', len(babies) == 1, babies, 'one baby fox', 'Fox')
    if len(babies) == 1:
        baby_uuid = babies[0].get('uuid')
        if not baby_uuid:
            from ..matrix_api import MissingCapability
            raise MissingCapability('snapshot:entities.uuid')
        trusted = data(ctx, baby_uuid, 'Trusted')
        check(ctx, 'fox-offspring-trusts-breeder', actor_uuid in trusted,
              trusted, actor_uuid, 'Fox#Trust')


def golden_dandelion(ctx):
    command(ctx, 'summon cow 3 100 3 {Age:-24000}')
    interact(ctx, 'cow', 'golden_dandelion')
    locked = entity_field(ctx, 'cow', 'AgeLocked')
    check(ctx, 'age-lock-enabled', locked is True, locked, True, 'Golden_Dandelion')
    ticks(ctx, 40)
    interact(ctx, 'cow', 'golden_dandelion')
    locked = entity_field(ctx, 'cow', 'AgeLocked')
    check(ctx, 'age-lock-toggle', locked is False, locked, False, 'Golden_Dandelion')


ADAPTERS = {'farming.' + fn.__name__: adapter(fn) for fn in (
    wheat_carrot_potato_beetroot, stem_crops, nether_wart,
    sugar_cane_cactus_bamboo, kelp_seagrass, sweet_berries_cocoa_glow_berries,
    chorus, mushrooms_vines_spread, saplings_trees, grass_mycelium_spread,
    farmland_trample_dry, random_tick_speed, shear_sheep_wool_regrow, milk_cow,
    chicken_eggs, bees_pollinate, taming, golden_dandelion,
)}
