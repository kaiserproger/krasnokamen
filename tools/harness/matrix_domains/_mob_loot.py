"""Loot-count bounds derived from the installed 26.1.2 vanilla data, not Krasnokamen.

Fixture: adult, no vehicle/equipment, not burning, no looting, white unsheared
sheep, size-one slime. Probability is not asserted from one random sample.
Unknown predicates/functions block rather than widening the accepted result.
"""
import json
from pathlib import Path
from ..matrix_api import MissingCapability


def _range(value):
    if isinstance(value, (int, float)):
        return (int(value), int(value))
    if value.get('type') == 'minecraft:uniform':
        return (int(value['min']), int(value['max']))
    raise MissingCapability('vanilla_loot_number:' + str(value))


def _condition(value, player_kill):
    kind = value['condition'].removeprefix('minecraft:')
    if kind == 'killed_by_player':
        return (player_kill, player_kill)
    if kind in {'random_chance', 'random_chance_with_enchanted_bonus'}:
        return (False, True)
    if kind == 'any_of':
        terms = [_condition(v, player_kill) for v in value['terms']]
        return (any(v[0] for v in terms), any(v[1] for v in terms))
    if kind == 'inverted':
        low, high = _condition(value['term'], player_kill)
        return (not high, not low)
    if kind == 'damage_source_properties':
        # The fixture dies to player melee or /kill, never frog/projectile/lightning.
        predicate = value['predicate']
        if any(key in predicate for key in ('source_entity', 'direct_entity', 'tags')):
            return (False, False)
    if kind == 'entity_properties':
        p = value.get('predicate', {})
        if not p:
            return (True, True)
        if value['entity'] != 'this':
            return (False, False)  # no skeleton attacker or enchanted main hand
        if 'vehicle' in p or p.get('flags', {}).get('is_on_fire') or p.get('flags', {}).get('is_baby'):
            return (False, False)
        specific = p.get('type_specific', {})
        if specific.get('type') == 'minecraft:raider':
            return (False, False)  # summoned, not a patrol captain
        if specific.get('type') == 'minecraft:slime':
            size = specific.get('size')
            passed = size == 1 if isinstance(size, int) else size.get('min', 0) <= 1 <= size.get('max', 1)
            return (passed, passed)
        if specific.get('type') == 'minecraft:sheep':
            passed = p.get('components', {}).get('minecraft:sheep/color') == 'white' and not specific.get('sheared', False)
            return (passed, passed)
    raise MissingCapability('vanilla_loot_condition:' + json.dumps(value, sort_keys=True))


def _conditions(values, player_kill):
    pairs = [_condition(v, player_kill) for v in values]
    return (all(v[0] for v in pairs), all(v[1] for v in pairs))


def _entry(entry, root, player_kill):
    certain, possible = _conditions(entry.get('conditions', []), player_kill)
    if not possible:
        return {}
    kind = entry['type'].removeprefix('minecraft:')
    if kind == 'empty':
        return {}
    if kind == 'alternatives':
        for child in entry['children']:
            _, allowed = _conditions(child.get('conditions', []), player_kill)
            if allowed:
                return _entry(child, root, player_kill)
        return {}
    if kind == 'loot_table':
        table = entry['value']
        if not isinstance(table, str):
            raise MissingCapability('vanilla_inline_loot_table')
        result = bounds(root, table.removeprefix('minecraft:'), player_kill)
    elif kind == 'item':
        low, high = 1, 1
        for function in entry.get('functions', []):
            _, enabled = _conditions(function.get('conditions', []), player_kill)
            if not enabled:
                continue
            f = function['function'].removeprefix('minecraft:')
            if f == 'set_count':
                a, b = _range(function['count'])
                low, high = (low+a, high+b) if function.get('add', False) else (a, b)
            elif f == 'enchanted_count_increase':
                continue  # explicitly zero looting
            elif f in {'set_potion', 'set_ominous_bottle_amplifier'}:
                continue  # count bounds only; component parity is a separate oracle
            else:
                raise MissingCapability('vanilla_loot_function:' + f)
        # Vanilla discards empty/nonpositive stacks. Spider eyes and wither
        # skeleton coal deliberately roll a negative minimum to encode no drop.
        result = {entry['name']: (max(0, low), max(0, high))}
    else:
        raise MissingCapability('vanilla_loot_entry:' + kind)
    if not certain:
        result = {key: (0, value[1]) for key, value in result.items()}
    return result


def bounds(root, table, player_kill):
    path = Path(root) / 'data/minecraft/loot_table' / (table + '.json')
    if not path.is_file():
        raise MissingCapability('vanilla_loot_table:' + table)
    data = json.loads(path.read_text())
    result = {}
    for pool in data.get('pools', []):
        certain, possible = _conditions(pool.get('conditions', []), player_kill)
        if not possible:
            continue
        roll_min, roll_max = _range(pool['rolls'])
        alternatives = [_entry(e, root, player_kill) for e in pool['entries']]
        keys = set().union(*(a.keys() for a in alternatives))
        for key in keys:
            low = min(a.get(key, (0, 0))[0] for a in alternatives) * roll_min if certain else 0
            high = max(a.get(key, (0, 0))[1] for a in alternatives) * roll_max
            old_low, old_high = result.get(key, (0, 0))
            result[key] = (old_low + low, old_high + high)
    return result
