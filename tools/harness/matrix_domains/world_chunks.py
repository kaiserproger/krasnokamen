"""Chunk adapters; 26.1.2 has forceload but NO spawnChunkRadius gamerule.

References: https://minecraft.wiki/w/Spawn_chunk, /Commands/forceload,
/Furnace. Full subject contracts are scenarios/domains/world_chunks.yaml.
"""
from __future__ import annotations

import math
import re

from tools.harness.matrix_api import Adapter, Check, MatrixContext, MissingCapability
from tools.harness.matrix_domains.session import command, data, required, unsupported


def _menu_item_slot(ctx: MatrixContext, item: str) -> int:
    menu = required(ctx.clients[0].snapshot(), 'menu')
    slots = required(menu, 'slots')
    for stack in slots:
        if stack and stack.get('item_id') == item:
            slot = required(stack, 'slot')
            if slot >= 3:
                return slot
    raise MissingCapability('observe:furnace_player_inventory_slot:' + item)


def _force_setup(ctx: MatrixContext) -> None:
    actor = ctx.clients[0]
    if ctx.factors['permission'] not in {'console', 'op_2', 'op_3', 'op_4'}:
        raise MissingCapability('observe:forceload_permission_refusal')
    data(ctx, 'entity ' + actor.name, 'Inventory')
    x, y, z = (math.floor(value) for value in required(actor.snapshot(), 'position'))
    ctx.state['origin'] = (x + 0.5, y, z + 0.5)
    ctx.state['furnace'] = (x + 2, y, z)
    command(ctx, f'setblock {x + 2} {y} {z} minecraft:furnace')
    command(ctx, f'clear {actor.name}')
    command(ctx, f'give {actor.name} minecraft:raw_iron 1')
    command(ctx, f'give {actor.name} minecraft:coal 1')
    actor.open_container(ctx.state['furnace'])
    for item in ('minecraft:raw_iron', 'minecraft:coal'):
        actor.click_slot(_menu_item_slot(ctx, item), kind='quick_move')
    # A received menu inventory change proves the gifts entered the machine.
    inventory = data(ctx, 'entity ' + actor.name, 'Inventory')
    if 'minecraft:raw_iron' in inventory or 'minecraft:coal' in inventory:
        raise RuntimeError('furnace fixture items did not leave the player inventory')
    # A full menu close verb is not necessary: leaving reach closes it natively.


def _force_act(ctx: MatrixContext) -> None:
    fx, fy, fz = ctx.state['furnace']
    if ctx.factors['permission'] == 'console':
        command(ctx, f'forceload add {fx} {fz}')
    else:
        ctx.clients[0].command(f'forceload add {fx} {fz}')
    def forced():
        output = command(ctx, f'forceload query {fx} {fz}')
        return output if re.search(r'(?i)(marked for force loading|is force loaded|is marked|commands\.forceload\.query\.success)', output) else None
    ctx.state['force_query'] = ctx.wait_until(forced, description='forceload ticket query confirms actor command')
    ctx.boundary()
    x, y, z = ctx.state['origin']
    for index, client in enumerate(ctx.clients):
        command(ctx, f'tp {client.name} {x + 4096 + index} {y} {z + 4096}')
    initial_items = data(ctx, f'block {fx} {fy} {fz}', 'Items')
    if 'minecraft:raw_iron' not in initial_items or 'minecraft:iron_ingot' in initial_items:
        raise RuntimeError('furnace completed before remote-ticking observation began')
    ctx.state['start_tick'] = required(ctx.server.dashboard_stats(), 'simulation_tick')
    # Everyone is outside the fixture radius. The server is not empty, so pause
    # policy cannot masquerade as a forceload failure.
    start = ctx.state['start_tick']
    def advanced():
        stats = ctx.server.dashboard_stats()
        return stats if required(stats, 'simulation_tick') >= start + 240 else None
    ctx.wait_until(advanced, description='240 authoritative ticks with furnace remotely forced')
    ctx.state['items'] = data(ctx, f'block {fx} {fy} {fz}', 'Items')
    ctx.state['old_rule_output'] = ctx.server.query('gamerule spawnChunkRadius')
    command(ctx, f'forceload remove {fx} {fz}')
    command(ctx, f'tp {ctx.clients[0].name} {x} {y} {z}')


def _force_oracle(ctx: MatrixContext) -> list[Check]:
    items = ctx.state['items']
    # Exactly one smeltable input was seeded; the only expected output is one
    # ingot. Unknown item SNBT formats block rather than accepting a substring.
    stacks = re.findall(r'\{[^{}]*\}', items)
    ingots = [stack for stack in stacks if 'minecraft:iron_ingot' in stack]
    count = re.search(r'\bcount\s*:\s*(\d+)', ingots[0]) if len(ingots) == 1 else None
    if ingots and count is None:
        raise MissingCapability('data_get_block:26_1_item_count')
    return [
        Check('forceload.exact_output', len(ingots) == 1 and count is not None and int(count.group(1)) == 1, items),
        Check('forceload.input_consumed', 'minecraft:raw_iron' not in items, items),
        Check('forceload.registered', bool(re.search(r'(?i)(marked for force loading|is force loaded|is marked|commands\.forceload\.query\.success)', ctx.state['force_query'])), ctx.state['force_query']),
        Check('26.1.2.removed_spawn_rule', bool(re.search(r'(?i)(unknown|incorrect|incomplete)', ctx.state['old_rule_output'])), ctx.state['old_rule_output']),
    ]


ADAPTERS = {
    'world_chunks.fresh_generation': unsupported('vanilla_26_1_2_generation_reference_and_chunk_sections'),
    'world_chunks.chunk_streaming_radius': unsupported('observe:chunk_coordinates_and_simulation_boundary'),
    'world_chunks.light_updates': unsupported('observe:server_and_client_light_arrays'),
    'world_chunks.anti_xray': unsupported('snapshot:chunk_block_state'),
    'world_chunks.distant_horizons_lod': unsupported('dh_lod_request'),
    'world_chunks.forceload_spawn_chunks': Adapter(_force_setup, _force_act, _force_oracle),
    'world_chunks.structures_loot': unsupported('data_get_block'),
    'world_chunks.pregenerate': unsupported('server_pregenerate'),
    'world_chunks.world_import_export_anvil': unsupported('server_world_import_export_and_vanilla_reference'),
    'world_chunks.worldgen_upgrade': unsupported('server_worldgen_upgrade'),
}
