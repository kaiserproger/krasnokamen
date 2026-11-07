"""Movement inputs and received/server position checks for Java 26.1.2."""
import math

from ._world import (adapter, block, check, command, data, entity_field, give,
                     inventory, observe, ticks, verb)


def position(ctx):
    return observe(ctx, 'position')


def travel(ctx, *, sprint=False, count=40):
    before = position(ctx)
    verb(ctx, 'move', forward=1, strafe=0, ticks=count, sprint=sprint)
    after = position(ctx)
    return math.hypot(after[0]-before[0], after[2]-before[2])


def walk_sprint_jump(ctx):
    ctx.clients[0].look_at(2.5, 101.6, 30)
    distance = travel(ctx, count=40)
    check(ctx, 'walk-speed', 3.5 <= distance/2 <= 4.8, distance/2, '4.317 m/s; acceleration/publication tolerance', 'Sprinting')
    command(ctx, f'tp {ctx.clients[0].name} 2.5 100 2.5')
    distance = travel(ctx, sprint=True, count=40)
    check(ctx, 'sprint-speed', 4.7 <= distance/2 <= 6.1, distance/2, '5.612 m/s; acceleration/publication tolerance', 'Sprinting')
    before = position(ctx)[1]
    verb(ctx, 'jump', ticks=1)
    peak = observe(ctx, 'movement.max_y')
    check(ctx, 'jump-rise', peak > before + 1, peak-before, '>1 block', 'Jumping')
    if ctx.factors['game_mode'] == 'survival':
        exhaustion = data(ctx, ctx.clients[0].name, 'foodExhaustionLevel')
        check(ctx, 'sprint-exhaustion', exhaustion > 0, exhaustion, '>0', 'Hunger#Exhaustion_level_increase')


def sneak_edge(ctx):
    command(ctx, 'fill 0 99 3 8 99 8 air')
    command(ctx, f'tp {ctx.clients[0].name} 2.5 100 2.5')
    ctx.clients[0].look_at(2.5, 101.6, 10)
    verb(ctx, 'sneak', True)
    travel(ctx, count=40)
    after = position(ctx)
    check(ctx, 'sneak-edge-clamp', after[1] >= 99.99 and after[2] < 3.31, after, 'y>=100 and z<3.31', 'Sneaking')
    verb(ctx, 'sneak', False)


def swim_and_sink(ctx):
    command(ctx, 'fill 1 100 1 10 108 10 water')
    command(ctx, f'tp {ctx.clients[0].name} 3.5 103 3.5')
    before = data(ctx, ctx.clients[0].name, 'Air')
    verb(ctx, 'move', forward=1, strafe=0, ticks=40, sprint=True)
    after = data(ctx, ctx.clients[0].name, 'Air')
    check(ctx, 'underwater-air-consumption', after < before, [before, after], 'decreases', 'Water#Swimming')
    pose = observe(ctx, 'pose')
    check(ctx, 'sprint-swimming-pose', pose == 'swimming', pose, 'swimming', 'Swimming')
    y = position(ctx)[1]
    verb(ctx, 'jump', ticks=20)
    check(ctx, 'swim-up', position(ctx)[1] > y, position(ctx)[1]-y, '>0', 'Water#Swimming')
    verb(ctx, 'sneak', True)
    verb(ctx, 'move', forward=0, strafe=0, ticks=20)
    check(ctx, 'sink-input', position(ctx)[1] < 108, position(ctx)[1], '<surface', 'Water#Swimming')
    verb(ctx, 'sneak', False)


def climb(ctx):
    for climbable in ('ladder[facing=south]', 'vine[north=true]', 'scaffolding'):
        command(ctx, 'fill 2 100 2 4 108 3 air')
        command(ctx, 'fill 3 100 2 3 108 2 stone')
        command(ctx, f'fill 3 100 3 3 108 3 {climbable}')
        command(ctx, f'tp {ctx.clients[0].name} 3.5 100 3.5')
        ctx.clients[0].look_at(3.5, 101.6, 1)
        verb(ctx, 'jump', ticks=20)
        verb(ctx, 'move', forward=1, strafe=0, ticks=20)
        y = position(ctx)[1]
        check(ctx, climbable + ':climbs', y > 100.5, y, '>100.5', 'Climbing')
        verb(ctx, 'sneak', True)
        verb(ctx, 'move', forward=0, strafe=0, ticks=20)
        check(ctx, climbable + ':sneak-holds', position(ctx)[1] >= y-.1, position(ctx)[1]-y, '>=-0.1', 'Climbing')
        verb(ctx, 'sneak', False)


def step_up_slab_stairs(ctx):
    for step in ('stone_slab[type=bottom]', 'stone_stairs[facing=south]'):
        command(ctx, f'setblock 3 100 4 {step}')
        command(ctx, f'tp {ctx.clients[0].name} 3.5 100 3.5')
        ctx.clients[0].look_at(3.5, 101.6, 8)
        travel(ctx, count=8)
        y = position(ctx)[1]
        check(ctx, step + ':walk-step-up', y >= 100.5, y, '>=100.5 without jump', 'Slab')
        command(ctx, 'setblock 3 100 4 air')


def surface_modifiers(ctx):
    distances = {}
    for surface in ('stone', 'ice', 'blue_ice', 'slime_block', 'honey_block', 'soul_sand'):
        command(ctx, f'fill 1 99 1 5 99 30 {surface}')
        command(ctx, f'tp {ctx.clients[0].name} 3.5 100 2.5')
        ctx.clients[0].look_at(3.5, 101.6, 30)
        distances[surface] = travel(ctx, count=40)
    for surface in ('honey_block', 'soul_sand'):
        check(ctx, surface + ':slowdown', distances[surface] < distances['stone'], distances, '<stone', surface.title())
    command(ctx, 'setblock 3 99 3 magma_block')
    command(ctx, f'tp {ctx.clients[0].name} 3.5 100 3.5')
    verb(ctx, 'sneak', True)
    health = observe(ctx, 'health')
    ticks(ctx, 40)
    check(ctx, 'sneaking-prevents-magma-damage', observe(ctx, 'health') == health, observe(ctx, 'health'), health, 'Magma_Block')
    verb(ctx, 'sneak', False)


def entangling_blocks(ctx):
    for obstacle in ('cobweb', 'powder_snow', 'sweet_berry_bush[age=3]'):
        command(ctx, f'fill 2 100 2 4 101 8 {obstacle}')
        command(ctx, f'tp {ctx.clients[0].name} 3.5 100 3.5')
        ctx.clients[0].look_at(3.5, 101.6, 20)
        before = observe(ctx, 'health')
        distance = travel(ctx, count=40)
        check(ctx, obstacle + ':slowdown', distance < 8, distance, '<normal 40-tick walk', 'Cobweb' if obstacle == 'cobweb' else 'Powder_Snow' if obstacle == 'powder_snow' else 'Sweet_Berries')
        if obstacle == 'powder_snow':
            ticks(ctx, 200)
            frozen = data(ctx, ctx.clients[0].name, 'TicksFrozen')
            check(ctx, 'powder-snow-freezing', frozen > 0, frozen, '>0', 'Powder_Snow#Freezing')
        if obstacle.startswith('sweet') and ctx.factors['game_mode'] in ('survival', 'adventure'):
            check(ctx, 'berry-moving-damage', observe(ctx, 'health') < before, observe(ctx, 'health'), '<before', 'Sweet_Berries')
        command(ctx, 'fill 2 100 2 4 101 8 air')


def bubble_column(ctx):
    for base, drag in [('soul_sand', False), ('magma_block', True)]:
        command(ctx, 'fill 2 100 2 4 108 4 glass')
        command(ctx, 'fill 3 100 3 3 107 3 water')
        command(ctx, f'setblock 3 99 3 {base}')
        ticks(ctx, 20)
        block(ctx, (3, 101, 3), 'bubble_column[drag=' + str(drag).lower() + ']', source='Bubble_Column')
        command(ctx, f'tp {ctx.clients[0].name} 3.5 103 3.5')
        y = position(ctx)[1]
        verb(ctx, 'move', forward=0, strafe=0, ticks=10)
        dy = position(ctx)[1]-y
        check(ctx, base + ':vertical-force', dy < 0 if drag else dy > 0, dy, '<0' if drag else '>0', 'Bubble_Column')


def chunk_boundary_crossing(ctx):
    command(ctx, 'fill 0 99 0 47 99 3 stone')
    command(ctx, f'tp {ctx.clients[0].name} 14.5 100 1.5')
    ctx.clients[0].look_at(46, 101.6, 1.5)
    travel(ctx, sprint=True, count=80)
    pos = position(ctx)
    check(ctx, 'crosses-two-chunk-seams', pos[0] > 32, pos, 'x>32', 'Chunk')
    corrections = observe(ctx, 'movement.corrections')
    check(ctx, 'no-unexpected-seam-correction', corrections == 0, corrections, 0, 'Chunk')


def illegal_move_correction(ctx):
    before = position(ctx)
    verb(ctx, 'move_to', before[0]+1000, before[1], before[2], on_ground=True)
    ticks(ctx, 2)
    after = position(ctx)
    check(ctx, 'too-fast-corrected', abs(after[0]-before[0]) < 1, after, before, 'Player#Movement')
    command(ctx, 'setblock 3 100 3 stone')
    verb(ctx, 'move_to', 3.5, 100, 3.5, on_ground=True)
    ticks(ctx, 2)
    after = position(ctx)
    check(ctx, 'solid-block-collision-corrected', not (3.2 < after[0] < 3.8 and 3.2 < after[2] < 3.8), after, 'outside solid cube', 'Player#Movement')


def teleport_confirm(ctx):
    verb(ctx, 'set_teleport_confirmation', False)
    command(ctx, f'tp {ctx.clients[0].name} 10 100 10')
    verb(ctx, 'move_to', 12, 100, 10, on_ground=True)
    actual = data(ctx, ctx.clients[0].name, 'Pos')
    check(ctx, 'pre-confirm-position-ignored', abs(actual[0]-10) < .01, actual, [10, 100, 10], 'Commands/teleport')
    verb(ctx, 'set_teleport_confirmation', True)
    verb(ctx, 'move', forward=1, strafe=0, ticks=10)
    actual = data(ctx, ctx.clients[0].name, 'Pos')
    check(ctx, 'post-confirm-movement-accepted', actual != [10, 100, 10], actual, 'changed after confirm', 'Commands/teleport')


def elytra_glide(ctx):
    give(ctx, 'elytra')
    verb(ctx, 'click_slot', 36)
    verb(ctx, 'click_slot', 6)
    command(ctx, f'tp {ctx.clients[0].name} 8 145 8')
    verb(ctx, 'start_fall_flying')
    ticks(ctx, 5)
    check(ctx, 'fall-flying-admitted', observe(ctx, 'pose') == 'fall_flying', observe(ctx, 'pose'), 'fall_flying', 'Elytra')
    give(ctx, 'firework_rocket', clear=False)
    before = data(ctx, ctx.clients[0].name, 'Motion')
    verb(ctx, 'use_item', ticks=1)
    after = data(ctx, ctx.clients[0].name, 'Motion')
    check(ctx, 'firework-boost', sum(v*v for v in after) > sum(v*v for v in before), after, 'speed increases', 'Elytra#Flying')


def riptide(ctx):
    give(ctx, 'trident')
    command(ctx, f'enchant {ctx.clients[0].name} riptide 3')
    for wet in (False, True):
        command(ctx, 'weather rain' if wet else 'weather clear')
        command(ctx, f'tp {ctx.clients[0].name} 8 100 8')
        ctx.clients[0].look_at(8, 110, 24)
        before = position(ctx)
        verb(ctx, 'use_item', ticks=20)
        verb(ctx, 'release_use_item')
        ticks(ctx, 3)
        after = position(ctx)
        distance = math.dist(before, after)
        check(ctx, 'riptide-wet' if wet else 'riptide-dry', distance > 2 if wet else distance < 1, distance, '>2' if wet else '<1', 'Riptide')


def movement_enchantments(ctx):
    for enchantment, item, surface in [('depth_strider', 'diamond_boots', 'water'), ('soul_speed', 'diamond_boots', 'soul_sand'), ('swift_sneak', 'diamond_leggings', 'stone')]:
        distances = []
        for level in (0, 3):
            command(ctx, 'fill 1 100 1 5 102 30 air')
            if surface == 'water':
                command(ctx, 'fill 1 100 1 5 102 30 water')
            else:
                command(ctx, f'fill 1 99 1 5 99 30 {surface}')
            give(ctx, item)
            if level:
                command(ctx, f'enchant {ctx.clients[0].name} {enchantment} {level}')
            verb(ctx, 'click_slot', 36)
            verb(ctx, 'click_slot', 8 if item.endswith('boots') else 7)
            command(ctx, f'tp {ctx.clients[0].name} 3.5 100 2.5')
            ctx.clients[0].look_at(3.5, 101.6, 30)
            if enchantment == 'swift_sneak':
                verb(ctx, 'sneak', True)
            distances.append(travel(ctx))
            verb(ctx, 'sneak', False)
        check(ctx, enchantment + ':speed-modifier', distances[1] > distances[0], distances, 'enchanted > plain', enchantment.title())


def collision_entities(ctx):
    command(ctx, 'gamerule max_entity_cramming 2')
    command(ctx, 'fill 7 100 7 9 102 9 stone')
    command(ctx, 'fill 8 100 8 8 102 8 air')
    for _ in range(4):
        command(ctx, 'summon cow 8.5 100 8.5')
    before = entity_field(ctx, 'cow', 'Health')
    ticks(ctx, 80)
    after = entity_field(ctx, 'cow', 'Health')
    check(ctx, 'over-cap-cramming-damage', after < before, after, '<before', 'Damage#Entity_cramming')
    command(ctx, 'gamerule max_entity_cramming 24')


ADAPTERS = {'movement.' + fn.__name__: adapter(fn) for fn in (
    walk_sprint_jump, sneak_edge, swim_and_sink, climb, step_up_slab_stairs,
    surface_modifiers, entangling_blocks, bubble_column, chunk_boundary_crossing,
    illegal_move_correction, teleport_confirm, elytra_glide, riptide,
    movement_enchantments, collision_entities,
)}
