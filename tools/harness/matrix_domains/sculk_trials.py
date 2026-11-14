"""Vanilla expectations: minecraft.wiki/w/{Sculk_Sensor,Calibrated_Sculk_Sensor,
Sculk_Catalyst,Sculk_Shrieker,Warden,Trial_Spawner,Ominous_Trial_Spawner}.
Krasnokamen sources: sculk.rs, vibration.rs, shrieker.rs, warden.rs, trial_spawner.rs.
Timing is observed by dashboard/native publication, never wall-clock sleeps.
"""
from tools.harness.matrix_api import Adapter
from . import _platform as p


def sensor(ctx, calibrated=False):
    x, y, z = p.arena(ctx)
    pos = (x+3, y, z)
    name = 'calibrated_sculk_sensor[facing=east]' if calibrated else 'sculk_sensor'
    p.admin(ctx, f'setblock {x+3} {y} {z} minecraft:{name}')
    p.ticks(ctx, 10)
    return pos


def sculk_sensor(ctx):
    pos = sensor(ctx)
    p.rich_action(ctx, 'move', forward=1, ticks=4)
    phases = p.rich_action(ctx, 'block_state_trace', pos, property='sculk_sensor_phase', ticks=50)
    p.check(ctx, 'active then cooldown then inactive', phases['transitions'], ['active', 'cooldown', 'inactive'])
    p.check(ctx, 'active duration', phases['durations']['active'], 30)
    p.check(ctx, 'cooldown duration', phases['durations']['cooldown'], 10)
    x, y, z = pos
    p.admin(ctx, f'fill {x-1} {y} {z-2} {x-1} {y+3} {z+2} minecraft:white_wool')
    p.rich_action(ctx, 'move', forward=-1, ticks=4)
    trace = p.rich_action(ctx, 'block_state_trace', pos, property='sculk_sensor_phase', ticks=50)
    p.test(ctx, 'wool occludes', 'active' not in trace['transitions'], trace)


def calibrated_sensor(ctx):
    pos = sensor(ctx, calibrated=True)
    x, y, z = pos
    # East-facing calibrated side accepts 15; block placement has frequency 13.
    p.admin(ctx, f'setblock {x+1} {y} {z} minecraft:redstone_block')
    p.admin(ctx, f'give {p.client(ctx).name} minecraft:stone 2')
    p.rich_action(ctx, 'select_inventory_item', 'minecraft:stone')
    p.use(ctx, (x-2, y-1, z))
    trace = p.rich_action(ctx, 'block_state_trace', pos, property='sculk_sensor_phase', ticks=40)
    p.test(ctx, 'mismatched frequency rejected', 'active' not in trace['transitions'], trace)
    p.admin(ctx, f'setblock {x+1} {y} {z} minecraft:air')
    p.ticks(ctx, 30)
    p.use(ctx, (x-2, y-1, z+1))
    trace = p.rich_action(ctx, 'block_state_trace', pos, property='sculk_sensor_phase', ticks=40)
    p.test(ctx, 'zero input accepts all frequencies', 'active' in trace['transitions'], trace)
    p.check(ctx, 'calibrated active duration', trace['durations']['active'], 10)


def sculk_catalyst_spread(ctx):
    x, y, z = p.arena(ctx)
    p.admin(ctx, f'setblock {x+2} {y} {z} minecraft:sculk_catalyst')
    p.admin(ctx, f'summon minecraft:zombie {x+3} {y} {z}')
    p.rich_action(ctx, 'attack_until_dead', p.rich_action(ctx, 'find_entity', 'minecraft:zombie'))
    p.ticks(ctx, 200)
    states = p.rich_action(ctx, 'read_blocks', (x-8,y-1,z-8), (x+8,y-1,z+8))
    sculk = [state for state in states if state['name'] == 'minecraft:sculk']
    p.test(ctx, 'XP death spreads sculk', len(sculk) > 0, sculk)
    p.check(ctx, 'catalyst consumes death XP', p.rich_action(ctx, 'entity_count', 'minecraft:experience_orb'), 0)


def shrieker_warden_summon(ctx):
    x, y, z = p.arena(ctx)
    p.admin(ctx, 'gamerule doWardenSpawning true')
    pos = (x+2,y,z)
    p.admin(ctx, f'setblock {x+2} {y} {z} minecraft:sculk_shrieker[can_summon=true]')
    for warning in range(1,5):
        p.client(ctx).teleport_to(x+2.5,y+1,z+.5)
        p.rich_action(ctx, 'move', forward=.1, ticks=1)
        p.ticks(ctx, 100)
        p.query(ctx, f'data get entity {p.client(ctx).name} warden_spawn_tracker.warning_level', rf'\b{warning}\b')
        if warning < 4:
            p.check(ctx, 'no early warden', p.rich_action(ctx, 'entity_count', 'minecraft:warden'), 0)
        p.client(ctx).teleport_to(x,y,z)
        p.ticks(ctx, 200)
    p.check(ctx, 'fourth warning summons one warden', p.rich_action(ctx, 'entity_count', 'minecraft:warden'), 1)
    p.test(ctx, 'darkness applied', 'minecraft:darkness' in p.observe(ctx, 'effects'), p.observe(ctx, 'effects'))


def warden_behaviour(ctx):
    x,y,z = p.arena(ctx)
    p.admin(ctx, f'summon minecraft:warden {x+6} {y} {z}')
    warden = p.rich_action(ctx, 'find_entity', 'minecraft:warden')
    p.rich_action(ctx, 'move', forward=1, ticks=20)
    p.test(ctx, 'vibrations increase anger', p.rich_action(ctx, 'entity_state', warden)['anger'] > 0,
           p.rich_action(ctx, 'entity_state', warden))
    p.admin(ctx, f'give {p.client(ctx).name} minecraft:snowball 1')
    p.rich_action(ctx, 'select_inventory_item', 'minecraft:snowball')
    p.client(ctx).look_at(x+5,y,z)
    p.rich_action(ctx, 'use_item', hand='main', ticks=1)
    p.check(ctx, 'projectile investigated', p.rich_action(ctx, 'entity_state', warden)['investigating'], True)
    for item, armor_slot in [('netherite_helmet',5),('netherite_chestplate',6),
                             ('netherite_leggings',7),('netherite_boots',8)]:
        p.admin(ctx, f'give {p.client(ctx).name} minecraft:{item} 1')
        source = next(s['slot'] for s in p.observe(ctx,'inventory')
                      if s['item_id'] == 'minecraft:'+item)
        p.client(ctx).click_slot(source)
        p.client(ctx).click_slot(armor_slot)
    p.client(ctx).teleport_to(x+4,y,z)
    p.client(ctx).attack(warden)
    p.client(ctx).teleport_to(x-2,y,z)
    p.admin(ctx, f'fill {x+2} {y} {z-3} {x+2} {y+4} {z+3} minecraft:stone')
    event = ctx.wait_until(lambda: next((e for e in p.observe(ctx,'damage_events')
                           if e['type']=='minecraft:sonic_boom'), None),
                           description='sonic boom through wall', timeout_seconds=60)
    p.check(ctx, 'sonic boom ignores armor', event['health_before']-event['health_after'],
            {'easy':5,'normal':10,'hard':15}.get(ctx.factors['difficulty'], 0))
    p.admin(ctx, 'kill @e[type=minecraft:warden]')
    p.admin(ctx, f'summon minecraft:warden {x+6} {y} {z}')
    quiet = p.rich_action(ctx,'find_entity','minecraft:warden')
    p.client(ctx).teleport_to(x-24,y,z)
    p.ticks(ctx, 1200)
    p.check(ctx, 'idle warden digs away', p.rich_action(ctx, 'entity_exists', quiet), False)


def trial(ctx):
    x,y,z = p.arena(ctx)
    pos = (x+4,y,z)
    p.admin(ctx, f'setblock {x+4} {y} {z} minecraft:trial_spawner')
    p.admin(ctx, f'give {p.client(ctx).name} minecraft:zombie_spawn_egg 1')
    p.rich_action(ctx, 'select_inventory_item', 'minecraft:zombie_spawn_egg')
    p.use(ctx, pos)
    return pos


def defeat_waves(ctx, pos, expected):
    seen = set()
    max_alive = 0
    start_events = len(p.observe(ctx,'entity_events'))
    reward_start = None
    for _ in range(500):
        entities = p.observe(ctx,'entities')
        mobs = [e for e in entities if e['type']=='minecraft:zombie']
        max_alive = max(max_alive,len(mobs))
        for mob in mobs:
            seen.add(mob['uuid'])
            if len(seen) > expected:
                raise AssertionError(f'trial spawned more than {expected} mobs')
            p.client(ctx).teleport_to(mob['position'][0]-1,mob['position'][1],mob['position'][2])
            p.rich_action(ctx,'attack_until_dead',mob['id'])
        phase = p.block(ctx,pos)['properties']['trial_spawner_state']
        if phase in ('waiting_for_reward_ejection', 'ejecting_reward') and reward_start is None:
            reward_start = len(p.observe(ctx,'entity_events'))
        if phase == 'cooldown':
            break
        p.ticks(ctx,20)
    else:
        raise AssertionError('trial did not complete within 10000 simulation ticks')
    events = p.observe(ctx,'entity_events')[start_events:]
    if reward_start is None:
        raise AssertionError('trial reward phase was not observed')
    drops = [e for e in p.observe(ctx,'entity_events')[reward_start:]
             if e['event']=='spawn' and e['type']=='minecraft:item']
    ominous = [e for e in events if e['event']=='spawn' and e['type']=='minecraft:ominous_item_spawner']
    return {'spawned':len(seen),'max_alive':max_alive,
            'reward_rolls':len(drops),
            'ominous_item_spawners':len(ominous)}


def trial_spawner_waves(ctx):
    pos = trial(ctx)
    participants = min(2, len(ctx.clients))
    # Do not force survival or normal: peaceful/creative rows must assert inactivity.
    active = ctx.factors['difficulty'] != 'peaceful' and ctx.factors['game_mode'] in ('survival','adventure')
    if not active:
        p.ticks(ctx, 100)
        p.query(ctx, f'data get block {pos[0]} {pos[1]} {pos[2]} total_mobs_spawned 1',
                r'after scale factor of 1\.00 is 0\b')
        return
    for i in range(participants):
        p.client(ctx, i).teleport_to(pos[0]-2,pos[1],pos[2]+i+1)
    result = defeat_waves(ctx, pos, 6+2*(participants-1))
    p.check(ctx, 'total default wave mobs', result['spawned'], 6+2*(participants-1))
    p.test(ctx, 'simultaneous limit', result['max_alive'] <= 2+(participants-1), result)
    p.check(ctx, 'one reward per admitted player', result['reward_rolls'], participants)
    p.block_is(ctx, pos, 'minecraft:trial_spawner', trial_spawner_state='cooldown')
    # The native block-state trace supplies published transition ticks; starting
    # a 35999-tick relative wait after a command reply would race cooldown onset.
    trace = p.rich_action(ctx, 'block_state_trace', pos,
                          property='trial_spawner_state', ticks=36001)
    p.check(ctx, 'cooldown lasts 36000 ticks', trace['durations']['cooldown'], 36000)


def ominous_trials(ctx):
    pos = trial(ctx)
    p.admin(ctx, f'effect give {p.client(ctx).name} minecraft:trial_omen 120 0')
    p.client(ctx).teleport_to(pos[0]-2,pos[1],pos[2])
    p.ticks(ctx, 40)
    p.block_is(ctx, pos, 'minecraft:trial_spawner', ominous='true')
    result = defeat_waves(ctx, pos, 64)
    p.test(ctx, 'ominous waves spawn', result['spawned'] > 0, result)
    p.test(ctx, 'ominous item spawner observed', result['ominous_item_spawners'] > 0, result)
    p.test(ctx, 'ominous reward emitted', result['reward_rolls'] > 0, result)


ADAPTERS = {'sculk_trials.' + fn.__name__: Adapter(p.setup, fn, p.oracle) for fn in (
    sculk_sensor, calibrated_sensor, sculk_catalyst_spread, shrieker_warden_summon,
    warden_behaviour, trial_spawner_waves, ominous_trials)}
