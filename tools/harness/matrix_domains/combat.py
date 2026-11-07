"""Public melee, ranged, defense and player-vs-player behavioral probes."""
from functools import partial
from ..matrix_api import MissingCapability
from ._items import (
    adapter, check, client, command, condition, count, entities, give, health,
    number, observe, require_verbs, select, summon, survival, ticks, verb, WIKI,
)

SOURCE = WIKI + 'Damage'
TARGET = '@e[type=iron_golem,name=MatrixTarget,limit=1]'


def target(ctx):
    command(ctx, 'kill @e[type=iron_golem,name=MatrixTarget]')
    return summon(ctx, 'iron_golem', nbt='{CustomName:"MatrixTarget",NoAI:1b}', pos='2.5 180 0.5')


def hit(ctx, entity, wait=25):
    ticks(ctx, wait)
    client(ctx).look_at(2.5, 181, .5)
    verb(ctx, 'attack', entity)
    ticks(ctx, 1)


def melee(ctx):
    require_verbs(ctx, 'jump')
    give(ctx, 'diamond_sword')
    select(ctx, 'diamond_sword')
    mob = target(ctx)
    before = number(ctx, TARGET, 'Health')
    hit(ctx, mob)
    full = before - number(ctx, TARGET, 'Health')
    check(ctx, 'fully_charged_diamond_sword', full, 7, source=WIKI + 'Sword')
    # A different target avoids conflating attack cooldown with hurt immunity.
    second = summon(ctx, 'iron_golem', nbt='{CustomName:"MatrixSecond",NoAI:1b}', pos='2.5 180 2.5')
    before = number(ctx, '@e[name=MatrixSecond,limit=1]', 'Health')
    verb(ctx, 'attack', second)
    partial_damage = before - number(ctx, '@e[name=MatrixSecond,limit=1]', 'Health')
    condition(ctx, 'partial_cooldown_lower_damage', 0 < partial_damage < full, partial_damage, source=WIKI + 'Damage#Attack_cooldown')
    mob = target(ctx)
    ticks(ctx, 25)
    verb(ctx, 'jump', ticks=1)
    ctx.wait_until(lambda: observe(ctx, 'velocity')[1] < 0 and not observe(ctx, 'on_ground'), description='falling critical-hit phase')
    before = number(ctx, TARGET, 'Health')
    verb(ctx, 'attack', mob)
    check(ctx, 'falling_critical_multiplier', before - number(ctx, TARGET, 'Health'), full * 1.5, source=WIKI + 'Critical_hit')
    ctx.wait_until(lambda: observe(ctx, 'on_ground'), description='land before sweep')
    mob = target(ctx)
    command(ctx, 'tp @e[name=MatrixSecond,limit=1] 2.5 180 1.0')
    before = number(ctx, '@e[name=MatrixSecond,limit=1]', 'Health')
    hit(ctx, mob)
    condition(ctx, 'sweep_hits_neighbour', number(ctx, '@e[name=MatrixSecond,limit=1]', 'Health') < before, before, source=WIKI + 'Sweeping_attack')


def knockback(ctx):
    require_verbs(ctx, 'sprint')
    mob = target(ctx)
    before = number(ctx, TARGET, 'Pos[0]')
    verb(ctx, 'sprint', enabled=True)
    hit(ctx, mob)
    ticks(ctx, 5)
    condition(ctx, 'sprint_hit_moves_target', number(ctx, TARGET, 'Pos[0]') != before, number(ctx, TARGET, 'Pos[0]'), source=WIKI + 'Knockback')
    verb(ctx, 'sprint', enabled=False)
    give(ctx, 'diamond_sword[enchantments={"minecraft:knockback":2}]')
    select(ctx, 'diamond_sword')
    mob = target(ctx)
    hit(ctx, mob)
    velocity = next(e['velocity'] for e in entities(ctx, 'iron_golem') if e['id'] == mob)
    condition(ctx, 'knockback_enchant_impulse', any(abs(v) > 0 for v in velocity), velocity, source='local:data/vanilla/data/minecraft/enchantment/knockback.json')


def second_player(ctx):
    if len(ctx.clients) < 2:
        raise MissingCapability('context:second_client')
    victim = ctx.clients[1]
    command(ctx, f'tp {victim.name} 2.5 180 0.5')
    command(ctx, f'clear {victim.name}')
    command(ctx, f'effect clear {victim.name}')
    return victim


def victim_id(ctx, victim):
    return next(e['id'] for e in observe(ctx, 'entities') if e.get('name') == victim.name)


def pvp(ctx):
    victim = second_player(ctx)
    survival(ctx)
    command(ctx, 'gamerule natural_regeneration false')
    for enabled in (False, True):
        command(ctx, 'gamerule pvp ' + str(enabled).lower())
        before = number(ctx, victim.name, 'Health')
        hit(ctx, victim_id(ctx, victim))
        check(ctx, 'pvp_' + str(enabled), before - number(ctx, victim.name, 'Health'), 1 if enabled else 0, source=WIKI + 'Player_versus_Player')
    command(ctx, 'team add MatrixTeam')
    command(ctx, f'team join MatrixTeam {client(ctx).name}')
    command(ctx, f'team join MatrixTeam {victim.name}')
    command(ctx, 'team modify MatrixTeam friendlyFire false')
    before = number(ctx, victim.name, 'Health')
    hit(ctx, victim_id(ctx, victim))
    check(ctx, 'team_friendly_fire_disabled', number(ctx, victim.name, 'Health'), before, source=WIKI + 'Commands/team')


def armor(ctx):
    victim = second_player(ctx)
    survival(ctx)
    command(ctx, 'gamerule natural_regeneration false')
    give(ctx, 'diamond_sword')
    select(ctx, 'diamond_sword')
    for material, points, toughness in (('leather', 7, 0), ('golden', 11, 0), ('chainmail', 12, 0), ('iron', 15, 0), ('diamond', 20, 8), ('netherite', 20, 12)):
        command(ctx, f'clear {victim.name}')
        for piece in ('boots', 'leggings', 'chestplate', 'helmet'):
            command(ctx, f'give {victim.name} {material}_{piece}')
            stack = next(s for s in victim.snapshot()['inventory'] if s['item_id'] == f'minecraft:{material}_{piece}')
            victim.select_hotbar(stack['slot'] - 36)
            method = getattr(victim, 'use_item', None)
            if not callable(method):
                raise MissingCapability('client:use_item')
            method(hand='main')
        command(ctx, f'effect give {victim.name} instant_health 1 10')
        before = number(ctx, victim.name, 'Health')
        hit(ctx, victim_id(ctx, victim))
        reduction = min(20, max(points / 5, points - 7 / (2 + toughness / 4)))
        expected = 7 * (1 - reduction / 25)
        check(ctx, material + '_armor_formula', before - number(ctx, victim.name, 'Health'), expected, source=WIKI + 'Armor#Damage_protection', tolerance=1e-4)
        for stack in victim.snapshot()['inventory']:
            if 5 <= stack['slot'] <= 8:
                if 'components' not in stack:
                    raise MissingCapability('snapshot:stack:components')
                check(ctx, material + '_armor_wear', stack['components'].get('minecraft:damage'), 1, source=WIKI + 'Armor')


def shield_vs_axe(ctx):
    victim = second_player(ctx)
    survival(ctx)
    command(ctx, 'gamerule natural_regeneration false')
    command(ctx, f'give {victim.name} shield')
    shield_slot = next(s['slot'] for s in victim.snapshot()['inventory'] if s['item_id'] == 'minecraft:shield')
    victim.select_hotbar(shield_slot - 36)
    victim.look_at(0.5, 181.5, .5)
    use = getattr(victim, 'use_item', None)
    if not callable(use):
        raise MissingCapability('client:use_item')
    use(hand='main', ticks=6)
    before = number(ctx, victim.name, 'Health')
    hit(ctx, victim_id(ctx, victim))
    check(ctx, 'frontal_melee_blocked', number(ctx, victim.name, 'Health'), before, source=WIKI + 'Shield')
    give(ctx, 'diamond_axe')
    select(ctx, 'diamond_axe')
    hit(ctx, victim_id(ctx, victim), wait=25)
    cooldowns = victim.snapshot().get('cooldowns')
    if cooldowns is None:
        raise MissingCapability('snapshot:cooldowns')
    condition(ctx, 'axe_shield_disable_cooldown', 0 < cooldowns.get('minecraft:shield', 0) <= 100, cooldowns, source=WIKI + 'Shield')
    ticks(ctx, 101)
    cooldowns = victim.snapshot().get('cooldowns')
    check(ctx, 'shield_recovers_after_100_ticks', cooldowns.get('minecraft:shield', 0), 0, source=WIKI + 'Shield')


def damage_enchants(ctx):
    for enchant, mob_kind, expected in (('sharpness', 'iron_golem', 8), ('smite', 'skeleton', 9.5), ('bane_of_arthropods', 'spider', 9.5)):
        command(ctx, f'clear {client(ctx).name}')
        give(ctx, f'diamond_sword[enchantments={{"minecraft:{enchant}":1}}]')
        select(ctx, 'diamond_sword')
        mob = summon(ctx, mob_kind, nbt='{CustomName:"EnchTarget",NoAI:1b}', pos='2.5 180 0.5')
        selector = f'@e[type={mob_kind},name=EnchTarget,limit=1]'
        before = number(ctx, selector, 'Health')
        hit(ctx, mob)
        check(ctx, enchant + '_bonus', before - number(ctx, selector, 'Health'), expected, source='local:data/vanilla/data/minecraft/enchantment/' + enchant + '.json', tolerance=1e-4)
        command(ctx, f'kill {selector}')
    command(ctx, f'clear {client(ctx).name}')
    give(ctx, 'diamond_sword[enchantments={"minecraft:fire_aspect":1}]')
    select(ctx, 'diamond_sword')
    mob = target(ctx)
    hit(ctx, mob)
    condition(ctx, 'fire_aspect_ignites', number(ctx, TARGET, 'Fire') > 0, number(ctx, TARGET, 'Fire'), source='local:data/vanilla/data/minecraft/enchantment/fire_aspect.json')


def thorns(ctx):
    from .damage import thorns as action
    ctx.state['health_before'] = health(ctx)
    ctx.state['damage_event_start'] = len(observe(ctx, 'damage_events'))
    action(ctx)


def bow(ctx):
    require_verbs(ctx, 'use_item', 'release_use_item')
    give(ctx, 'bow')
    give(ctx, 'arrow', 4)
    select(ctx, 'bow')
    mob = target(ctx)
    client(ctx).look_at(2.5, 181.5, .5)
    before = number(ctx, TARGET, 'Health')
    verb(ctx, 'use_item', hand='main', ticks=20)
    verb(ctx, 'release_use_item')
    ticks(ctx, 10)
    check(ctx, 'bow_consumes_one_arrow', count(ctx, 'arrow'), 4 if ctx.factors['game_mode'] == 'creative' else 3, source=WIKI + 'Bow')
    condition(ctx, 'bow_hits_target', number(ctx, TARGET, 'Health') < before, number(ctx, TARGET, 'Health'), source=WIKI + 'Bow')
    command(ctx, f'clear {client(ctx).name} bow')
    give(ctx, 'bow[enchantments={"minecraft:infinity":1,"minecraft:power":1,"minecraft:punch":1,"minecraft:flame":1}]')
    select(ctx, 'bow')
    ammo = count(ctx, 'arrow')
    verb(ctx, 'use_item', hand='main', ticks=20)
    verb(ctx, 'release_use_item')
    ticks(ctx, 10)
    check(ctx, 'infinity_keeps_arrow', count(ctx, 'arrow'), ammo, source=WIKI + 'Infinity')
    condition(ctx, 'flame_arrow_ignites', number(ctx, TARGET, 'Fire') > 0, number(ctx, TARGET, 'Fire'), source=WIKI + 'Flame')


def crossbow(ctx):
    require_verbs(ctx, 'use_item', 'release_use_item')
    give(ctx, 'crossbow[enchantments={"minecraft:multishot":1,"minecraft:quick_charge":3}]')
    give(ctx, 'arrow', 4)
    select(ctx, 'crossbow')
    verb(ctx, 'use_item', hand='main', ticks=12)
    verb(ctx, 'release_use_item')
    stack = next(s for s in observe(ctx, 'inventory') if s['item_id'] == 'minecraft:crossbow')
    if 'components' not in stack:
        raise MissingCapability('snapshot:stack:components')
    check(ctx, 'multishot_charges_three_projectiles', len(stack['components'].get('minecraft:charged_projectiles', [])), 3, source=WIKI + 'Crossbow')
    before = len(observe(ctx, 'projectile_events'))
    client(ctx).look_at(8, 190, 0)
    verb(ctx, 'use_item', hand='main')
    events = observe(ctx, 'projectile_events')[before:]
    check(ctx, 'multishot_fires_three', sum(e['type'] == 'minecraft:arrow' for e in events), 3, source=WIKI + 'Multishot')
    check(ctx, 'multishot_costs_one_ammo', count(ctx, 'arrow'), 4 if ctx.factors['game_mode'] == 'creative' else 3, source=WIKI + 'Multishot')


def trident(ctx):
    require_verbs(ctx, 'use_item', 'release_use_item')
    give(ctx, 'trident[enchantments={"minecraft:loyalty":3}]')
    select(ctx, 'trident')
    client(ctx).look_at(6, 180, 0)
    verb(ctx, 'use_item', hand='main', ticks=12)
    verb(ctx, 'release_use_item')
    condition(ctx, 'trident_projectile_spawn', any(e['type'] == 'minecraft:trident' for e in observe(ctx, 'projectile_events')), observe(ctx, 'projectile_events'), source=WIKI + 'Trident')
    ctx.wait_until(lambda: count(ctx, 'trident') == 1, description='loyalty trident returns', timeout_seconds=30)
    check(ctx, 'loyalty_returns_exact_trident', count(ctx, 'trident'), 1, source=WIKI + 'Loyalty')


def mace(ctx):
    require_verbs(ctx, 'press_inputs')
    give(ctx, 'mace')
    select(ctx, 'mace')
    mob = target(ctx)
    before = number(ctx, TARGET, 'Health')
    command(ctx, f'tp {client(ctx).name} 2.5 190 0.5')
    verb(ctx, 'press_inputs', ticks=15)
    ctx.wait_until(lambda: observe(ctx, 'position')[1] < 184, description='mace enters smash reach')
    verb(ctx, 'attack', mob)
    condition(ctx, 'smash_exceeds_base_six', before - number(ctx, TARGET, 'Health') > 6, before - number(ctx, TARGET, 'Health'), source=WIKI + 'Mace')
    check(ctx, 'smash_resets_fall_distance', number(ctx, client(ctx).name, 'FallDistance'), 0, source=WIKI + 'Mace')


def invulnerability(ctx):
    mob = target(ctx)
    before = number(ctx, TARGET, 'Health')
    hit(ctx, mob)
    first = number(ctx, TARGET, 'Health')
    verb(ctx, 'attack', mob)
    check(ctx, 'immediate_weaker_hit_ignored', number(ctx, TARGET, 'Health'), first, source=WIKI + 'Damage#Immunity')
    ticks(ctx, 25)
    verb(ctx, 'attack', mob)
    condition(ctx, 'post_window_hit_admitted', number(ctx, TARGET, 'Health') < first < before, number(ctx, TARGET, 'Health'), source=WIKI + 'Damage#Immunity')


def projectile_pickup(ctx):
    for status in (0, 1, 2):
        command(ctx, f'clear {client(ctx).name} arrow')
        command(ctx, f'summon arrow 0.5 180 0.5 {{inGround:1b,pickup:{status}b}}')
        ticks(ctx, 20)
        mode = ctx.factors['game_mode']
        admitted = mode != 'spectator' and (status == 1 or (status == 2 and mode == 'creative'))
        # ALLOWED calls Inventory.add even in creative; CREATIVE_ONLY instead
        # consumes the entity without inserting an item (26.1.2 bytecode).
        expected = int(status == 1 and mode != 'spectator')
        source = 'local:scenarios/runs/DomItems/facts/AbstractArrow.javap:1752-1807'
        check(ctx, 'arrow_pickup_status_' + str(status), count(ctx, 'arrow'), expected, source=source)
        check(ctx, 'arrow_entity_removed_' + str(status), len(entities(ctx, 'arrow')), 0 if admitted else 1, source=source)
        command(ctx, 'kill @e[type=arrow]')


def mob_targets(ctx):
    mob = summon(ctx, 'zombie', pos='8.5 180 0.5')
    before = health(ctx)
    if ctx.factors['difficulty'] == 'peaceful' or ctx.factors['game_mode'] in ('creative', 'spectator'):
        ticks(ctx, 80)
        check(ctx, 'hostile_context_immunity', health(ctx), before, source=WIKI + 'Zombie')
    else:
        ctx.wait_until(lambda: health(ctx) < before, description='zombie acquires and attacks', timeout_seconds=30)
        condition(ctx, 'hostile_damage_received', health(ctx) < before, health(ctx), source=WIKI + 'Zombie')


ADAPTERS = {f'combat.{name}': adapter(fn) for name, fn in {
    'melee_cooldown_crit_sweep': melee, 'knockback': knockback, 'pvp': pvp,
    'armor_mitigation': armor, 'shield_vs_axe': shield_vs_axe, 'damage_enchants': damage_enchants,
    'thorns': thorns, 'bow': bow, 'crossbow': crossbow, 'trident': trident,
    'mace': mace, 'invulnerability_frames': invulnerability, 'projectile_pickup': projectile_pickup,
    'mob_targets_player': mob_targets,
}.items()}
