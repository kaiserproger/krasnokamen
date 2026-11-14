"""Fluid behavior oracles, Java 26.1.2; block queries are server authoritative."""
from ._world import (adapter, block, check, command, entity_field, give,
                     inventory, plant, ticks, use, verb)


def water_flow(ctx):
    command(ctx, 'setblock 16 100 16 water[level=0]')
    ticks(ctx, 60)
    for offset in range(1, 8):
        block(ctx, (16 + offset, 100, 16), f'water[level={offset}]', source='Water#Spreading')
    block(ctx, (24, 100, 16), 'air', source='Water#Spreading')
    command(ctx, 'fill 18 96 16 18 99 16 air')
    ticks(ctx, 40)
    block(ctx, (18, 98, 16), 'water', source='Water#Spreading')
    # A separate sealed channel compares two reachable drops, not merely falling.
    # East is two blocks from the source; west is three blocks away.
    command(ctx, 'fill 31 97 15 39 101 17 stone')
    command(ctx, 'fill 32 100 16 38 101 16 air')
    command(ctx, 'fill 32 98 16 32 99 16 air')
    command(ctx, 'fill 37 98 16 37 99 16 air')
    command(ctx, 'setblock 35 100 16 water[level=0]')
    ticks(ctx, 40)
    block(ctx, (36, 100, 16), 'water[level=1]', source='Water#Spreading')
    block(ctx, (37, 99, 16), 'water', source='Water#Spreading')
    block(ctx, (34, 100, 16), 'air', source='Water#Spreading')


def lava_flow_by_dimension(ctx):
    command(ctx, 'setblock 16 100 16 lava[level=0]')
    ticks(ctx, 240)
    nether = ctx.factors['dimension'] == 'the_nether'
    extent, decay = (7, 1) if nether else (3, 2)
    for offset in range(1, extent + 1):
        block(ctx, (16 + offset, 100, 16), f'lava[level={offset*decay}]', source='Lava#Flow')
    block(ctx, (17 + extent, 100, 16), 'air', source='Lava#Flow')


def source_conversion(ctx, fluid, enabled):
    command(ctx, 'fill 6 100 7 10 100 9 stone')
    command(ctx, 'fill 7 100 8 9 100 8 air')
    command(ctx, f'gamerule {fluid}_source_conversion {str(enabled).lower()}')
    command(ctx, f'setblock 7 100 8 {fluid}[level=0]')
    command(ctx, f'setblock 9 100 8 {fluid}[level=0]')
    ticks(ctx, 120)
    block(ctx, (8, 100, 8), fluid + ('[level=0]' if enabled else '[level=1]' if fluid == 'water' or ctx.factors['dimension'] == 'the_nether' else '[level=2]'), source=fluid.title() + '#Source_blocks')


def infinite_water_source(ctx):
    for enabled in (False, True):
        source_conversion(ctx, 'water', enabled)


def lava_source_conversion(ctx):
    for enabled in (False, True):
        source_conversion(ctx, 'lava', enabled)
    command(ctx, 'gamerule lava_source_conversion false')


def water_lava_interaction(ctx):
    for lava, result in [('lava[level=0]', 'obsidian'), ('lava[level=2]', 'cobblestone')]:
        command(ctx, 'fill 7 100 7 9 101 9 air')
        command(ctx, f'setblock 8 100 8 {lava}')
        command(ctx, 'setblock 9 100 8 water')
        ticks(ctx, 40)
        block(ctx, (8, 100, 8), result, source='Fluid#Mixing')
    command(ctx, 'fill 7 100 7 9 102 9 air')
    command(ctx, 'setblock 8 100 8 water')
    command(ctx, 'setblock 8 101 8 lava')
    ticks(ctx, 40)
    block(ctx, (8, 100, 8), 'stone', source='Fluid#Mixing')
    command(ctx, 'fill 7 100 7 9 102 9 air')
    command(ctx, 'setblock 8 99 8 soul_soil')
    command(ctx, 'setblock 9 100 8 blue_ice')
    command(ctx, 'setblock 8 101 8 lava')
    ticks(ctx, 60)
    block(ctx, (8, 100, 8), 'basalt', source='Basalt#Post-generation')


def buckets(ctx):
    for fluid in ('water', 'lava', 'powder_snow'):
        command(ctx, f'setblock 3 100 3 {fluid}')
        use(ctx, (3, 100, 3), 'bucket')
        spectator = ctx.factors['game_mode'] == 'spectator'
        block(ctx, (3, 100, 3), fluid if spectator else 'air', source='Bucket')
        count = inventory(ctx, fluid + '_bucket')
        check(ctx, fluid + ':filled-bucket', count == (0 if spectator else 1), count, 0 if spectator else 1, 'Bucket')
        give(ctx, fluid + '_bucket')
        use(ctx, (3, 99, 3))
        evaporates = fluid == 'water' and ctx.factors['dimension'] == 'the_nether'
        block(ctx, (3, 100, 3), 'air' if evaporates or spectator else fluid, source='Bucket')
        if ctx.factors['game_mode'] == 'survival':
            count = inventory(ctx, 'bucket')
            check(ctx, fluid + ':empty-bucket', count == 1, count, 1, 'Bucket')


def water_in_nether(ctx):
    use(ctx, (3, 99, 3), 'water_bucket')
    block(ctx, (3, 100, 3), 'air', source='Water#Natural_generation')
    if ctx.factors['game_mode'] == 'survival':
        count = inventory(ctx, 'bucket')
        check(ctx, 'water-evaporates-bucket-remains', count == 1, count, 1, 'Water#Natural_generation')


def waterlogging(ctx):
    for material in ('oak_slab', 'oak_stairs', 'oak_fence'):
        command(ctx, f'setblock 3 100 3 {material}[waterlogged=false]')
        use(ctx, (3, 100, 3), 'water_bucket')
        expected = ctx.factors['game_mode'] != 'spectator' and ctx.factors['dimension'] != 'the_nether'
        block(ctx, (3, 100, 3), f'{material}[waterlogged={str(expected).lower()}]', source='Waterlogging')
        if expected:
            use(ctx, (3, 100, 3), 'bucket')
            block(ctx, (3, 100, 3), f'{material}[waterlogged=false]', source='Waterlogging')
            use(ctx, (3, 100, 3), 'water_bucket')
            verb(ctx, 'break_block', (3, 100, 3))
            block(ctx, (3, 100, 3), 'water[level=0]', source='Waterlogging')


def entity_push_items(ctx):
    command(ctx, 'fill 6 100 7 16 101 9 stone')
    command(ctx, 'fill 7 100 8 15 101 8 air')
    command(ctx, 'setblock 7 100 8 water')
    ticks(ctx, 50)
    command(ctx, 'summon item 8.5 100.2 8.5 {Item:{id:"minecraft:stone",count:1},PickupDelay:32767}')
    command(ctx, 'summon pig 8.5 100.2 8.5')
    for kind in ('item', 'pig'):
        before = entity_field(ctx, kind, 'Pos')[0]
        ticks(ctx, 40)
        after = entity_field(ctx, kind, 'Pos')[0]
        check(ctx, kind + ':flow-push-positive-x', after > before, after - before, '>0', 'Water#Current')


def fire_extinguish(ctx):
    plant(ctx, 'netherrack', 'fire', (8, 100, 8))
    command(ctx, 'setblock 7 100 8 water')
    ticks(ctx, 20)
    block(ctx, (8, 100, 8), 'water', source='Water#Spreading')
    command(ctx, 'setblock 12 100 8 campfire[lit=true]')
    command(ctx, 'setblock 11 100 8 water')
    ticks(ctx, 20)
    block(ctx, (12, 100, 8), 'campfire[lit=false,waterlogged=true]', source='Campfire#Extinguishing')


def concrete_powder(ctx):
    for color in ('white', 'red', 'blue', 'black'):
        command(ctx, 'setblock 8 100 8 air')
        command(ctx, f'setblock 8 100 8 {color}_concrete_powder')
        command(ctx, 'setblock 9 100 8 water')
        ticks(ctx, 10)
        block(ctx, (8, 100, 8), color + '_concrete', source='Concrete_Powder#Creating_concrete')
        command(ctx, 'setblock 9 100 8 air')


def sponge(ctx):
    ctx.clients[0].teleport_to(2.5, 100, 13.5)
    verb(ctx, 'close_container')
    give(ctx, 'sponge')
    command(ctx, 'fill 1 100 14 5 102 18 water')
    command(ctx, 'setblock 3 100 16 air')
    use(ctx, (3, 99, 16))
    if ctx.factors['game_mode'] in ('survival', 'creative'):
        block(ctx, (3, 100, 16), 'wet_sponge', source='Sponge#Absorbing_water')
        block(ctx, (3, 101, 16), 'air', source='Sponge#Absorbing_water')
    else:
        block(ctx, (3, 100, 16), 'water', source='Sponge#Absorbing_water')


ADAPTERS = {'fluids.' + fn.__name__: adapter(fn) for fn in (
    water_flow, lava_flow_by_dimension, infinite_water_source, lava_source_conversion,
    water_lava_interaction, buckets, water_in_nether, waterlogging,
    entity_push_items, fire_extinguish, concrete_powder, sponge,
)}
