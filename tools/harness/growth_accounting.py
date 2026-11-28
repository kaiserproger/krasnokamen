"""Read construction receipts and physical custody without causal exemptions."""
from collections import Counter
import json
from pathlib import Path
import re
import shutil
import subprocess

from .growth_io import dump
from .gathering_audit import nonzero as sorted_counts


def audit_world(auditor, world, plugins, data, positions, out, label, archive=None, epoch=None):
    copied = out / f'{label}-audit-copy'
    shutil.copytree(world, copied)
    points = out / f'{label}-bank-points.json'
    dump(points, sorted(map(list, positions)))
    receipt = out / f'{label}-audit.json'
    command = [str(auditor), str(copied), str(data), str(plugins), '100', str(receipt), str(points)]
    if archive is not None:
        command.extend([str(archive), str(epoch)])
    with (out / f'{label}-audit.log').open('wb') as log:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=240)
    return json.loads(receipt.read_text())


def custody(audit, names):
    # Sparse quantities: an empty actor identity is not additional custody.
    categories = {name: Counter() for name in ('bank', 'villager', 'world_drop', 'player')}
    carries = {}
    def add(category, stack):
        if stack is not None and stack['count']:
            if stack['count'] < 0:
                raise ValueError('negative physical stack')
            categories[category][names[stack['item_id']]] += stack['count']
    for bank in audit['banks']:
        for stack in bank['slots']:
            add('bank', stack)
    for replay in audit['dimensions'].values():
        for entity in replay['entities'].values():
            add('world_drop', entity['item'])
            if entity['villager'] is not None:
                counts = Counter()
                for stack in entity['villager']['inventory']:
                    add('villager', stack)
                    if stack is not None and stack['count']:
                        counts[names[stack['item_id']]] += stack['count']
                carried = sorted_counts(counts)
                if carried:
                    carries[entity['uuid']] = carried
        for player in replay['players'].values():
            for stack in [*player['inventory'], player['carried'], *player['grid']]:
                add('player', stack)
    total = Counter()
    for counts in categories.values():
        total.update(counts)
    return {'categories': {key: sorted_counts(value) for key, value in categories.items()},
            'total': sorted_counts(total), 'carries': carries}


def live_carries(replies):
    result = {}
    prefix = 'Villager has the following entity data: '
    pattern = re.compile(r'\{count: (\d+), id: "([^"]+)"\}')
    for uuid, reply in replies.items():
        if reply.count(prefix) != 1:
            raise ValueError(f'unknown carry reply for {uuid}: {reply}')
        payload = reply.partition(prefix)[2]
        entries = list(pattern.finditer(payload))
        if payload != '[' + ', '.join(entry.group() for entry in entries) + ']':
            raise ValueError(f'unsupported carry NBT for {uuid}: {payload}')
        counts = Counter()
        for entry in entries:
            counts[entry[2]] += int(entry[1])
        carried = sorted_counts(counts)
        if carried:
            result[uuid] = carried
    return result


def receipts(audit, names):
    totals = {key: Counter() for key in ('cut', 'crafting', 'consumed')}
    records = {}
    for key, record in audit['structure_postimages'].items():
        if not key.startswith('structure/'):
            continue
        snapshot = record['snapshot']
        records[key] = {'snapshot': snapshot, 'cut_credits': record.get('cut_credits', {}),
                        'crafting_delta': record.get('crafting_delta', {})}
        for item, count in record.get('cut_credits', {}).items():
            totals['cut'][names[int(item)]] += count
        for item, count in record.get('crafting_delta', {}).items():
            totals['crafting'][names[int(item)]] += count
        for material in snapshot['consumed']:
            totals['consumed'][material['resource']] += material['quantity']
    return totals, records


def reconcile(before_audit, after_audit, names, required_items=(), *, require_no_ground=True):
    before, after = custody(before_audit, names), custody(after_audit, names)
    first, previous_records = receipts(before_audit, names)
    last, records = receipts(after_audit, names)
    causal = {}
    for key in first:
        delta = last[key].copy()
        delta.subtract(first[key])
        causal[key] = delta
    touched = {item for values in causal.values() for item, count in values.items() if count}
    touched.update(required_items)
    for key, record in records.items():
        previous_record = previous_records.get(key, {})
        previous = previous_record.get('snapshot', {})
        if (record['snapshot']['watermark'] != previous.get('watermark', 0)
                or record['crafting_delta'] != previous_record.get('crafting_delta', {})):
            touched.update(names[int(item)] for item in record['crafting_delta'])
            for stage in record['snapshot']['stages']:
                touched.update(material['resource'] for material in stage['materials'])
    delta = Counter(after['total'])
    delta.subtract(before['total'])
    residual = delta.copy()
    residual.subtract(causal['cut'])
    residual.subtract(causal['crafting'])
    residual.update(causal['consumed'])
    unexplained = {item: residual[item] for item in sorted(touched) if residual[item]}
    ground = {item: {'before': before['categories']['world_drop'].get(item, 0),
                     'after': after['categories']['world_drop'].get(item, 0)}
              for item in sorted(touched)
              if before['categories']['world_drop'].get(item, 0) or after['categories']['world_drop'].get(item, 0)}
    for item, witness in after_audit.get('ground_witnesses', {}).items():
        name = names[int(item)]
        if name in touched:
            ground.setdefault(name, {})['published_journal_witness'] = witness
    unrelated = {item: {'before': before['total'].get(item, 0), 'after': after['total'].get(item, 0),
                        'residual': residual[item], 'label': 'unrelated farming/mining/food/old-ground-expiry; raw, not excused'}
                 for item in sorted(set(before['total']) | set(after['total'])) if item not in touched}
    return {'status': 'PASS' if not unexplained and (not require_no_ground or not ground) else 'FAIL',
            'require_no_ground': require_no_ground, 'before': before, 'after': after,
            'journaled': {key: sorted_counts(value) for key, value in causal.items()},
            'touched_item_kinds': sorted(touched), 'unexplained_construction_delta': unexplained,
            'construction_kinds_on_ground': ground, 'unrelated_raw': unrelated,
            'structure_records': records}
