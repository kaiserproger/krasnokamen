"""Dimension/portal transitions with actual identity, block and entity oracles."""
from ._world import (adapter, block, boundary, check, command, condition, data,
                     entity, entity_field, exists, give, observe, ticks, use, verb)


def frame(ctx, *, x=3, dimension=None):
    command(ctx, f'fill {x} 100 3 {x+3} 104 3 obsidian', dimension)
    command(ctx, f'fill {x+1} 101 3 {x+2} 103 3 air', dimension)


def light(ctx):
    frame(ctx)
    command(ctx, f'tp {ctx.clients[0].name} 4.5 101 1.5')
    use(ctx, (4, 100, 3), 'flint_and_steel')


def nether_portal_build_light(ctx):
    ctx.clients[0].teleport_to(10.5, 101, 13.5)
    permitted = ctx.factors['game_mode'] in ('survival', 'creative')
    for width, height in ((4, 5), (23, 23), (24, 23)):
        command(ctx, 'fill 10 100 16 33 123 16 air')
        command(ctx, f'fill 10 100 16 {9+width} {99+height} 16 obsidian')
        command(ctx, f'fill 11 101 16 {8+width} {98+height} 16 air')
        use(ctx, (11, 100, 16), 'flint_and_steel')
        ticks(ctx, 2)
        if width <= 23:
            for x in range(11, 9+width):
                for y in range(101, 99+height):
                    block(ctx, (x, y, 16), 'nether_portal[axis=x]' if permitted else 'air',
                          source='Nether_Portal#Creation', name=f'frame:{width}x{height}:{x}:{y}')
        else:
            block(ctx, (11, 102, 16), 'air', source='Nether_Portal#Creation',
                  name='oversize-frame-does-not-light')
        command(ctx, 'setblock 10 102 16 air')
        ticks(ctx, 2)
        block(ctx, (11, 102, 16), 'air', source='Nether_Portal#Creation')
        use(ctx, (11, 100, 16), 'flint_and_steel')
        ticks(ctx, 2)
        block(ctx, (11, 102, 16), 'air', source='Nether_Portal#Creation')


def nether_portal_travel_link(ctx):
    source = ctx.factors['dimension']
    destination = 'overworld' if source == 'the_nether' else 'the_nether'
    frame(ctx, x=80, dimension='overworld')
    frame(ctx, x=10, dimension='the_nether')
    for dim, x in [('overworld', 80), ('the_nether', 10)]:
        command(ctx, f'setblock {x+1} 101 3 fire', dim)
    x = 81 if source == 'overworld' else 11
    command(ctx, f'tp {ctx.clients[0].name} {x+.5} 101 3.5')
    boundary(ctx)
    ticks(ctx, 90)
    actual = observe(ctx, 'dimension')
    check(ctx, 'portal-destination', actual == 'minecraft:' + destination, actual, 'minecraft:' + destination, 'Nether_Portal#Behavior')
    pos = observe(ctx, 'position')
    target_x = 11 if destination == 'the_nether' else 81
    check(ctx, 'existing-scaled-portal-reused', abs(pos[0]-target_x) < 4, pos, f'x within 4 of {target_x}', 'Nether_Portal#Portal_linkage_between_Overworld_and_Nether')


def portal_gamerules(ctx):
    light(ctx)
    command(ctx, 'gamerule allow_entering_nether_using_portals false')
    command(ctx, f'tp {ctx.clients[0].name} 4.5 101 3.5')
    ticks(ctx, 100)
    actual = observe(ctx, 'dimension')
    check(ctx, 'nether-travel-disabled', actual == 'minecraft:' + ctx.factors['dimension'], actual, 'minecraft:' + ctx.factors['dimension'], 'Nether_Portal')
    command(ctx, 'gamerule allow_entering_nether_using_portals true')
    command(ctx, 'gamerule players_nether_portal_default_delay 20')
    command(ctx, 'gamerule players_nether_portal_creative_delay 20')
    command(ctx, f'tp {ctx.clients[0].name} 4.5 101 1.5')
    command(ctx, f'tp {ctx.clients[0].name} 4.5 101 3.5')
    ticks(ctx, 30)
    actual = observe(ctx, 'dimension')
    expected = 'minecraft:the_nether' if ctx.factors['dimension'] == 'overworld' else 'minecraft:overworld'
    check(ctx, 'portal-delay-allows-travel', actual == expected, actual, expected, 'Nether_Portal')


def entities_through_portals(ctx):
    frame(ctx)
    command(ctx, 'setblock 4 101 3 fire')
    command(ctx, 'summon item 4.5 101 3.5 {Item:{id:"minecraft:diamond",count:1},PickupDelay:32767}')
    command(ctx, 'summon pig 4.5 101 3.5')
    boundary(ctx)
    ticks(ctx, 10)
    destination = 'the_nether' if ctx.factors['dimension'] == 'overworld' else 'overworld'
    for kind in ('item', 'pig'):
        exists(ctx, f'@e[type=minecraft:{kind}]', expected=False, source='Nether_Portal')
        exists(ctx, f'@e[type=minecraft:{kind}]', dimension=destination, source='Nether_Portal')


def end_portal_travel(ctx):
    command(ctx, 'setblock 3 100 3 end_portal')
    command(ctx, f'tp {ctx.clients[0].name} 3.5 100 3.5')
    boundary(ctx)
    ticks(ctx, 10)
    dimension = observe(ctx, 'dimension')
    check(ctx, 'end-entry', dimension == 'minecraft:the_end', dimension, 'minecraft:the_end', 'End_Portal#Behavior')
    block(ctx, (100, 48, 0), 'obsidian', dimension='the_end', source='Obsidian_platform')


def end_exit_return(ctx):
    command(ctx, 'setworldspawn 8 100 8', 'overworld')
    command(ctx, 'setblock 3 100 3 end_portal')
    command(ctx, f'tp {ctx.clients[0].name} 3.5 100 3.5')
    boundary(ctx)
    ticks(ctx, 10)
    dimension = observe(ctx, 'dimension')
    check(ctx, 'end-exit-overworld', dimension == 'minecraft:overworld', dimension, 'minecraft:overworld', 'Exit_portal')
    pos = observe(ctx, 'position')
    check(ctx, 'end-exit-spawn', abs(pos[0]-8) <= 10 and abs(pos[2]-8) <= 10, pos, 'world spawn neighborhood', 'Exit_portal')


def end_gateway(ctx):
    command(ctx, 'setblock 3 100 3 end_gateway')
    command(ctx, f'tp {ctx.clients[0].name} 3.5 100 3.5')
    boundary(ctx)
    ticks(ctx, 10)
    pos = observe(ctx, 'position')
    check(ctx, 'gateway-reaches-outer-islands', pos[0]**2 + pos[2]**2 >= 768**2, pos, 'horizontal radius >=768', 'End_Gateway#Behavior')
    # A second use must actually return through the generated remote gateway.
    nearby = observe(ctx, 'nearby_blocks')
    gateways = [b['position'] for b in nearby if b.get('block_id') == 'minecraft:end_gateway']
    if not gateways:
        from ..matrix_api import MissingCapability
        raise MissingCapability('observation:remote_end_gateway_position')
    ticks(ctx, 40)
    x, y, z = gateways[0]
    command(ctx, f'tp {ctx.clients[0].name} {x+.5} {y} {z+.5}')
    ticks(ctx, 10)
    pos = observe(ctx, 'position')
    check(ctx, 'gateway-returns-inner-island', pos[0]**2 + pos[2]**2 < 256**2, pos, 'radius <256', 'End_Gateway')


def dragon_fight(ctx):
    command(ctx, f'tp {ctx.clients[0].name} 0 80 0')
    # Destroy actual crystals by client attacks, not /kill the fight owner.
    for target in observe(ctx, 'entities'):
        if target.get('type') == 'minecraft:end_crystal':
            x, y, z = target['position']
            command(ctx, f'tp {ctx.clients[0].name} {x+2} {y} {z}')
            verb(ctx, 'attack', target['id'])
    boundary(ctx)
    give(ctx, 'netherite_sword')
    phases = set()
    for _ in range(300):
        if not condition(ctx, 'entity @e[type=minecraft:ender_dragon]'):
            break
        phases.add(data(ctx, '@e[type=minecraft:ender_dragon,limit=1]', 'DragonPhase'))
        target = entity(ctx, 'ender_dragon')
        x, y, z = target['position']
        command(ctx, f'tp {ctx.clients[0].name} {x+2} {y} {z}')
        verb(ctx, 'attack', target['id'])
        ticks(ctx, 12)
    ticks(ctx, 220)
    exists(ctx, '@e[type=minecraft:ender_dragon]', expected=False, source='Ender_Dragon#Death')
    exists(ctx, '@e[type=minecraft:experience_orb]', source='Ender_Dragon#Drops')
    check(ctx, 'dragon-has-flight-phases', len(phases) > 1, sorted(phases), 'multiple observed DragonPhase values', 'Ender_Dragon#Behavior')
    block(ctx, (0, 64, 0), 'dragon_egg', source='Ender_Dragon#Death')


def dragon_respawn(ctx):
    # Native dragon death must complete before crystals may trigger a new fight.
    command(ctx, 'kill @e[type=minecraft:ender_dragon]')
    ticks(ctx, 220)
    for x, z in [(0, -2), (0, 2), (-2, 0), (2, 0)]:
        command(ctx, f'tp {ctx.clients[0].name} {x+1.5} 64 {z+1.5}')
        use(ctx, (x, 63, z), 'end_crystal')
    ticks(ctx, 700)
    exists(ctx, '@e[type=minecraft:ender_dragon]', source='Ender_Dragon#Respawning')
    exists(ctx, '@e[type=minecraft:end_crystal]', source='Ender_Dragon#Respawning')


def eye_of_ender_throw(ctx):
    give(ctx, 'ender_eye')
    verb(ctx, 'use_item', ticks=1)
    exists(ctx, '@e[type=minecraft:eye_of_ender]', source='Eye_of_Ender#Locating_strongholds')
    before = entity_field(ctx, 'eye_of_ender', 'Pos')
    ticks(ctx, 10)
    after = entity_field(ctx, 'eye_of_ender', 'Pos')
    check(ctx, 'eye-travels', before != after, [before, after], 'position changes toward stronghold', 'Eye_of_Ender')
    ticks(ctx, 80)
    exists(ctx, '@e[type=minecraft:eye_of_ender]', expected=False, source='Eye_of_Ender')


def command_dimension_travel(ctx):
    initial = ctx.factors['dimension']
    for dimension in ('the_nether', 'the_end', initial):
        text = f'execute in minecraft:{dimension} run tp {ctx.clients[0].name} 8 100 8'
        if ctx.factors['permission'] == 'console':
            ctx.server.command(text)
        else:
            ctx.clients[0].command(text)
        boundary(ctx)
        ticks(ctx, 5)
        actual = observe(ctx, 'dimension')
        check(ctx, 'command-dimension:' + dimension, actual == 'minecraft:' + dimension, actual, 'minecraft:' + dimension, 'Commands/execute#in')
        message = 'world-dimension-' + dimension
        ctx.clients[0].chat(message)
        events = observe(ctx, 'events')
        check(ctx, 'chat-after-dimension:' + dimension, any(message in str(e) for e in events), events, message, 'Chat')


ADAPTERS = {'portals_dimensions.' + fn.__name__: adapter(fn) for fn in (
    nether_portal_build_light, nether_portal_travel_link, portal_gamerules,
    entities_through_portals, end_portal_travel, end_exit_return, end_gateway,
    dragon_fight, dragon_respawn, eye_of_ender_throw, command_dimension_travel,
)}
