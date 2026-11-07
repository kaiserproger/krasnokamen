"""Effect behavior and lifecycle, referenced to the 26.1.2 Mojang classes.

A packet containing an effect is not by itself a behavioral pass. Each adapter
executes a gameplay probe as well as expiry/milk checks (instant effects excepted).
"""
from functools import partial
import re

from ..matrix_api import MissingCapability
from ._items import (
    adapter, block_is, check, client, command, condition, consume, entities,
    give, health, number, observe, query, require_verbs, select, setup, summon,
    survival, ticks, verb, WIKI,
)

FACT = 'local:scenarios/runs/DomItems/facts/'


def effect_name(subject):
    return re.sub(r'(?<!^)(?=[A-Z])', '_', subject).lower()


def active(ctx, name):
    return next((e for e in observe(ctx, 'effects') if e['id'] == 'minecraft:' + name), None)


def apply(ctx, name, seconds=30, amplifier=1):
    if ctx.factors['permission'] == 'console':
        command(ctx, f'effect give {client(ctx).name} {name} {seconds} {amplifier}')
    else:
        client(ctx).command(f'effect give @s {name} {seconds} {amplifier}')
    if name not in ('instant_health', 'instant_damage', 'saturation'):
        ctx.wait_until(lambda: client(ctx).snapshot().get('effects') is not None and active(ctx, name) is not None,
                       description='effect application observed: ' + name)


def lifecycle(ctx, name):
    entry = active(ctx, name)
    condition(ctx, 'effect_visible_to_client', entry is not None, entry, source=WIKI + 'Effect')
    if entry is not None:
        check(ctx, 'effect_amplifier', entry['amplifier'], 1, source=WIKI + 'Effect')
        flags = entry.get('flags')
        if flags is None:
            raise MissingCapability('snapshot:effects:flags')
        check(ctx, 'local_effect_icon_flag', flags['show_icon'], True, source=WIKI + 'Effect')
        check(ctx, 'local_effect_particle_flag', flags['visible'], True, source=WIKI + 'Effect')
    if len(ctx.clients) > 1:
        from ..matrix_execution import offline_uuid
        actor_uuid = str(offline_uuid(client(ctx).name))
        def observer_effect():
            snapshot = ctx.clients[1].snapshot()
            names = snapshot.get('registries', {}).get('minecraft:mob_effect', {})
            if not names:
                raise MissingCapability('snapshot:registries:minecraft:mob_effect')
            for entity in observe(ctx, 'entities', observer=1):
                if str(entity['uuid']) != actor_uuid:
                    continue
                for effect_id, effect in entity.get('effects', {}).items():
                    if names.get(str(effect_id)) == 'minecraft:' + name:
                        return effect
            return None
        ctx.wait_until(lambda: observer_effect() is not None, description='observer receives actor effect ' + name)
        observed = observer_effect()
        check(ctx, 'observer_receives_visible_effect', observed['flags']['visible'], True, source=WIKI + 'Effect')
    require_verbs(ctx, 'use_item', 'release_use_item')
    give(ctx, 'milk_bucket')
    consume(ctx, 'milk_bucket')
    check(ctx, 'milk_clears_effect', active(ctx, name), None, source=WIKI + 'Milk_Bucket')
    apply(ctx, name, seconds=2)
    ticks(ctx, 42)
    check(ctx, 'effect_expires', active(ctx, name), None, source=WIKI + 'Effect')
    apply(ctx, name)
    client(ctx).command('effect clear @s')
    ctx.wait_until(lambda: active(ctx, name) is None, description='command clears effect')
    check(ctx, 'command_clear_removes_effect', active(ctx, name), None, source=WIKI + 'Commands/effect')


def attribute(ctx, name, attribute_id, base, amount, multiply=False):
    apply(ctx, name)
    attributes = observe(ctx, 'attributes')
    key = 'minecraft:' + attribute_id
    if key not in attributes:
        raise MissingCapability('snapshot:attributes:' + key)
    expected = base * (1 + amount * 2) if multiply else base + amount * 2
    if attribute_id == 'attack_damage':
        expected = max(0, expected)
    check(ctx, 'effective_attribute_' + attribute_id, attributes[key], expected,
          source=FACT + 'MobEffects.javap', tolerance=1e-6)
    lifecycle(ctx, name)


def instant(ctx, name):
    survival(ctx)
    command(ctx, f'gamerule natural_regeneration false')
    command(ctx, f'effect give {client(ctx).name} instant_health 1 10')
    if name == 'instant_health':
        command(ctx, f'effect give {client(ctx).name} instant_damage 1 0')
        before = health(ctx)
        apply(ctx, name, 1, 0)
        check(ctx, 'instant_health_four', health(ctx), min(20, before + 4), source=FACT + 'HealOrHarmMobEffect.javap')
    else:
        before = health(ctx)
        apply(ctx, name, 1, 0)
        check(ctx, 'instant_damage_six', health(ctx), before - 6, source=FACT + 'HealOrHarmMobEffect.javap')
    # The undead polarity is separately observable on a submerged, named zombie.
    command(ctx, 'summon zombie 5 180 0 {CustomName:"EffectUndead",NoAI:1b,Health:10f}')
    target = '@e[type=zombie,name=EffectUndead,limit=1]'
    command(ctx, f'effect give {target} {name} 1 0')
    expected = 4 if name == 'instant_health' else 14
    check(ctx, 'undead_polarity', number(ctx, target, 'Health'), expected, source=FACT + 'HealOrHarmMobEffect.javap')


def periodic(ctx, name):
    survival(ctx)
    command(ctx, 'gamerule natural_regeneration false')
    command(ctx, f'effect give {client(ctx).name} instant_health 1 10')
    if name == 'regeneration':
        command(ctx, f'effect give {client(ctx).name} instant_damage 1 1')
        apply(ctx, name)
        ticks(ctx, 310)
        check(ctx, 'regeneration_restores_capped_health', health(ctx), 20, source=FACT + 'RegenerationMobEffect.javap')
        lifecycle(ctx, name)
    elif name == 'poison':
        apply(ctx, name)
        ticks(ctx, 260)
        check(ctx, 'poison_stops_at_one', health(ctx), 1, source=FACT + 'PoisonMobEffect.javap')
        lifecycle(ctx, name)
    elif name == 'wither':
        require_verbs(ctx, 'respawn')
        apply(ctx, name)
        ctx.wait_until(lambda: health(ctx) <= 0, description='wither reaches lethal health', timeout_seconds=40)
        check(ctx, 'wither_can_kill', health(ctx), 0, source=FACT + 'WitherMobEffect.javap')
        verb(ctx, 'respawn')
        check(ctx, 'death_clears_wither', active(ctx, name), None, source=WIKI + 'Effect')
    elif name == 'hunger':
        before = number(ctx, client(ctx).name, 'foodExhaustionLevel')
        apply(ctx, name)
        ticks(ctx, 100)
        after = number(ctx, client(ctx).name, 'foodExhaustionLevel')
        condition(ctx, 'hunger_adds_exhaustion', after > before, {'before': before, 'after': after}, source=FACT + 'HungerMobEffect.javap')
        lifecycle(ctx, name)
    elif name == 'saturation':
        apply(ctx, name)
        ticks(ctx, 20)
        check(ctx, 'saturation_fills_food', number(ctx, client(ctx).name, 'foodLevel'), 20, source=FACT + 'SaturationMobEffect.javap')
        check(ctx, 'saturation_fills_saturation', number(ctx, client(ctx).name, 'foodSaturationLevel'), 20, source=FACT + 'SaturationMobEffect.javap')
        lifecycle(ctx, name)


def absorption(ctx):
    survival(ctx)
    apply(ctx, 'absorption')
    check(ctx, 'absorption_II_eight_hearts_points', number(ctx, client(ctx).name, 'AbsorptionAmount'), 8, source=FACT + 'MobEffects.javap')
    before = health(ctx)
    command(ctx, f'effect give {client(ctx).name} instant_damage 1 0')
    check(ctx, 'absorption_spent_first', number(ctx, client(ctx).name, 'AbsorptionAmount'), 2, source=WIKI + 'Absorption')
    check(ctx, 'health_not_spent', health(ctx), before, source=WIKI + 'Absorption')
    lifecycle(ctx, 'absorption')


def resistance(ctx):
    survival(ctx)
    command(ctx, 'gamerule natural_regeneration false')
    apply(ctx, 'resistance')
    before = health(ctx)
    command(ctx, f'effect give {client(ctx).name} instant_damage 1 0')
    check(ctx, 'resistance_II_reduces_40_percent', health(ctx), before - 3.6, source=WIKI + 'Resistance', tolerance=1e-4)
    lifecycle(ctx, 'resistance')


def fire_resistance(ctx):
    survival(ctx)
    apply(ctx, 'fire_resistance')
    before = health(ctx)
    command(ctx, 'setblock 0 180 0 lava')
    ticks(ctx, 45)
    check(ctx, 'lava_immunity', health(ctx), before, source=WIKI + 'Fire_Resistance')
    command(ctx, 'setblock 0 180 0 air')
    command(ctx, f'tp {client(ctx).name} 5.5 180 5.5')
    lifecycle(ctx, 'fire_resistance')


def breathing(ctx, name):
    survival(ctx)
    command(ctx, 'fill -1 180 -1 1 183 1 water')
    apply(ctx, name)
    before = number(ctx, client(ctx).name, 'Air')
    ticks(ctx, 100)
    after = number(ctx, client(ctx).name, 'Air')
    condition(ctx, 'air_not_depleted_underwater', after >= before, {'before': before, 'after': after}, source=WIKI + name.title())
    command(ctx, 'fill -1 180 -1 1 183 1 air')
    lifecycle(ctx, name)


def movement(ctx, name):
    survival(ctx)
    require_verbs(ctx, 'move', 'jump')
    if name == 'dolphins_grace':
        command(ctx, 'fill -2 180 -2 12 183 2 water')
        verb(ctx, 'move', forward=1, strafe=0, ticks=20, sprint=True)
        baseline = observe(ctx, 'position')
        command(ctx, f'tp {client(ctx).name} 0.5 180 0.5')
        apply(ctx, name)
        verb(ctx, 'move', forward=1, strafe=0, ticks=20, sprint=True)
        after = observe(ctx, 'position')
        distance = lambda p: ((p[0] - 0.5) ** 2 + (p[2] - 0.5) ** 2) ** 0.5
        condition(ctx, 'swimming_speed_increases', distance(after) > distance(baseline), {'baseline': baseline, 'effect': after}, source=WIKI + "Dolphin%27s_Grace")
        command(ctx, 'fill -2 180 -2 12 183 2 air')
    elif name == 'levitation':
        apply(ctx, name)
        y0 = observe(ctx, 'position')[1]
        ticks(ctx, 40)
        y1 = observe(ctx, 'position')[1]
        condition(ctx, 'levitation_raises_body', y1 > y0 + 1, {'before_y': y0, 'after_y': y1}, source=WIKI + 'Levitation')
    elif name == 'slow_falling':
        apply(ctx, name)
        before = health(ctx)
        command(ctx, f'tp {client(ctx).name} 0.5 195 0.5')
        ctx.wait_until(lambda: observe(ctx, 'position')[1] <= 180.1, description='slow falling reaches platform', timeout_seconds=40)
        check(ctx, 'slow_falling_prevents_fall_damage', health(ctx), before, source=WIKI + 'Slow_Falling')
    elif name == 'blindness':
        apply(ctx, name)
        verb(ctx, 'move', forward=1, strafe=0, ticks=10, sprint=True)
        check(ctx, 'blindness_disables_sprint', observe(ctx, 'sprinting'), False, source=WIKI + 'Blindness')
    lifecycle(ctx, name)


def visual(ctx, name):
    apply(ctx, name)
    # These are client-rendering behaviors, not server-only substitutes.
    render = observe(ctx, 'effect_rendering')
    if name not in render:
        raise MissingCapability('snapshot:effect_rendering:' + name)
    expected = {'nausea': 'screen_distortion', 'night_vision': 'full_brightness',
                'darkness': 'pulsating_darkness', 'glowing': 'outline_through_blocks',
                'invisibility': 'body_hidden_equipment_visible'}[name]
    check(ctx, 'render_effect', render[name], expected, source=WIKI + 'Effect')
    lifecycle(ctx, name)


def omen(ctx, name):
    if name in ('bad_omen', 'raid_omen', 'hero_of_the_village'):
        command(ctx, 'setblock 4 180 0 white_bed[part=foot,facing=south]')
        command(ctx, 'setblock 4 180 1 white_bed[part=head,facing=south]')
        command(ctx, 'setblock 5 180 0 composter')
        villager = summon(ctx, 'villager', nbt='{VillagerData:{profession:"minecraft:farmer",level:2,type:"minecraft:plains"}}')
        ticks(ctx, 100)
    if name == 'hero_of_the_village':
        require_verbs(ctx, 'interact_entity', 'close_container')
        verb(ctx, 'interact_entity', entity_id=villager, hand='main')
        prices = [offer['buy']['count'] for offer in observe(ctx, 'merchant_offers')]
        verb(ctx, 'close_container')
        apply(ctx, name)
        verb(ctx, 'interact_entity', entity_id=villager, hand='main')
        discounted = [offer['buy']['count'] for offer in observe(ctx, 'merchant_offers')]
        condition(ctx, 'hero_discounts_offers', len(prices) == len(discounted) and any(b < a for a, b in zip(prices, discounted)), {'before': prices, 'after': discounted}, source=WIKI + 'Hero_of_the_Village')
        verb(ctx, 'close_container')
    elif name == 'bad_omen':
        apply(ctx, name)
        command(ctx, f'tp {client(ctx).name} 4.5 180 0.5')
        ticks(ctx, 20)
        condition(ctx, 'bad_omen_converts_to_raid_omen', active(ctx, 'raid_omen') is not None, observe(ctx, 'effects'), source=WIKI + 'Bad_Omen')
        command(ctx, f'effect clear {client(ctx).name}')
        return
    elif name == 'raid_omen':
        apply(ctx, name)
        ticks(ctx, 605)
        condition(ctx, 'raid_bossbar_started', any('raid' in str(b).lower() for b in observe(ctx, 'bossbars')), observe(ctx, 'bossbars'), source=WIKI + 'Raid_Omen')
        return
    else:
        command(ctx, 'setblock 4 180 0 trial_spawner')
        apply(ctx, name)
        ticks(ctx, 100)
        check(ctx, 'trial_spawner_becomes_ominous', block_is(ctx, (4, 180, 0), 'trial_spawner[ominous=true]'), True, source=WIKI + 'Trial_Omen')
    lifecycle(ctx, name)


def death_trigger(ctx, name):
    cow = summon(ctx, 'cow', nbt='{CustomName:"EffectCow",NoAI:1b}')
    target = '@e[type=cow,name=EffectCow,limit=1]'
    command(ctx, f'effect give {target} {name} 30 0')
    if name == 'wind_charged':
        victim = summon(ctx, 'pig', pos='5 180 0.5', nbt='{NoAI:1b}')
        before = next(e['position'] for e in entities(ctx, 'pig') if e['id'] == victim)
    command(ctx, f'kill {target}')
    ticks(ctx, 22)
    if name == 'oozing':
        check(ctx, 'oozing_spawns_two_slimes', len(entities(ctx, 'slime')), 2, source=FACT + 'OozingMobEffect.javap')
    elif name == 'weaving':
        webs = sum(block_is(ctx, (x, y, z), 'cobweb') for x in range(3, 6) for y in range(179, 182) for z in range(-1, 2))
        condition(ctx, 'weaving_places_cobwebs', 1 <= webs <= 3, webs, source=FACT + 'WeavingMobEffect.javap')
    else:
        after = next(e['position'] for e in entities(ctx, 'pig') if e['id'] == victim)
        condition(ctx, 'death_wind_moves_neighbour', after != before, {'before': before, 'after': after}, source=WIKI + 'Wind_Charged')


def infested(ctx):
    target = '@e[type=cow,name=EffectCow,limit=1]'
    summon(ctx, 'cow', nbt='{CustomName:"EffectCow",NoAI:1b}')
    command(ctx, f'effect give {target} infested 300 0')
    # Bounded stochastic trial: 100 independent 10% events; no-event likelihood
    # is 0.9^100. Every observed burst must independently satisfy 1..3.
    total = 0
    for _ in range(100):
        command(ctx, f'effect give {target} instant_health 1 4')
        ticks(ctx, 11)
        command(ctx, f'effect give {target} instant_damage 1 0')
        ticks(ctx, 2)
        current = len(entities(ctx, 'silverfish'))
        condition(ctx, 'infested_burst_bounds', 0 <= current - total <= 3, current - total, source=FACT + 'InfestedMobEffect.javap')
        total = current
    condition(ctx, 'infested_has_triggered_in_100_hits', total > 0, {'total': total, 'false_negative_probability': 0.9 ** 100}, source=FACT + 'InfestedMobEffect.javap')


ADAPTERS = {}
for subject, attr, base, delta, mult in (
    ('Speed', 'movement_speed', .1, .2, True),
    ('Slowness', 'movement_speed', .1, -.15, True),
    ('Haste', 'attack_speed', 4., .1, True),
    ('MiningFatigue', 'attack_speed', 4., -.1, True),
    ('Strength', 'attack_damage', 1., 3., False),
    ('Weakness', 'attack_damage', 1., -4., False),
    ('JumpBoost', 'safe_fall_distance', 3., 1., False),
    ('HealthBoost', 'max_health', 20., 4., False),
    ('Luck', 'luck', 0., 1., False), ('Unluck', 'luck', 0., -1., False),
):
    ADAPTERS['effects.' + subject] = adapter(partial(attribute, name=effect_name(subject), attribute_id=attr, base=base, amount=delta, multiply=mult))
for subject in ('InstantHealth', 'InstantDamage'):
    ADAPTERS['effects.' + subject] = adapter(partial(instant, name=effect_name(subject)))
for subject in ('Regeneration', 'Poison', 'Wither', 'Hunger', 'Saturation'):
    ADAPTERS['effects.' + subject] = adapter(partial(periodic, name=effect_name(subject)))
for subject in ('WaterBreathing', 'ConduitPower', 'BreathOfTheNautilus'):
    ADAPTERS['effects.' + subject] = adapter(partial(breathing, name=effect_name(subject)))
for subject in ('DolphinsGrace', 'Levitation', 'SlowFalling', 'Blindness'):
    ADAPTERS['effects.' + subject] = adapter(partial(movement, name=effect_name(subject)))
for subject in ('Nausea', 'NightVision', 'Darkness', 'Glowing', 'Invisibility'):
    ADAPTERS['effects.' + subject] = adapter(partial(visual, name=effect_name(subject)))
for subject in ('BadOmen', 'RaidOmen', 'TrialOmen', 'HeroOfTheVillage'):
    ADAPTERS['effects.' + subject] = adapter(partial(omen, name=effect_name(subject)))
for subject in ('WindCharged', 'Weaving', 'Oozing'):
    ADAPTERS['effects.' + subject] = adapter(partial(death_trigger, name=effect_name(subject)))
ADAPTERS.update({'effects.Absorption': adapter(absorption), 'effects.Resistance': adapter(resistance),
                 'effects.FireResistance': adapter(fire_resistance), 'effects.Infested': adapter(infested)})


def authorized_action(action, ctx):
    permission = ctx.factors['permission']
    level = 4 if permission == 'console' else int(permission.rsplit('_', 1)[-1])
    if level < 2:
        name = effect_name(ctx.row['subject'].split('.', 1)[1])
        client(ctx).command(f'effect give @s {name} 30 1')
        ticks(ctx, 2)
        check(ctx, 'operator_permission_rejects_effect_command', active(ctx, name), None,
              source=WIKI + 'Commands/effect')
        return
    action(ctx)


from ..matrix_api import Adapter
ADAPTERS = {name: Adapter(value.setup, partial(authorized_action, value.act), value.oracle)
            for name, value in ADAPTERS.items()}
