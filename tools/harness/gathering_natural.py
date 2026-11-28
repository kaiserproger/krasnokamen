"""Non-worker D54 causes: periodic outputs, death loot, leaf decay and expiry.

Only bounded matching physical transactions are credited. Unwitnessed sources
remain residuals in gathering_audit; an item birth by itself is not a cause.
"""
from collections import Counter
import math

# Native ordinary, non-player-looting death outputs. Equipment and unsupported
# mobs deliberately receive no credit without a corresponding source witness.
DEATH_OUTPUTS = {
    'minecraft:zombie': {'minecraft:rotten_flesh': 2},
    'minecraft:drowned': {'minecraft:rotten_flesh': 2},
    'minecraft:zombie_villager': {'minecraft:rotten_flesh': 2},
    'minecraft:spider': {'minecraft:string': 2},
    'minecraft:cat': {'minecraft:string': 2},
    'minecraft:pig': {'minecraft:porkchop': 3, 'minecraft:cooked_porkchop': 3},
    'minecraft:enderman': {'minecraft:ender_pearl': 1},
    'minecraft:iron_golem': {'minecraft:iron_ingot': 5, 'minecraft:poppy': 2},
    'minecraft:skeleton': {'minecraft:bone': 2, 'minecraft:arrow': 2},
    'minecraft:creeper': {'minecraft:gunpowder': 2},
    'minecraft:sheep': {'minecraft:mutton': 2, 'minecraft:cooked_mutton': 2,
                        **{'minecraft:' + color + '_wool': 1 for color in
                           ('white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime',
                            'pink', 'gray', 'light_gray', 'cyan', 'purple', 'blue',
                            'brown', 'green', 'red', 'black')}},
}


def chunk(position):
    return math.floor(position[0]) // 16, math.floor(position[2]) // 16


def game_at(clock, tick):
    return tick - clock['running']['held_ticks'] if 'running' in clock else clock['held']['game_ticks']


class NaturalObserver:
    def __init__(self, before, names, states, entity_types):
        self.entities = {key: dict(value) for key, value in before['dimensions']['minecraft:overworld']['entities'].items()}
        self.debt = {tuple(row['chunk']): row for row in before['globals']['checkpoint'].get('debt', [])}
        self.clock = before['globals']['checkpoint']['climate']['game_clock']
        self.game_tick = game_at(self.clock, before['checkpoint_tick'])
        self.names, self.states, self.types = names, states, entity_types
        self.produced, self.consumed = Counter(), Counter()
        self.births, self.expiry = [], []

    def consume(self, entries, filename, tick, block_before):
        old = self.entities
        images, explicit_ages, removed, births = {}, set(), set(), []
        debt_images = {tuple(v['image']['chunk']): v['image'] for k, v in entries if k == 'SimDebt'}
        for kind, value in entries:
            if kind == 'Climate':
                self.clock = value['value']['game_clock']
        game_tick = game_at(self.clock, tick)
        advance = game_tick - self.game_tick
        if advance not in (0, 1):
            raise ValueError(f'non-unit simulation clock advance: {filename}: {advance}')
        self.game_tick = game_tick
        running = advance == 1
        for kind, value in entries:
            key = str(value.get('id'))
            if kind == 'Entity':
                images[key] = dict(value)
                explicit_ages.add(key)
                if key not in old and value.get('item'):
                    births.append(value)
            elif kind in ('EntityState', 'EntityAppearance', 'ItemMotion') and key in old:
                image = images.setdefault(key, dict(old[key]))
                if kind == 'EntityState':
                    image['state'] = value['state']
                    image['age'] = value['state']['age']
                    explicit_ages.add(key)
                else:
                    image.update({k: v for k, v in value.items() if k != 'id'})
                if kind == 'ItemMotion' or 'age' in value:
                    explicit_ages.add(key)
            elif kind == 'RemoveEntity':
                removed.add(key)
        sources = []
        for key, image in images.items():
            previous = old.get(key)
            if not previous or image.get('item'):
                continue
            first, last = previous.get('state', {}), image.get('state', {})
            source_type = self.types.get(image.get('type_id'))
            if (not image.get('alive', True) and last.get('loot', {}).get('death_handled')
                    and not first.get('loot', {}).get('death_handled')):
                sources.append((key, image, 'death_loot', DEATH_OUTPUTS.get(source_type, {})))
            first_harvest = first.get('entity_life', {}).get('harvest_ticks', 0)
            last_harvest = last.get('entity_life', {}).get('harvest_ticks', 0)
            elapsed = max(0, last.get('age', 0) - first.get('age', 0))
            projected = max(0, first_harvest - elapsed)
            if (source_type == 'minecraft:chicken' and image.get('alive', True)
                    and first_harvest > 0 and projected < last_harvest
                    and 6000 <= last_harvest < 12000):
                sources.append((key, image, 'chicken_egg_timer_reset', {'minecraft:egg': 1}))
            first_scute = first.get('entity_life', {}).get('armadillo') or {}
            last_scute = last.get('entity_life', {}).get('armadillo') or {}
            if (source_type == 'minecraft:armadillo' and image.get('alive', True)
                    and last_scute.get('drops', 0) == first_scute.get('drops', 0) + 1):
                sources.append((key, image, 'armadillo_scute_counter', {'minecraft:armadillo_scute': 1}))
        used = Counter()
        for birth in births:
            item = self.names[birth['item']['item_id']]
            count = birth['item']['count']
            candidates = []
            for key, image, cause, limits in sources:
                same_position = image['pos'] == birth['pos']
                if (0 < count <= limits.get(item, 0) - used[key, item] and same_position):
                    candidates.append({'source_uuid': image['uuid'], 'source_id': key,
                                       'cause': cause, 'state_before': old[key]['state'],
                                       'state_after': image['state']})
            for kind, value in entries:
                if kind != 'Block':
                    continue
                at = tuple(value['pos'])
                prior = block_before.get(at)
                if prior is None:
                    continue
                name, _ = self.states[prior]
                following, _ = self.states[value['state']]
                limit = (2 if item == 'minecraft:stick' else
                         1 if item == name.removesuffix('_leaves') + '_sapling' else 0)
                if (name.endswith('_leaves') and following == 'minecraft:air'
                        and 0 < count <= limit - used['leaf', at, item]
                        and all(abs(birth['pos'][i] - at[i] - 0.5) < 0.001 for i in range(3))):
                    candidates.append({'cause': 'leaf_decay', 'block': list(at), 'state_before': prior})
            if len(candidates) == 1:
                witness = candidates[0]
                if witness['cause'] == 'leaf_decay':
                    used['leaf', tuple(witness['block']), item] += count
                used[witness.get('source_id'), item] += count
                self.produced[item] += count
                self.births.append({'wal': filename, 'tick': tick, 'drop_uuid': birth['uuid'],
                                    'items': {item: count}, **witness})
        for key, entity in old.items():
            if not entity.get('item') or key in explicit_ages:
                continue
            position = chunk(entity['pos'])
            previous = self.debt.get(position)
            following = debt_images.get(position, previous)
            payment = 0
            if previous and following:
                delta = previous['remaining'] - following['remaining']
                # A nonzero remaining tail proves a bounded spend, not debt
                # cancellation. Ambiguous final/cancelled steps get no credit.
                if delta == 200 and following['remaining'] > 0:
                    payment = delta
            live = int(running and bool(following and following['active']))
            image = images.setdefault(key, dict(entity))
            image['age'] += payment + live
            if key in removed and image['age'] >= 6000 and image['age'] != 0xFFFFFFFF:
                item = self.names[entity['item']['item_id']]
                count = entity['item']['count']
                self.consumed[item] += count
                self.expiry.append({'wal': filename, 'tick': tick, 'uuid': entity['uuid'],
                                    'items': {item: count}, 'observed_age': entity['age'],
                                    'final_age': image['age'], 'catchup_payment': payment,
                                    'live_tick': live, 'chunk_debt_before': previous,
                                    'chunk_debt_after': following})
        old.update(images)
        for key in removed:
            old.pop(key, None)
        self.debt.update(debt_images)
