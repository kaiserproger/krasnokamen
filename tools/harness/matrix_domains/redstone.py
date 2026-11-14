"""Redstone fixtures and authoritative assertions; no command-ack oracles."""
from ..matrix_api import MissingCapability
from ._world import (adapter, block, check, command, condition, entity_field,
                     exists, give, observe, ticks, use, verb)


def wire_power_levels(ctx):
    command(ctx, 'fill 8 100 8 22 100 8 redstone_wire')
    command(ctx, 'setblock 7 100 8 redstone_block')
    command(ctx, 'setblock 23 100 8 redstone_lamp')
    ticks(ctx, 10)
    for i in range(15):
        block(ctx, (8+i, 100, 8), f'redstone_wire[power={15-i}]', source='Redstone_Dust#Power')
    block(ctx, (23, 100, 8), 'redstone_lamp[lit=true]', source='Redstone_Lamp')
    command(ctx, 'setblock 7 100 8 air')
    ticks(ctx, 10)
    block(ctx, (23, 100, 8), 'redstone_lamp[lit=false]', source='Redstone_Lamp')


def torch_inversion_burnout(ctx):
    command(ctx, 'setblock 8 100 8 stone')
    command(ctx, 'setblock 8 101 8 redstone_torch')
    command(ctx, 'setblock 7 100 8 redstone_block')
    ticks(ctx, 3)
    block(ctx, (8, 101, 8), 'redstone_torch[lit=false]', source='Redstone_Torch')
    command(ctx, 'setblock 7 100 8 air')
    ticks(ctx, 3)
    block(ctx, (8, 101, 8), 'redstone_torch[lit=true]', source='Redstone_Torch')
    # A real server-side clock avoids network latency stretching the 60-tick window.
    command(ctx, 'setblock 8 102 8 stone')
    command(ctx, 'setblock 7 102 8 redstone_wire')
    command(ctx, 'setblock 7 101 8 stone')
    command(ctx, 'setblock 7 100 8 redstone_wire')
    ticks(ctx, 60)
    events = observe(ctx, 'block_updates')
    off = [e['tick'] for e in events if e.get('position') == [8, 101, 8] and e.get('state') == 'minecraft:redstone_torch[lit=false]']
    check(ctx, 'torch-burnout-clock', len(off) >= 8 and max(off[-8:])-min(off[-8:]) <= 60, off, '8 off transitions in <=60 game ticks', 'Redstone_Torch#Burnout')


def repeater_delay_lock(ctx):
    for delay in range(1, 5):
        command(ctx, f'setblock 8 100 8 repeater[facing=east,delay={delay}]')
        command(ctx, 'setblock 7 100 8 redstone_block')
        ticks(ctx, 12)
        block(ctx, (8, 100, 8), 'repeater[powered=true]', source='Redstone_Repeater')
        command(ctx, 'setblock 8 100 7 repeater[facing=south,powered=true]')
        command(ctx, 'setblock 8 100 6 redstone_block')
        ticks(ctx, 4)
        command(ctx, 'setblock 7 100 8 air')
        ticks(ctx, 12)
        block(ctx, (8, 100, 8), 'repeater[powered=true,locked=true]', source='Redstone_Repeater#Signal_locking')
        command(ctx, 'setblock 8 100 6 air')
        command(ctx, 'setblock 8 100 7 air')
        ticks(ctx, 12)
        block(ctx, (8, 100, 8), 'repeater[powered=false]', source='Redstone_Repeater')
    # Settled outputs cannot establish 2/4/6/8 authoritative game-tick delays.
    raise MissingCapability('server_tick_block_updates')


def comparator_modes(ctx):
    command(ctx, 'setblock 8 100 8 comparator[facing=east]')
    command(ctx, 'setblock 9 100 8 redstone_wire')
    for bites in range(7):
        command(ctx, f'setblock 7 100 8 cake[bites={bites}]')
        ticks(ctx, 4)
        block(ctx, (9, 100, 8), f'redstone_wire[power={14-2*bites}]', source='Redstone_Comparator#Measure_block_state')
    command(ctx, 'setblock 7 100 8 redstone_block')
    command(ctx, 'setblock 8 100 7 redstone_block')
    command(ctx, 'setblock 8 100 8 comparator[facing=east,mode=subtract]')
    ticks(ctx, 4)
    block(ctx, (9, 100, 8), 'redstone_wire[power=0]', source='Redstone_Comparator#Subtract_signal_strength')
    command(ctx, 'setblock 8 100 7 air')
    command(ctx, 'setblock 7 100 8 chest')
    ticks(ctx, 4)
    block(ctx, (9, 100, 8), 'redstone_wire[power=0]', source='Redstone_Comparator#Containers')
    # Separate fixtures outside the live plugin's spawn-market region.
    ctx.clients[0].teleport_to(5.5, 100, 16.5)
    verb(ctx, 'close_container')
    command(ctx, 'setblock 8 100 16 comparator[facing=east]')
    command(ctx, 'setblock 9 100 16 redstone_wire')
    for count, strength in ((0, 0), (64, 1), (864, 8), (1728, 15)):
        command(ctx, 'setblock 7 100 16 air')
        command(ctx, 'setblock 7 100 16 chest')
        if count:
            _load_container(ctx, (7, 100, 16), 27, 'stone', count)
        ticks(ctx, 4)
        block(ctx, (9, 100, 16), f'redstone_wire[power={strength}]',
              source='Redstone_Comparator#Fullness_of_containers')
    # A frame attached to the back of the measured full cube yields rotation+1.
    command(ctx, 'setblock 7 100 16 stone')
    for rotation in range(8):
        command(ctx, 'kill @e[type=minecraft:item_frame]')
        command(ctx, 'summon item_frame 6 100 16 '
                '{Facing:4b,Item:{id:"minecraft:stone",count:1},'
                f'ItemRotation:{rotation}b' + '}')
        ticks(ctx, 4)
        block(ctx, (9, 100, 16), f'redstone_wire[power={rotation+1}]',
              source='Redstone_Comparator#Item_frames')
    command(ctx, 'kill @e[type=minecraft:item_frame]')
    command(ctx, 'setblock 7 100 16 redstone_block')
    command(ctx, 'setblock 8 100 14 redstone_block')
    command(ctx, 'setblock 8 100 15 redstone_wire')
    command(ctx, 'setblock 8 100 16 comparator[facing=east,mode=subtract]')
    ticks(ctx, 4)
    block(ctx, (9, 100, 16), 'redstone_wire[power=0]',
          source='Redstone_Comparator#Subtract_signal_strength')

def _load_container(ctx, position, slots, item, count):
    """Fill through the real menu; player inventory slots start at nine."""
    give(ctx, item, count)
    stacks = [s for s in observe(ctx, 'inventory')
              if s['item_id'] == 'minecraft:' + item]
    ctx.clients[0].open_container(position)
    for stack in stacks:
        verb(ctx, 'click_slot', stack['slot'] + slots - 9, kind='quick_move')
    contents = [s for s in observe(ctx, 'menu.slots') if s['slot'] < slots]
    actual = sum(s['count'] for s in contents if s['item_id'] == 'minecraft:' + item)
    check(ctx, f'container-loaded:{position}:{item}', actual == count,
          actual, count, 'Chest#Usage')
    verb(ctx, 'close_container')


def _container_count(ctx, position, slots, item):
    ctx.clients[0].open_container(position)
    count = sum(s['count'] for s in observe(ctx, 'menu.slots')
                if s['slot'] < slots and s['item_id'] == 'minecraft:' + item)
    verb(ctx, 'close_container')
    return count



def pressure_plates_buttons(ctx):
    ctx.clients[0].teleport_to(2.5, 100, 13.5)
    for material, reset in [('stone_button', 20), ('oak_button', 30)]:
        command(ctx, f'setblock 3 100 16 {material}[face=floor]')
        use(ctx, (3, 100, 16))
        block(ctx, (3, 100, 16), material + '[powered=' + ('false' if ctx.factors['game_mode'] == 'spectator' else 'true') + ']', source='Button')
        ticks(ctx, reset + 2)
        block(ctx, (3, 100, 16), material + '[powered=false]', source='Button')
        command(ctx, 'summon arrow 3.5 102 16.5 {Motion:[0.0d,-1.0d,0.0d]}')
        ticks(ctx, 10)
        block(ctx, (3, 100, 16), material + '[powered=' + str(material == 'oak_button').lower() + ']',
              source='Button', name=material + ':arrow')
        command(ctx, 'kill @e[type=minecraft:arrow]')
    for plate, item_triggers in [('stone_pressure_plate', False), ('oak_pressure_plate', True), ('light_weighted_pressure_plate', True), ('heavy_weighted_pressure_plate', True)]:
        command(ctx, f'setblock 8 100 16 {plate}')
        command(ctx, 'summon item 8.5 100.1 16.5 {Item:{id:"minecraft:stone",count:1},PickupDelay:32767}')
        ticks(ctx, 20)
        property_ = 'power=1' if 'weighted' in plate else 'powered=' + str(item_triggers).lower()
        block(ctx, (8, 100, 16), plate + '[' + property_ + ']', source='Pressure_Plate')
        command(ctx, 'kill @e[type=minecraft:item]')
        command(ctx, 'summon armor_stand 8.5 100 16.5 {NoGravity:1b}')
        ticks(ctx, 20)
        block(ctx, (8, 100, 16), plate + '[' + ('power=1' if 'weighted' in plate else 'powered=true') + ']',
              source='Pressure_Plate', name=plate + ':living-entity')
        command(ctx, 'kill @e[type=minecraft:armor_stand]')
        command(ctx, 'kill @e[type=minecraft:item]')
    for plate, counts, divisor in (
            ('light_weighted_pressure_plate', (1, 14, 15, 16), 1),
            ('heavy_weighted_pressure_plate', (1, 10, 11, 150), 10)):
        command(ctx, f'setblock 8 100 16 {plate}')
        previous = 0
        for count in counts:
            for _ in range(count - previous):
                command(ctx, 'summon armor_stand 8.5 100 16.5 {NoGravity:1b}')
            ticks(ctx, 20)
            power = min(15, (count + divisor - 1) // divisor)
            block(ctx, (8, 100, 16), f'{plate}[power={power}]',
                  source='Weighted_Pressure_Plate', name=f'{plate}:{count}-entities')
            previous = count
        command(ctx, 'kill @e[type=minecraft:armor_stand]')
        command(ctx, 'kill @e[type=minecraft:item]')
        ticks(ctx, 20)
        block(ctx, (8, 100, 16), f'{plate}[power=0]', source='Weighted_Pressure_Plate')


def observer_pulse(ctx):
    command(ctx, 'setblock 8 100 8 observer[facing=west]')
    command(ctx, 'setblock 9 100 8 copper_bulb')
    ticks(ctx, 6)
    command(ctx, 'setblock 7 100 8 stone')
    ticks(ctx, 6)
    block(ctx, (9, 100, 8), 'copper_bulb[lit=true]', source='Observer')
    block(ctx, (8, 100, 8), 'observer[powered=false]', source='Observer')
    updates = observe(ctx, 'block_updates')
    edges = [e for e in updates if e.get('position') == [8, 100, 8]]
    on = next((e['tick'] for e in edges if 'powered=true' in e.get('state', '')), None)
    off = next((e['tick'] for e in edges if on is not None and e['tick'] > on and 'powered=false' in e.get('state', '')), None)
    check(ctx, 'observer-two-game-tick-pulse', on is not None and off == on + 2, edges, 2, 'Observer#Behavior')


def piston_push_pull(ctx):
    for count in (12, 13):
        command(ctx, 'fill 7 100 7 24 101 9 air')
        command(ctx, 'setblock 8 100 8 piston[facing=east]')
        command(ctx, f'fill 9 100 8 {8+count} 100 8 stone')
        command(ctx, 'setblock 7 100 8 redstone_block')
        ticks(ctx, 6)
        block(ctx, (8, 100, 8), 'piston[extended=' + str(count == 12).lower() + ']', source='Piston#Pushing_blocks')
        block(ctx, (9+count, 100, 8), 'stone' if count == 12 else 'air', source='Piston#Pushing_blocks')
    for obstacle in ('obsidian', 'chest'):
        command(ctx, 'fill 7 100 7 24 101 9 air')
        command(ctx, 'setblock 8 100 8 piston[facing=east]')
        command(ctx, f'setblock 9 100 8 {obstacle}')
        command(ctx, 'setblock 7 100 8 redstone_block')
        ticks(ctx, 6)
        block(ctx, (8, 100, 8), 'piston[extended=false]', source='Piston#Limitations')
    command(ctx, 'fill 7 100 7 24 101 9 air')
    command(ctx, 'setblock 8 100 8 sticky_piston[facing=east]')
    command(ctx, 'setblock 9 100 8 stone')
    command(ctx, 'setblock 7 100 8 redstone_block')
    ticks(ctx, 6)
    command(ctx, 'setblock 7 100 8 air')
    ticks(ctx, 6)
    block(ctx, (9, 100, 8), 'stone', source='Piston')
    block(ctx, (10, 100, 8), 'air', source='Piston')


def slime_honey_flying_machine(ctx):
    for sticky, other in [('slime_block', 'honey_block'), ('honey_block', 'slime_block')]:
        command(ctx, 'fill 7 100 7 13 102 10 air')
        command(ctx, 'setblock 8 101 8 sticky_piston[facing=east]')
        command(ctx, f'setblock 9 101 8 {sticky}')
        command(ctx, 'setblock 9 102 8 stone')
        command(ctx, f'setblock 9 101 9 {other}')
        command(ctx, 'setblock 7 101 8 redstone_block')
        ticks(ctx, 6)
        block(ctx, (10, 101, 8), sticky, source='Honey_Block#Pistons')
        block(ctx, (10, 102, 8), 'stone', source='Honey_Block#Pistons')
        block(ctx, (9, 101, 9), other, source='Honey_Block#Pistons')


def quasi_connectivity(ctx):
    command(ctx, 'setblock 8 100 8 piston[facing=east]')
    command(ctx, 'setblock 7 101 8 redstone_block')
    command(ctx, 'setblock 8 100 7 stone')
    ticks(ctx, 6)
    block(ctx, (8, 100, 8), 'piston[extended=true]', source='Quasi-connectivity')


def dispenser_behaviours(ctx):
    # Single occupied slot makes selection deterministic; each case is isolated.
    ctx.clients[0].teleport_to(2.5, 100, 13.5)
    verb(ctx, 'close_container')
    cases = (
        ('arrow', 'air', 'arrow'), ('fire_charge', 'air', 'small_fireball'),
        ('tnt', 'air', 'tnt'), ('minecart', 'rail', 'minecart'),
        ('oak_boat', 'water', 'oak_boat'), ('water_bucket', 'air', None),
        ('lava_bucket', 'air', None), ('powder_snow_bucket', 'air', None),
        ('bucket', 'water[level=0]', None), ('bone_meal', 'wheat[age=0]', None),
        ('shulker_box', 'air', None), ('shears', 'beehive[honey_level=5]', None),
        ('iron_chestplate', 'air', None),
    )
    for item, front, entity_type in cases:
        command(ctx, 'kill @e[type=!minecraft:player]')
        command(ctx, 'fill 2 100 15 6 102 17 air')
        command(ctx, 'setblock 3 100 16 dispenser[facing=east]')
        command(ctx, 'setblock 4 99 16 farmland[moisture=7]' if item == 'bone_meal'
                else 'setblock 4 99 16 stone')
        command(ctx, f'setblock 4 100 16 {front}')
        if item == 'iron_chestplate':
            command(ctx, 'summon armor_stand 4.5 100 16.5')
        _load_container(ctx, (3, 100, 16), 9, item, 1)
        command(ctx, 'setblock 2 100 16 redstone_block')
        ticks(ctx, 5)
        if entity_type:
            exists(ctx, f'@e[type=minecraft:{entity_type}]', source='Dispenser#Behavior')
        elif item.endswith('_bucket'):
            fluid = item.removesuffix('_bucket')
            expected = 'air' if fluid == 'water' and ctx.factors['dimension'] == 'the_nether' else fluid
            block(ctx, (4, 100, 16), expected, source='Dispenser#Behavior')
        elif item == 'bucket':
            block(ctx, (4, 100, 16), 'air', source='Dispenser#Behavior')
        elif item == 'bone_meal':
            grew = any(condition(ctx, f'block 4 100 16 wheat[age={age}]')
                       for age in range(2, 6))
            check(ctx, 'dispenser-bonemeal-grows-wheat', grew, grew, True,
                  'Bone_Meal#Crops')
        elif item == 'shulker_box':
            block(ctx, (4, 100, 16), 'shulker_box', source='Dispenser#Behavior')
        elif item == 'shears':
            block(ctx, (4, 100, 16), 'beehive[honey_level=0]', source='Dispenser#Behavior')
            exists(ctx, '@e[type=minecraft:item]', source='Dispenser#Behavior')
        # Returned bucket/tool stays in the dispenser; consumed items do not.
        residue = 'bucket' if item.endswith('_bucket') else 'water_bucket' if item == 'bucket' else item
        expected_count = 1 if item.endswith('_bucket') or item in ('bucket', 'shears') else 0
        actual = _container_count(ctx, (3, 100, 16), 9, residue)
        check(ctx, 'dispenser-custody:' + item, actual == expected_count,
              actual, expected_count, 'Dispenser#Behavior')
        if entity_type:
            spawned = [e for e in observe(ctx, 'entities')
                       if e.get('type') == 'minecraft:' + entity_type]
            check(ctx, 'dispenser-spawn-count:' + item, len(spawned) == 1,
                  len(spawned), 1, 'Dispenser#Behavior')
        if item == 'shears':
            drops = [e for e in observe(ctx, 'entities') if e.get('type') == 'minecraft:item']
            if any(e.get('item') is None for e in drops):
                raise MissingCapability('snapshot:entities.item')
            honeycomb = sum(e['item']['count'] for e in drops
                           if e['item']['item_id'] == 'minecraft:honeycomb')
            check(ctx, 'dispenser-three-honeycombs', honeycomb == 3,
                  honeycomb, 3, 'Shears#Bees')
            ctx.clients[0].open_container((3, 100, 16))
            tools = [s for s in observe(ctx, 'menu.slots')
                     if s['slot'] < 9 and s['item_id'] == 'minecraft:shears']
            damage = tools[0].get('components', {}).get('minecraft:damage') if len(tools) == 1 else None
            check(ctx, 'dispenser-shears-wear', damage == 1, damage, 1, 'Shears#Bees')
            verb(ctx, 'close_container')
        if item == 'iron_chestplate':
            actual = entity_field(ctx, 'armor_stand', 'equipment.chest.id')
            check(ctx, 'dispenser-equips-armor', actual == 'minecraft:iron_chestplate',
                  actual, 'minecraft:iron_chestplate', 'Dispenser#Behavior')
        command(ctx, 'setblock 2 100 16 air')


def hopper_chains(ctx):
    ctx.clients[0].teleport_to(2.5, 100, 13.5)
    verb(ctx, 'close_container')
    command(ctx, 'setblock 3 100 16 hopper[facing=east]')
    command(ctx, 'setblock 4 100 16 furnace')
    command(ctx, 'setblock 3 101 16 chest')
    command(ctx, 'setblock 2 100 16 redstone_block')
    _load_container(ctx, (3, 101, 16), 27, 'coal', 2)
    ticks(ctx, 24)
    block(ctx, (3, 100, 16), 'hopper[enabled=false]', source='Hopper#Redstone_component')
    for position, slots, expected in (((3, 101, 16), 27, 2),
                                     ((3, 100, 16), 5, 0),
                                     ((4, 100, 16), 3, 0)):
        count = _container_count(ctx, position, slots, 'coal')
        check(ctx, 'locked-hopper-custody:' + str(position), count == expected,
              count, expected, 'Hopper#Redstone_component')
    command(ctx, 'setblock 2 100 16 air')
    ticks(ctx, 24)
    ctx.clients[0].open_container((4, 100, 16))
    slots = observe(ctx, 'menu.slots')
    fuel = next((stack for stack in slots if stack['slot'] == 1), None)
    check(ctx, 'hopper-side-input-is-furnace-fuel', fuel is not None and fuel.get('item_id') == 'minecraft:coal' and fuel.get('count') == 2, fuel, {'item_id': 'minecraft:coal', 'count': 2}, 'Hopper#Furnaces')
    verb(ctx, 'close_container')
    for position, slots in (((3, 101, 16), 27), ((3, 100, 16), 5)):
        count = _container_count(ctx, position, slots, 'coal')
        check(ctx, 'unlocked-hopper-source-drained:' + str(position), count == 0,
              count, 0, 'Hopper#Collecting_items')
    command(ctx, 'setblock 4 101 16 hopper[facing=down]')
    command(ctx, 'setblock 4 102 16 chest')
    command(ctx, 'setblock 4 99 16 hopper[facing=down]')
    command(ctx, 'setblock 4 98 16 chest')
    _load_container(ctx, (4, 102, 16), 27, 'raw_iron', 2)
    ticks(ctx, 440)
    for position, slots, item, expected in (
            ((4, 102, 16), 27, 'raw_iron', 0),
            ((4, 101, 16), 5, 'raw_iron', 0),
            ((4, 98, 16), 27, 'iron_ingot', 2)):
        count = _container_count(ctx, position, slots, item)
        check(ctx, 'hopper-top-input-bottom-output:' + str(position), count == expected,
              count, expected, 'Hopper#Furnaces')
    # Console round trips do not measure the native eight-tick transfer period.
    raise MissingCapability('server_tick_container_transfers')


def rails_powered_detector_activator(ctx):
    for rail in ('powered_rail', 'detector_rail', 'activator_rail'):
        command(ctx, f'fill 6 100 8 16 100 8 {rail}[shape=east_west]')
        command(ctx, 'setblock 5 100 8 redstone_block')
        if rail == 'detector_rail':
            command(ctx, 'summon minecart 8.5 100.1 8.5')
        ticks(ctx, 10)
        block(ctx, (8, 100, 8), rail + '[powered=true]', source=rail.title())
        if rail == 'activator_rail':
            command(ctx, 'summon tnt_minecart 8.5 100.1 8.5')
            ticks(ctx, 2)
            fuse = entity_field(ctx, 'tnt_minecart', 'TNTFuse')
            check(ctx, 'activator-primes-tnt-cart', 0 <= fuse <= 80, fuse, '0..80', 'Activator_Rail')


def daylight_target_bulb_lamp(ctx):
    command(ctx, 'weather clear')
    command(ctx, 'time set noon')
    command(ctx, 'setblock 8 100 8 daylight_detector')
    ticks(ctx, 40)
    if ctx.factors['dimension'] == 'overworld':
        block(ctx, (8, 100, 8), 'daylight_detector[power=15]', source='Daylight_Detector')
        command(ctx, 'time set midnight')
        ticks(ctx, 40)
        block(ctx, (8, 100, 8), 'daylight_detector[power=0]', source='Daylight_Detector')
    else:
        block(ctx, (8, 100, 8), 'daylight_detector[power=0]', source='Daylight_Detector#Default_mode')
    for component in ('copper_bulb', 'redstone_lamp'):
        command(ctx, f'setblock 8 100 10 {component}')
        command(ctx, 'setblock 7 100 10 redstone_block')
        ticks(ctx, 4)
        block(ctx, (8, 100, 10), component + '[lit=true]', source=component.title())
        command(ctx, 'setblock 7 100 10 air')
        ticks(ctx, 6)
        block(ctx, (8, 100, 10), component + '[lit=' + str(component == 'copper_bulb').lower() + ']', source=component.title())
        command(ctx, 'setblock 7 100 10 redstone_block')
        ticks(ctx, 4)
        block(ctx, (8, 100, 10), component + '[lit=' + str(component == 'redstone_lamp').lower() + ']', source=component.title())
        command(ctx, 'setblock 7 100 10 air')
        ticks(ctx, 6)
        block(ctx, (8, 100, 10), component + '[lit=false]', source=component.title())


def sculk_sensor_signal(ctx):
    # Powering a note block emits frequency ten without fabricated game events.
    # Keep the actor out of detection range so footsteps cannot win arbitration.
    command(ctx, 'gamerule doMobSpawning false')
    command(ctx, 'kill @e[type=!minecraft:player]')
    for calibrated, distance, selected in (
            (False, 2, 0), (False, 6, 0),
            (True, 2, 10), (True, 2, 8), (True, 12, 10)):
        command(ctx, 'fill 6 100 0 41 102 21 air')
        command(ctx, 'fill 6 99 0 41 99 21 stone')
        kind = 'calibrated_sculk_sensor' if calibrated else 'sculk_sensor'
        command(ctx, f'setblock 24 100 16 {kind}' + ('[facing=north]' if calibrated else ''))
        command(ctx, 'fill 25 99 16 39 99 16 copper_bulb')
        command(ctx, 'fill 25 100 16 39 100 16 redstone_wire')
        # A bulb beside the dust is not the downward latch: it is a control.
        command(ctx, 'setblock 27 100 17 copper_bulb')
        # Diode FACING points toward its input, not its output.
        command(ctx, 'setblock 23 100 16 comparator[facing=east]')
        command(ctx, 'fill 8 99 16 22 99 16 copper_bulb')
        command(ctx, 'fill 8 100 16 22 100 16 redstone_wire')
        if calibrated:
            command(ctx, f'setblock 24 100 19 cake[bites={(14-selected)//2}]')
            command(ctx, 'setblock 24 100 18 comparator[facing=south]')
            command(ctx, 'setblock 24 100 17 redstone_wire')
        command(ctx, f'setblock 24 100 {16-distance} note_block')
        ticks(ctx, 45)
        block(ctx, (25, 99, 16), 'copper_bulb[lit=false]', source='Sculk_Sensor')
        block(ctx, (22, 99, 16), 'copper_bulb[lit=false]', source='Sculk_Sensor')
        accepts = not calibrated or selected == 10
        strength = max(1, 15 - (15 * distance // (16 if calibrated else 8))) if accepts else 0
        label = f'calibrated={calibrated}:distance={distance}:selected={selected}'
        call = getattr(ctx.clients[0], 'call', None)
        command(ctx, f'setblock 23 100 {16-distance} redstone_block')
        # Native matrix evidence must observe the sensor transition itself,
        # not infer it from a latched downstream bulb after the pulse.
        if accepts and callable(call):
            received = []
            for phase in ('active', 'cooldown', 'inactive'):
                properties = {'sculk_sensor_phase': phase}
                if phase == 'active':
                    properties['power'] = str(strength)
                reply = call('minecraft_wait_for_block_state', {
                    'x': 24, 'y': 100, 'z': 16,
                    'block_id': 'minecraft:' + kind,
                    'properties': properties,
                    'timeout_seconds': 30.0,
                })
                received.append(reply['block']['properties']['sculk_sensor_phase'])
            expected_phases = ['active', 'cooldown', 'inactive']
            check(ctx, label + ':received-phases', received == expected_phases,
                  received, expected_phases, source='Sculk_Sensor')
        # Bulbs retain the pulse after the sensor becomes inactive. Console
        # round trips must not decide whether a ten-tick pulse was observed.
        ticks(ctx, distance + 40)
        block(ctx, (24, 100, 16-distance), 'note_block[powered=true]',
              source='Note_Block#Redstone_component', name=label + ':stimulus')
        block(ctx, (27, 100, 17), 'copper_bulb[lit=false]',
              source='Sculk_Sensor#Redstone_emission', name=label + ':perpendicular-control')
        def received_bulb(position, lit, name):
            if not callable(call):
                return
            reply = call('minecraft_wait_for_block_state', {
                **dict(zip(('x', 'y', 'z'), position)),
                'block_id': 'minecraft:copper_bulb',
                'properties': {'lit': str(lit).lower()},
                'timeout_seconds': 30.0,
            })
            observed = reply['block']
            expected = {'block_id': 'minecraft:copper_bulb', 'lit': str(lit).lower()}
            actual = {'block_id': observed['block_id'], 'lit': observed['properties']['lit']}
            check(ctx, name + ':received', actual == expected, actual, expected,
                  source='Sculk_Sensor#Redstone_emission')
        received_bulb((27, 100, 17), False, label + ':perpendicular-control')
        for index in range(1, 16):
            block(ctx, (24+index, 99, 16),
                  f'copper_bulb[lit={str(index <= strength).lower()}]',
                  source='Sculk_Sensor#Redstone_emission',
                  name=f'{label}:strength-latch:{index}')
            received_bulb((24 + index, 99, 16), index <= strength,
                          f'{label}:strength-latch:{index}')
            block(ctx, (23-index, 99, 16),
                  f'copper_bulb[lit={str(accepts and index <= 10).lower()}]',
                  source='Sculk_Sensor#Vibration_frequencies',
                  name=f'{label}:frequency-latch:{index}')
            received_bulb((23 - index, 99, 16), accepts and index <= 10,
                          f'{label}:frequency-latch:{index}')
        block(ctx, (24, 100, 16), f'{kind}[sculk_sensor_phase=inactive]',
              source='Calibrated_Sculk_Sensor#Filtering' if calibrated else 'Sculk_Sensor')


def trapped_chest_note_door_power(ctx):
    command(ctx, 'setblock 3 100 3 trapped_chest')
    command(ctx, 'setblock 4 100 3 redstone_lamp')
    ctx.clients[0].open_container((3, 100, 3))
    ticks(ctx, 2)
    block(ctx, (4, 100, 3), 'redstone_lamp[lit=true]', source='Trapped_Chest')
    verb(ctx, 'close_container')
    command(ctx, 'setblock 8 100 8 iron_door[half=lower]')
    command(ctx, 'setblock 8 101 8 iron_door[half=upper]')
    command(ctx, 'setblock 7 100 8 redstone_block')
    ticks(ctx, 2)
    block(ctx, (8, 100, 8), 'iron_door[open=true]', source='Door')
    command(ctx, 'setblock 12 100 8 note_block')
    command(ctx, 'setblock 11 100 8 redstone_block')
    ticks(ctx, 2)
    block(ctx, (12, 100, 8), 'note_block[powered=true]', source='Note_Block')
    command(ctx, 'setblock 16 100 8 tnt')
    command(ctx, 'setblock 15 100 8 redstone_block')
    ticks(ctx, 2)
    exists(ctx, '@e[type=minecraft:tnt]', source='TNT')


def command_block(ctx):
    for kind in ('command_block', 'repeating_command_block'):
        command(ctx, f'setblock 8 100 8 {kind}[facing=east]')
        command(ctx, 'setblock 9 100 8 chain_command_block[facing=east]')
        command(ctx, 'data merge block 8 100 8 {Command:"setblock ~ ~ ~2 minecraft:gold_block"}')
        command(ctx, 'data merge block 9 100 8 {Command:"setblock ~ ~ ~2 minecraft:diamond_block",auto:1b}')
        command(ctx, 'setblock 7 100 8 redstone_block')
        ticks(ctx, 3)
        block(ctx, (8, 100, 10), 'gold_block', source='Command_Block', name=kind + ':relative-origin')
        block(ctx, (9, 100, 10), 'diamond_block', source='Command_Block', name=kind + ':chain')
        command(ctx, 'fill 8 100 10 9 100 10 air')
        ticks(ctx, 3)
        repeating = kind == 'repeating_command_block'
        block(ctx, (8, 100, 10), 'gold_block' if repeating else 'air',
              source='Command_Block', name=kind + ':held-power')
        command(ctx, 'setblock 7 100 8 air')
        command(ctx, 'fill 8 100 10 9 100 10 air')
    raise MissingCapability('command_block_output:operator-chat-and-console-cursor')


def update_order_quirks(ctx):
    # Exact first cell of quirk_parity/scenarios/tnt-duper-found-bud.json;
    # that local 26.1.2 oracle records two primed TNT from one built block.
    quasi_connectivity(ctx)
    for line in (
        'gamerule tnt_explodes false',
        'fill 0 63 0 15 63 15 stone',
        'setblock 7 65 6 slime_block',
        'setblock 8 65 6 redstone_torch',
        'setblock 5 64 6 piston[facing=east]',
        'setblock 7 63 6 redstone_block',
        'setblock 8 64 6 observer[facing=west,powered=true]',
        'setblock 7 64 6 tnt',
        'setblock 6 64 6 redstone_block',
        'setblock 4 64 6 piston[facing=east]',
        'gamerule tnt_explodes true',
    ):
        command(ctx, line)
    ticks(ctx, 2)
    command(ctx, 'setblock 4 64 5 redstone_block')
    ticks(ctx, 8)
    ctx.server.command('scoreboard players set dw_tnt dw_probe 0')
    command(ctx, 'execute as @e[type=minecraft:tnt,x=0,y=60,z=0,dx=15,dy=12,dz=15] run scoreboard players add dw_tnt dw_probe 1')
    response = ctx.server.query('scoreboard players get dw_tnt dw_probe')
    check(ctx, 'tnt-duper-two-from-one', 'dw_tnt has 2 [dw_probe]' in response,
          response, 2, 'Tutorial:Block_and_item_duplication#TNT_duplication')


ADAPTERS = {'redstone.' + fn.__name__: adapter(fn) for fn in (
    wire_power_levels, torch_inversion_burnout, repeater_delay_lock, comparator_modes,
    pressure_plates_buttons, observer_pulse, piston_push_pull, slime_honey_flying_machine,
    quasi_connectivity, dispenser_behaviours, hopper_chains,
    rails_powered_detector_activator, daylight_target_bulb_lamp, sculk_sensor_signal,
    trapped_chest_note_door_power, command_block, update_order_quirks,
)}
