"""Actual damage sources, armor bypass and wire identity; no /damage substitute
for environment mechanics. /damage is used only for the explicit Generic fixture.
"""
from functools import partial
from ..matrix_api import MissingCapability
from ._items import (
    adapter, check, client, command, condition, give, health, number, observe,
    require_verbs, select, summon, survival, ticks, verb, WIKI,
)

SOURCE = WIKI + 'Damage'


def prepare(ctx):
    from ._items import setup
    setup(ctx)
    command(ctx, 'gamerule natural_regeneration false')
    command(ctx, f'effect give {client(ctx).name} instant_health 1 10')
    ctx.state['health_before'] = health(ctx)
    ctx.state['damage_event_start'] = len(observe(ctx, 'damage_events'))


def damage_observation(ctx, kind, amount=None, *, timeout=40):
    before = ctx.state['health_before']
    immune = ctx.factors['game_mode'] in ('creative', 'spectator') and kind != 'out_of_world'
    if immune:
        ticks(ctx, 40)
        check(ctx, 'mode_damage_immunity', health(ctx), before, source=SOURCE)
        return
    ctx.wait_until(lambda: health(ctx) < before, description='received authoritative damage health', timeout_seconds=timeout)
    after = health(ctx)
    if amount is not None:
        check(ctx, 'first_damage_amount', before - after, amount, source=SOURCE, tolerance=1e-4)
    else:
        condition(ctx, 'health_decreased', after < before, {'before': before, 'after': after}, source=SOURCE)
    events = observe(ctx, 'damage_events')[ctx.state['damage_event_start']:]
    check(ctx, 'damage_type_identity', any(e['type'] == 'minecraft:' + kind for e in events), True,
          source='local:data/vanilla/data/minecraft/damage_type/' + kind + '.json')


def environment(ctx, subject):
    entries = {
        'InFire': ('setblock 0 180 0 fire', 'in_fire', 1),
        'Lava': ('setblock 0 180 0 lava', 'lava', 4),
        'Suffocation': ('setblock 0 181 0 stone', 'in_wall', 1),
        'SweetBerryBush': ('setblock 0 180 0 sweet_berry_bush[age=3]', 'sweet_berry_bush', 1),
        'HotFloor': ('setblock 0 179 0 magma_block', 'hot_floor', 1),
        'Campfire': ('setblock 0 180 0 campfire[lit=true]', 'campfire', 1),
        'Cactus': ('setblock 1 180 0 cactus', 'cactus', 1),
    }
    fixture, kind, amount = entries[subject]
    if subject == 'Cactus':
        command(ctx, 'setblock 1 179 0 sand')
        command(ctx, fixture)
        command(ctx, f'tp {client(ctx).name} 0.75 180 0.5')
        require_verbs(ctx, 'move')
        verb(ctx, 'move', forward=0, strafe=1, ticks=10, sprint=False)
    elif subject == 'SweetBerryBush':
        command(ctx, fixture)
        require_verbs(ctx, 'move')
        verb(ctx, 'move', forward=1, strafe=0, ticks=10, sprint=False)
    else:
        command(ctx, fixture)
    damage_observation(ctx, kind, amount)
    if subject == 'HotFloor' and ctx.factors['game_mode'] in ('survival', 'adventure'):
        require_verbs(ctx, 'sneak')
        verb(ctx, 'sneak', enabled=True)
        before = health(ctx)
        ticks(ctx, 30)
        check(ctx, 'sneak_magma_immunity', health(ctx), before, source=WIKI + 'Magma_Block')
    if subject == 'Lava':
        command(ctx, f'effect give {client(ctx).name} fire_resistance 30 0')
        before = health(ctx)
        ticks(ctx, 30)
        check(ctx, 'fire_resistance_immunity', health(ctx), before, source=WIKI + 'Fire_Resistance')


def on_fire(ctx):
    command(ctx, 'setblock 0 180 0 fire')
    ticks(ctx, 2)
    command(ctx, 'setblock 0 180 0 air')
    ctx.state['health_before'] = health(ctx)
    ctx.state['damage_event_start'] = len(observe(ctx, 'damage_events'))
    damage_observation(ctx, 'on_fire', 1)


def drowning(ctx):
    command(ctx, 'fill -1 180 -1 1 183 1 water')
    damage_observation(ctx, 'drown', 2, timeout=50)
    command(ctx, f'effect give {client(ctx).name} water_breathing 30 0')
    before = health(ctx)
    ticks(ctx, 45)
    check(ctx, 'water_breathing_stops_drowning', health(ctx), before, source=WIKI + 'Water_Breathing')


def effect_damage(ctx, subject):
    effect = {'Poison': 'poison', 'Wither': 'wither', 'Magic': 'instant_damage'}[subject]
    command(ctx, f'effect give {client(ctx).name} {effect} 30 0')
    damage_observation(ctx, 'wither' if subject == 'Wither' else 'magic', 6 if subject == 'Magic' else 1)
    if subject == 'Poison' and ctx.factors['game_mode'] in ('survival', 'adventure'):
        ticks(ctx, 500)
        check(ctx, 'poison_nonlethal_floor', health(ctx), 1, source=WIKI + 'Poison')


def generic(ctx):
    response = command(ctx, f'damage {client(ctx).name} 4 minecraft:generic')
    if any(token in response.lower() for token in ('unknown', 'unsupported', 'not implemented')):
        raise MissingCapability('server:damage')
    damage_observation(ctx, 'generic', 4)


def fall(ctx, stalagmite=False):
    require_verbs(ctx, 'press_inputs')
    if stalagmite:
        command(ctx, 'setblock 0 180 0 pointed_dripstone[vertical_direction=up,thickness=tip]')
    command(ctx, f'tp {client(ctx).name} 0.5 190 0.5')
    verb(ctx, 'press_inputs', ticks=60)
    damage_observation(ctx, 'stalagmite' if stalagmite else 'fall')


def falling_block(ctx, stalactite=False):
    if stalactite:
        command(ctx, 'setblock 0 186 0 stone')
        command(ctx, 'setblock 0 185 0 pointed_dripstone[vertical_direction=down,thickness=tip]')
        command(ctx, 'setblock 0 186 0 air')
    else:
        command(ctx, 'setblock 0 185 0 anvil')
    damage_observation(ctx, 'falling_stalactite' if stalactite else 'falling_anvil')


def source_entity(ctx, subject):
    if subject == 'Lightning':
        command(ctx, 'summon lightning_bolt 0.5 180 0.5')
        damage_observation(ctx, 'lightning_bolt', 5)
    elif subject == 'Explosion':
        command(ctx, 'summon tnt 1.5 180 0.5 {fuse:1s}')
        damage_observation(ctx, 'explosion')
    elif subject == 'Melee':
        summon(ctx, 'zombie', pos='1.5 180 0.5')
        if ctx.factors['difficulty'] == 'peaceful':
            ticks(ctx, 40)
            check(ctx, 'peaceful_no_hostile_damage', health(ctx), ctx.state['health_before'], source=WIKI + 'Difficulty')
        else:
            damage_observation(ctx, 'mob_attack')
    elif subject == 'Projectile':
        command(ctx, 'summon arrow 0.5 181 4.5 {Motion:[0d,0d,-1d],damage:2d,pickup:0b}')
        damage_observation(ctx, 'arrow')
    elif subject == 'Fireball':
        command(ctx, 'summon small_fireball 0.5 181 4.5 {Motion:[0d,0d,-1d]}')
        damage_observation(ctx, 'fireball')


def thorns(ctx):
    target = summon(ctx, 'zombie', nbt='{NoAI:1b,equipment:{chest:{id:"minecraft:diamond_chestplate",count:1,components:{"minecraft:enchantments":{"minecraft:thorns":3}}}}}', pos='1.5 180 0.5')
    before = health(ctx)
    for _ in range(30):
        command(ctx, 'effect give @e[type=zombie,limit=1] instant_damage 1 4')
        verb(ctx, 'attack', target)
        ticks(ctx, 21)
        if health(ctx) < before:
            break
    damage_observation(ctx, 'thorns')


def starvation(ctx):
    survival(ctx)
    require_verbs(ctx, 'move')
    if ctx.factors['difficulty'] == 'peaceful':
        command(ctx, f'effect give {client(ctx).name} hunger 120 255')
        verb(ctx, 'move', forward=1, strafe=0, ticks=100, sprint=True)
        check(ctx, 'peaceful_no_starvation_damage', health(ctx), ctx.state['health_before'], source=WIKI + 'Hunger')
        return
    command(ctx, f'effect give {client(ctx).name} hunger 120 255')
    ticks(ctx, 200)
    check(ctx, 'starvation_zero_food', number(ctx, client(ctx).name, 'foodLevel'), 0, source=WIKI + 'Hunger')
    damage_observation(ctx, 'starve', 1, timeout=60)
    ticks(ctx, 1600)
    expected = {'easy': 10, 'normal': 1, 'hard': 0}[ctx.factors['difficulty']]
    check(ctx, 'difficulty_starvation_floor', health(ctx), expected, source=WIKI + 'Hunger')


def void(ctx):
    command(ctx, f'tp {client(ctx).name} 0 -200 0')
    damage_observation(ctx, 'out_of_world')


def freeze(ctx):
    command(ctx, 'fill -1 180 -1 1 182 1 powder_snow')
    damage_observation(ctx, 'freeze', timeout=40)


def cramming(ctx):
    command(ctx, 'gamerule max_entity_cramming 24')
    command(ctx, 'fill -1 180 -1 1 182 1 stone')
    command(ctx, 'fill 0 180 0 0 182 0 air')
    for _ in range(25):
        command(ctx, 'summon cow 0.5 180 0.5 {NoAI:1b}')
    damage_observation(ctx, 'cramming', timeout=30)


def sonic(ctx):
    command(ctx, 'fill 1 180 -2 1 184 2 stone')
    summon(ctx, 'warden', pos='5.5 180 0.5')
    require_verbs(ctx, 'move')
    for _ in range(6):
        verb(ctx, 'move', forward=1, strafe=0, ticks=20, sprint=True)
        verb(ctx, 'move', forward=-1, strafe=0, ticks=20, sprint=True)
    damage_observation(ctx, 'sonic_boom', timeout=90)


def dragon_breath(ctx):
    # The actual flaming-phase cloud carries instant damage. A particle-only
    # summoned cloud has no effects and cannot be used as a damaging fixture.
    # 26.1.2 AreaEffectCloud:645-652 -> HealOrHarmMobEffect:96 ->
    # DamageSources.indirectMagic (not the unused dragon_breath registry entry).
    from ._items import entities
    command(ctx, 'gamerule mob_griefing false')
    command(ctx, 'summon ender_dragon 0.5 180 0.5 {DragonPhase:5}')
    ctx.wait_until(lambda: entities(ctx, 'area_effect_cloud'),
                   description='dragon sitting-flaming cloud', timeout_seconds=40)
    pos = entities(ctx, 'area_effect_cloud')[0]['position']
    client(ctx).teleport_to(*pos)
    ctx.state['health_before'] = health(ctx)
    damage_observation(ctx, 'indirect_magic')


def fly_into_wall(ctx):
    require_verbs(ctx, 'use_item', 'press_inputs')
    give(ctx, 'elytra')
    select(ctx, 'elytra')
    verb(ctx, 'use_item', hand='main')
    command(ctx, 'fill -3 180 8 3 210 8 stone')
    command(ctx, f'tp {client(ctx).name} 0.5 205 0.5')
    client(ctx).look_at(0.5, 190, 8)
    verb(ctx, 'press_inputs', forward=True, jump=True, ticks=60)
    damage_observation(ctx, 'fly_into_wall')


ADAPTERS = {}
for subject in ('InFire', 'Lava', 'Suffocation', 'SweetBerryBush', 'HotFloor', 'Campfire', 'Cactus'):
    ADAPTERS['damage.' + subject] = adapter(partial(environment, subject=subject), prepare)
for subject in ('Poison', 'Wither', 'Magic'):
    ADAPTERS['damage.' + subject] = adapter(partial(effect_damage, subject=subject), prepare)
for subject in ('Lightning', 'Explosion', 'Melee', 'Projectile', 'Fireball'):
    ADAPTERS['damage.' + subject] = adapter(partial(source_entity, subject=subject), prepare)
ADAPTERS.update({f'damage.{name}': adapter(fn, prepare) for name, fn in {
    'Generic': generic, 'OnFire': on_fire, 'Drowning': drowning, 'Fall': fall,
    'FallingAnvil': falling_block, 'FallingStalactite': partial(falling_block, stalactite=True),
    'Thorns': thorns, 'Starvation': starvation, 'Void': void,
    'vanilla_freeze': freeze, 'vanilla_cramming': cramming, 'vanilla_sonic_boom': sonic,
    'vanilla_dragon_breath': dragon_breath, 'vanilla_stalagmite': partial(fall, stalagmite=True),
    'vanilla_fly_into_wall': fly_into_wall,
}.items()})
