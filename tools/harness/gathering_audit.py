"""Fail-closed D54 accounting from immutable native WAL and physical preimages.

A carry increase alone is not production. Credit requires a profession, a
matching work cursor (miners/loggers), an actual block transition and the
configured yield in that actor's inventory. Deposits are separate transfers,
never production. Unknown causes stay visible in the all-item balance.
"""
from collections import Counter
import copy
import hashlib
import json
from pathlib import Path
import struct
import zlib

from .gathering_natural import NaturalObserver
DIMENSION = 'minecraft:overworld'
MINING_DROPS = {
    'grass_block': 'dirt', 'podzol': 'dirt', 'mycelium': 'dirt',
    'stone': 'cobblestone', 'deepslate': 'cobbled_deepslate', 'clay': 'clay_ball',
    **{prefix + ore + '_ore': drop for prefix in ('', 'deepslate_')
       for ore, drop in (('coal', 'coal'), ('iron', 'raw_iron'),
                         ('copper', 'raw_copper'), ('gold', 'raw_gold'),
                         ('diamond', 'diamond'), ('emerald', 'emerald'),
                         ('redstone', 'redstone'), ('lapis', 'lapis_lazuli'))},
}
CROPS = {'wheat': ('wheat', 1, 'wheat_seeds', 7),
         'beetroots': ('beetroot', 1, 'beetroot_seeds', 3),
         'carrots': ('carrot', 3, None, 7), 'potatoes': ('potato', 3, None, 7)}


def nonzero(counts):
    return {item: count for item, count in sorted(counts.items()) if count}


def stacks(slots, names):
    result = Counter()
    for stack in slots:
        if stack and stack['count']:
            if stack['count'] < 0:
                raise ValueError('negative custody stack')
            result[names[stack['item_id']]] += stack['count']
    return result


def entries_in(entries, dimension=DIMENSION):
    for entry in entries:
        kind, value = next(iter(entry.items()))
        if kind == 'Dimension':
            yield from entries_in(value['entries'], value['dimension'])
        elif dimension == DIMENSION:
            yield kind, value


def batches(archive, start, end):
    """Validate CRC, filename, and the complete contiguous publication interval."""
    frontier = start
    for path in sorted(Path(archive).glob('batch-*.bin')):
        data = path.read_bytes()
        if len(data) < 32:
            raise ValueError(f'truncated WAL header: {path}')
        magic, version, lo, hi, size, crc = struct.unpack('<IIQQII', data[:32])
        if (magic != 0x534F4C4A or version != 1 or size != len(data) - 32
                or zlib.crc32(data[32:]) != crc
                or path.name != f'batch-{lo:020d}-{hi:020d}.bin'):
            raise ValueError(f'invalid WAL frame: {path}')
        if hi <= start or lo > end:
            continue
        if lo != frontier + 1 or hi > end:
            raise ValueError(f'noncontiguous WAL: {frontier} -> {lo}..{hi}')
        frontier = hi
        yield path.name, lo, hi, list(entries_in([json.loads(line) for line in data[32:].splitlines()]))
    if frontier != end:
        raise ValueError(f'incomplete WAL: {frontier}, expected {end}')


def near(position, block, radius):
    return sum((position[i] - (block[i] + (0.5 if i != 1 else 0))) ** 2
               for i in range(3)) <= radius * radius


def mining_cursor(before, after):
    first, last = before['mining_progress'], after['mining_progress']
    identity = ('building', 'revision', 'bounds', 'lane', 'level', 'pass')
    return (first['bounds'] is not None and first['building']
            and all(first[k] == last[k] for k in identity)
            and ((first['cell'] == last['cell'] and last['layer'] == first['layer'] + 1)
                 or (last['cell'] == first['cell'] + 1 and last['layer'] == 0)))


def gathering_event(old, new, block_changes, block_before, states, names, yield_factor):
    """Return one uniquely witnessed harvest, or no credit (never infer a residual)."""
    before, after = old.get('villager'), new.get('villager')
    if not before or not after or old['uuid'] != new['uuid']:
        return None
    delta = stacks(after['inventory'], names)
    delta.subtract(stacks(before['inventory'], names))
    gain = {item: count for item, count in delta.items() if count > 0}
    if not gain:
        return None
    role = after['data']['profession']
    candidates = []
    for block in block_changes:
        position = tuple(block['pos'])
        prior = block_before.get(position)
        if prior is None:
            continue
        previous_name, previous_props = states[prior]
        next_name, next_props = states[block['state']]
        path = previous_name.removeprefix('minecraft:')
        expected = None
        cursor = None
        if role == 'Miner' and mining_cursor(before, after):
            bounds = after['mining_progress']['bounds']
            if (next_name == 'minecraft:air' and previous_name != next_name
                    and all(bounds['min'][i] <= position[i] <= bounds['max'][i] for i in range(3))
                    and near(new['pos'], position, 6)):
                # Native mining::work gathers one base item per physical layer,
                # including clay; this is not construction's drop_count table.
                expected = {'minecraft:' + MINING_DROPS.get(path, path): yield_factor}
                cursor = {'before': before['mining_progress'], 'after': after['mining_progress']}
        elif role == 'Lumberjack':
            first, last = before['lumber_progress'], after['lumber_progress']
            target = first if first['has_target'] else last
            base = target['base']
            layer = first['layer'] if first['has_target'] else 0
            at = (base[0], base[1] + layer, base[2])
            advanced = (not last['has_target'] or
                        (last['base'] == base and last['layer'] == layer + 1))
            if (target['has_target'] and target['building'] and advanced
                    and position == at and path.endswith('_log')
                    and next_name == 'minecraft:air' and near(new['pos'], base, 6)):
                expected = {previous_name: yield_factor}
                cursor = {'before': first, 'after': last}
        elif role == 'Farmer' and path in CROPS:
            item, count, seed, age = CROPS[path]
            if (previous_props.get('age') == str(age) and next_name == previous_name
                    and next_props.get('age') == '0' and near(new['pos'], position, 4)):
                expected = {'minecraft:' + item: count * yield_factor}
                if seed:
                    expected['minecraft:' + seed] = yield_factor
                cursor = {'physical_target': list(position), 'age_before': age, 'age_after': 0}
        if expected is not None and gain == expected:
            candidates.append({'actor': new['uuid'], 'entity_id': new['id'],
                               'profession': role, 'tick': new['tick'],
                               'position': list(position), 'block_before': prior,
                               'block_after': block['state'], 'items': expected,
                               'cursor': cursor})
    return candidates[0] if len(candidates) == 1 else None


def record_projects(projects, entries, filename):
    for kind, value in entries:
        if kind == 'Settlement':
            for project in value['image']['value']['projects']:
                projects[project['structure']] = {'settlement': value['image']['id'],
                                                 'wal': filename, **project}


def observe(before, archive, end, names, states, block_preimages, yield_factor, entity_types):
    if not isinstance(yield_factor, int) or yield_factor < 1:
        raise ValueError('observer requires the recorded integral gathering yield')
    entities = copy.deepcopy(before['dimensions'][DIMENSION]['entities'])
    banks = {tuple(bank['position']): stacks(bank['slots'], names) for bank in before['banks']}
    blocks = dict(block_preimages)
    gathered = Counter()
    events, deposits, rejected = [], [], []
    projects = {}
    for key, value in before['globals'].items():
        if key.startswith('settlement/') and value['dimension'] == DIMENSION:
            record_projects(projects, [('Settlement', {'image': {
                'id': value['value']['id'], 'value': value['value']}})],
                f'checkpoint/{before["checkpoint_tick"]}')
    natural = NaturalObserver(before, names, states, entity_types)
    for filename, lo, hi, entries in batches(archive, before['checkpoint_tick'], end):
        # D54's immutable pin publishes one simulation tick per batch. Refuse
        # coalesced frames: postimages cannot prove intra-frame causal ordering.
        if lo != hi:
            raise ValueError(f'causal observer requires single-tick frames: {filename}')
        record_projects(projects, entries, filename)
        natural.consume(entries, filename, hi, blocks)
        changes = [value for kind, value in entries if kind == 'Block']
        bank_gains = {}
        for kind, value in entries:
            if kind == 'BlockEntity':
                point = tuple(value['pos'])
                current = stacks(value['slots'], names)
                diff = current.copy()
                diff.subtract(banks.get(point, Counter()))
                bank_gains[point] = diff
        claimed = set()
        for kind, value in entries:
            key = str(value.get('id'))
            if kind == 'Entity':
                old = entities.get(key, {})
                event = gathering_event(old, value, changes, blocks, states, names, yield_factor)
                if event:
                    point = tuple(event['position'])
                    if point in claimed:
                        raise ValueError(f'two gatherers claim the same block: {filename} {point}')
                    claimed.add(point)
                    event['wal'] = filename
                    events.append(event)
                    gathered.update(event['items'])
                first, last = old.get('villager'), value.get('villager')
                if first and last:
                    diff = stacks(last['inventory'], names)
                    diff.subtract(stacks(first['inventory'], names))
                    losses = {item: -count for item, count in diff.items() if count < 0}
                    if losses:
                        destinations = [point for point, gains in bank_gains.items()
                                        if near(value['pos'], point, 2.5)
                                        and all(gains[item] >= count for item, count in losses.items())]
                        if len(destinations) == 1:
                            point = destinations[0]
                            deposits.append({'wal': filename, 'tick': value['tick'], 'actor': value['uuid'],
                                             'storage': list(point), 'items': losses})
                            bank_gains[point].subtract(losses)
                    gains = {item: count for item, count in diff.items() if count > 0}
                    if gains and event is None and last['data']['profession'] in ('Miner', 'Lumberjack', 'Farmer'):
                        rejected.append({'wal': filename, 'actor': value['uuid'], 'items': gains,
                                         'reason': 'no unique block/cursor/yield witness; no production credit'})
                entities[key] = value
            elif kind == 'ItemMotion' and key in entities:
                entities[key].update(value)
            elif kind == 'EntityState' and key in entities:
                entities[key]['state'] = value['state']
            elif kind == 'EntityAppearance' and key in entities:
                entities[key].update({k: v for k, v in value.items() if k != 'id'})
            elif kind == 'RemoveEntity':
                entities.pop(key, None)
            elif kind == 'BlockEntity':
                banks[tuple(value['pos'])] = stacks(value['slots'], names)
        for block in changes:
            blocks[tuple(block['pos'])] = block['state']
    return {'gathered': nonzero(gathered), 'other_consumed': nonzero(natural.consumed),
            'other_produced': nonzero(natural.produced), 'natural_production': natural.births,
            'gathering': events, 'deposits': deposits, 'ground_expiry': natural.expiry,
            'rejected_actor_gains': rejected, 'frontier': end, 'project_structures': projects}


def full_balance(construction, observer):
    before, after = construction['before'], construction['after']
    causal = construction['journaled']
    items = set(before['total']) | set(after['total'])
    for counts in [*causal.values(), observer['gathered'], observer['other_consumed'], observer['other_produced']]:
        items.update(counts)
    rows, residual = {}, {}
    for item in sorted(items):
        row = {'before': before['total'].get(item, 0), 'after': after['total'].get(item, 0),
               'gathering': observer['gathered'].get(item, 0),
               'other_produced': observer['other_produced'].get(item, 0),
               'construction_cut': causal['cut'].get(item, 0),
               'crafting': causal['crafting'].get(item, 0),
               'construction_consumed': causal['consumed'].get(item, 0),
               'other_consumed': observer['other_consumed'].get(item, 0)}
        row['delta'] = row['after'] - row['before']
        row['unexplained'] = (row['delta'] - row['gathering'] - row['construction_cut']
                              - row['crafting'] - row['other_produced'] + row['construction_consumed'] + row['other_consumed'])
        rows[item] = row
        if row['unexplained']:
            residual[item] = row['unexplained']
    return {'status': 'FAIL' if residual or construction['construction_kinds_on_ground'] else 'PASS',
            'before': before, 'after': after, 'per_item': rows, 'unexplained_delta': residual,
            'structure_records': construction['structure_records'],
            'construction_kinds_on_ground': construction['construction_kinds_on_ground'],
            'journaled': causal, 'causal_observer': observer}


def warehouse_proof(building, projects, records, after_audit, after_custody, names):
    """Resolve the stable structure ID, not its transformed preparation origin."""
    matches = [p for p in projects.values()
               if p['blueprint'] == 'storage' and p['origin'] == building['min']]
    if len(matches) != 1:
        return {'status': 'FAIL', 'building': building, 'error': 'no unique journaled storage project identity'}
    project = matches[0]
    key = 'structure/' + project['structure']
    record = records[key]
    dirt = next(item for item, name in names.items() if name == 'minecraft:dirt')
    cut = record['cut_credits'].get(str(dirt), 0)
    consumed = sum(row['quantity'] for row in record['snapshot']['consumed']
                   if row['resource'] == 'minecraft:dirt')
    builder = project['builder'][0]
    carry = after_custody['carries'].get(builder, {})
    receiving = [bank for bank in after_audit['banks']
                 if all(building['min'][i] <= bank['position'][i] <= building['max'][i] for i in range(3))
                 and any(slot and slot['count'] for slot in bank['slots'])]
    checks = {'committed': record['snapshot']['state'] == 'committed',
              'receiving_stock': bool(receiving), 'real_soil_cut': cut > 0,
              'soil_surplus_returned': cut <= consumed or not carry.get('minecraft:dirt', 0)}
    return {'status': 'PASS' if all(checks.values()) else 'FAIL', 'checks': checks,
            'project': project, 'structure_key': key, 'structure_origin': record['snapshot']['origin'],
            'building': building, 'soil_cut': cut, 'soil_consumed': consumed,
            'builder_carry': carry, 'receiving_banks': receiving}


def audit_growth(before, after, names, out, binary, config, source_world, probe_world, reconcile):
    """Gate finalizer; probe the immutable baseline, never the mutated final world."""
    archive = out / 'journal-witness'
    end = after['dimensions'][DIMENSION]['tick']
    positions = {tuple(value['pos']) for _, _, _, entries in batches(archive, before['checkpoint_tick'], end)
                 for kind, value in entries if kind == 'Block'}
    preimages = {}
    if positions:
        physical = probe_world(binary, config, source_world, positions, out / 'gathering-preimages.json')
        if not physical['source_unchanged'] or physical['checkpoint_tick'] != before['checkpoint_tick']:
            raise ValueError('gathering preimages do not match the immutable accounting baseline')
        preimages = {(r['x'], r['y'], r['z']): r['state'] for r in physical['blocks']}
    registry = Path(__file__).resolve().parents[2] / 'data/vanilla/reports/blocks.json'
    states = {state['id']: (name, state.get('properties', {}))
              for name, block in json.loads(registry.read_text()).items() for state in block['states']}
    import tomllib
    acceleration = tomllib.loads(config.read_text())['test_acceleration']['gathering_yield']
    if int(acceleration) != acceleration:
        raise ValueError('D54 observer requires an integral configured yield')
    registries = json.loads((registry.parent / 'registries.json').read_text())
    entity_types = {entry['protocol_id']: name
                    for name, entry in registries['minecraft:entity_type']['entries'].items()}
    observer = observe(before, archive, end, names, states, preimages, int(acceleration), entity_types)
    observer['block_registry_sha256'] = hashlib.sha256(registry.read_bytes()).hexdigest()
    observer['configured_gathering_yield'] = acceleration
    (out / 'gathering-receipts.json').write_text(json.dumps(observer, indent=2) + '\n')
    result = full_balance(reconcile(before, after, names, required_items={'minecraft:dirt'}), observer)
    run = json.loads((out / 'run.json').read_text())
    stores = []
    if run['snapshots']:
        initial_stores = {tuple(b['min']) for town in run['snapshots'][0]['settlements']
                          for b in town['buildings'] if b['kind'] == 'storage'}
        stores = [b for town in run['snapshots'][-1]['settlements'] for b in town['buildings']
                  if b['kind'] == 'storage' and tuple(b['min']) not in initial_stores]
    result['warehouse_proofs'] = [warehouse_proof(b, projects=observer['project_structures'],
        records=result['structure_records'], after_audit=after, after_custody=result['after'], names=names)
        for b in stores]
    return result
