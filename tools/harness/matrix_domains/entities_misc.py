"""Nonliving entities and mounts; absent observations are explicit capabilities."""
from ._world import (adapter, boundary, check, command, data, data_size, entity,
                     entity_field, exists, give, interact, inventory, observe,
                     ticks, use, verb)


def boats(ctx):
    for boat in ('oak_boat', 'oak_chest_boat'):
        for surface in ('water', 'ice'):
            command(ctx, f'fill 1 99 1 8 99 30 {surface}')
            command(ctx, f'summon {boat} 3.5 100 3.5')
            interact(ctx, boat)
            before = entity_field(ctx, boat, 'Pos')
            verb(ctx, 'vehicle_move', forward=1, strafe=0, ticks=40)
            after = entity_field(ctx, boat, 'Pos')
            moved = sum((a-b)**2 for a, b in zip(after, before)) > 1
            check(ctx, boat + ':' + surface + ':rows', moved, [before, after], 'distance >1', 'Boat#Behavior')
            verb(ctx, 'sneak', True)
            verb(ctx, 'sneak', False)
            target = entity(ctx, boat)['id']
            for _ in range(5):
                verb(ctx, 'attack', target)
            ticks(ctx, 3)
            exists(ctx, f'@e[type=minecraft:{boat}]', expected=False, source='Boat#Breaking')


def minecarts(ctx):
    command(ctx, 'fill 2 100 3 30 100 3 rail[shape=east_west]')
    command(ctx, 'summon minecart 3.5 100.1 3.5')
    interact(ctx, 'minecart')
    before = entity_field(ctx, 'minecart', 'Pos')
    verb(ctx, 'vehicle_move', forward=1, strafe=0, ticks=40)
    after = entity_field(ctx, 'minecart', 'Pos')
    check(ctx, 'ride-cart-moves', after[0] != before[0], [before, after], 'x changes on rail', 'Minecart')
    verb(ctx, 'sneak', True)
    verb(ctx, 'sneak', False)
    command(ctx, 'summon furnace_minecart 3.5 100.1 3.5')
    interact(ctx, 'furnace_minecart', 'coal')
    fuel = entity_field(ctx, 'furnace_minecart', 'Fuel')
    check(ctx, 'furnace-cart-fueled', fuel > 0, fuel, '>0', 'Minecart_with_Furnace')


def horses_mounts(ctx):
    for kind, lure in [('horse', None), ('pig', 'carrot_on_a_stick'), ('strider', 'warped_fungus_on_a_stick'), ('camel', None)]:
        command(ctx, f'summon {kind} 3.5 100 3.5' + (' {Tame:1b}' if kind == 'horse' else ''))
        interact(ctx, kind, 'saddle')
        interact(ctx, kind, lure)
        before = entity_field(ctx, kind, 'Pos')
        verb(ctx, 'vehicle_move', forward=1, strafe=0, ticks=40)
        after = entity_field(ctx, kind, 'Pos')
        check(ctx, kind + ':steering', after != before, [before, after], 'position changes', 'Horse' if kind == 'horse' else kind.title())
        if kind == 'camel':
            verb(ctx, 'jump', ticks=20)
            dash = entity_field(ctx, kind, 'Motion')
            check(ctx, 'camel-dash-velocity', any(abs(v) > .1 for v in dash), dash, 'nonzero dash motion', 'Camel')
        verb(ctx, 'sneak', True)
        verb(ctx, 'sneak', False)
        command(ctx, f'kill @e[type=minecraft:{kind}]')


def leash(ctx):
    command(ctx, 'summon cow 3 100 3')
    interact(ctx, 'cow', 'lead')
    leash_fields = data_size(ctx, '@e[type=minecraft:cow,limit=1]', 'leash')
    check(ctx, 'cow-leashed', leash_fields > 0, leash_fields, 'nonempty leash attachment', 'Lead')
    command(ctx, 'setblock 3 100 4 oak_fence')
    use(ctx, (3, 100, 4))
    exists(ctx, '@e[type=minecraft:leash_knot]', source='Lead')
    command(ctx, 'tp @e[type=minecraft:cow,limit=1] 30 100 3')
    ticks(ctx, 5)
    # 26.1.2 uses the post-1.21.6 lead range (12), not the matrix's old 10.
    exists(ctx, '@e[type=minecraft:item]', source='Lead')


def armor_stand_entity(ctx):
    command(ctx, 'summon armor_stand 3 100 3')
    interact(ctx, 'armor_stand', 'diamond_chestplate')
    equipped = data(ctx, '@e[type=minecraft:armor_stand,limit=1]', 'equipment.chest.id')
    check(ctx, 'armor-stand-equipment', equipped == 'minecraft:diamond_chestplate', equipped, 'minecraft:diamond_chestplate', 'Armor_Stand')
    target = entity(ctx, 'armor_stand')['id']
    verb(ctx, 'attack', target)
    boundary(ctx)
    ticks(ctx, 2)
    verb(ctx, 'attack', target)
    ticks(ctx, 2)
    exists(ctx, '@e[type=minecraft:armor_stand]', expected=False, source='Armor_Stand#Breaking')
    exists(ctx, '@e[type=minecraft:item]', source='Armor_Stand#Breaking')


def item_frame_entity(ctx):
    for kind in ('item_frame', 'glow_item_frame'):
        command(ctx, 'setblock 3 101 3 stone')
        use(ctx, (3, 101, 3), kind, face='north')
        interact(ctx, kind, 'diamond')
        before = entity_field(ctx, kind, 'ItemRotation')
        interact(ctx, kind, 'stick')
        after = entity_field(ctx, kind, 'ItemRotation')
        check(ctx, kind + ':rotation', after == (before+1)%8, after, (before+1)%8, 'Item_Frame')
        verb(ctx, 'break_block', (3, 101, 3))
        ticks(ctx, 3)
        exists(ctx, f'@e[type=minecraft:{kind}]', expected=False, source='Item_Frame#Breaking')
        exists(ctx, '@e[type=minecraft:item]', source='Item_Frame#Breaking')


def experience_orbs(ctx):
    # Force a real XP update; an absent initial SetExperience is not zero XP.
    command(ctx, f'experience add {ctx.clients[0].name} 1 points')
    ctx.wait_until(lambda: ctx.clients[0].snapshot().get('xp') is not None,
                   description='received fixture experience update')
    command(ctx, 'summon experience_orb 8 100 8 {Value:3s}')
    command(ctx, 'summon experience_orb 8 100 8 {Value:3s}')
    ticks(ctx, 40)
    before = observe(ctx, 'xp.total')
    command(ctx, f'tp {ctx.clients[0].name} 8 100 8')
    ticks(ctx, 10)
    after = observe(ctx, 'xp.total')
    check(ctx, 'xp-conserved-through-merge-pickup', after-before == 6, after-before, 6, 'Experience#Experience_orbs')
    give(ctx, 'diamond_pickaxe[minecraft:damage=20,minecraft:enchantments={"minecraft:mending":1}]')
    command(ctx, f'summon experience_orb 8 100 8 {{Value:3s}}')
    ticks(ctx, 10)
    tools = [stack for stack in observe(ctx, 'inventory')
             if stack['item_id'] == 'minecraft:diamond_pickaxe']
    if len(tools) != 1 or 'minecraft:damage' not in tools[0].get('components', {}):
        from ..matrix_api import MissingCapability
        raise MissingCapability('snapshot:inventory.components.minecraft:damage')
    damage = tools[0]['components']['minecraft:damage']
    check(ctx, 'mending-two-durability-per-xp', damage == 14, damage, 14, 'Mending')


def display_interaction_markers(ctx):
    for kind, fields in [('block_display', '{block_state:{Name:"minecraft:stone"}}'), ('item_display', '{item:{id:"minecraft:diamond",count:1}}'), ('text_display', '{text:"WorldProbe"}'), ('interaction', '{width:1f,height:2f,response:1b}'), ('marker', '{}')]:
        ctx.clients[0].command(f'summon {kind} 3 100 3 {fields}')
        boundary(ctx)
        exists(ctx, f'@e[type=minecraft:{kind}]', source='Display' if 'display' in kind else kind.title())
    interact(ctx, 'interaction')
    timestamp = entity_field(ctx, 'interaction', 'interaction.timestamp')
    check(ctx, 'interaction-records-use', timestamp >= 0, timestamp, 'nonnegative game time', 'Interaction')


def area_effect_cloud(ctx):
    give(ctx, 'lingering_potion[minecraft:potion_contents={potion:"minecraft:poison"}]')
    ctx.clients[0].look_at(3, 99, 3)
    verb(ctx, 'use_item', ticks=1)
    ticks(ctx, 30)
    exists(ctx, '@e[type=minecraft:area_effect_cloud]', source='Lingering_Potion')
    command(ctx, 'summon cow 3 100 3')
    ticks(ctx, 40)
    effect = data(ctx, '@e[type=minecraft:cow,limit=1]', 'active_effects[{id:"minecraft:poison"}].id')
    check(ctx, 'cloud-applies-poison', effect == 'minecraft:poison', effect, 'minecraft:poison', 'Area_Effect_Cloud')


def paintings(ctx):
    for width, height in [(1, 1), (2, 1), (1, 2), (2, 2), (3, 3), (3, 4), (4, 2), (4, 3), (4, 4)]:
        command(ctx, 'fill 2 100 4 7 105 4 air')
        command(ctx, f'fill 3 100 4 {2+width} {99+height} 4 stone')
        use(ctx, (3, 100, 4), 'painting', face='north')
        exists(ctx, '@e[type=minecraft:painting]', source='Painting#Placement')
        variant = entity_field(ctx, 'painting', 'variant')
        check(ctx, f'painting-{width}x{height}-has-variant', isinstance(variant, str) and variant.startswith('minecraft:'), variant, 'registry painting variant', 'Painting')
        verb(ctx, 'break_block', (3, 100, 4))
        ticks(ctx, 40)
        exists(ctx, '@e[type=minecraft:painting]', expected=False, source='Painting#Breaking')


def happy_ghast_harness(ctx):
    command(ctx, 'summon happy_ghast 3 100 3')
    interact(ctx, 'happy_ghast', 'white_harness')
    for client in ctx.clients[:4]:
        method = getattr(client, 'interact_entity', None)
        if not callable(method):
            from ..matrix_api import MissingCapability
            raise MissingCapability('client:interact_entity')
        method(entity(ctx, 'happy_ghast')['id'])
    boundary(ctx)
    riders = data_size(ctx, '@e[type=minecraft:happy_ghast,limit=1]', 'Passengers')
    check(ctx, 'happy-ghast-rider-capacity', riders == min(4, len(ctx.clients)), riders, min(4, len(ctx.clients)), 'Happy_Ghast#Riding')
    before = entity_field(ctx, 'happy_ghast', 'Pos')
    verb(ctx, 'vehicle_move', forward=1, strafe=0, ticks=40)
    after = entity_field(ctx, 'happy_ghast', 'Pos')
    check(ctx, 'harnessed-flight', after != before, [before, after], 'position changes', 'Happy_Ghast#Riding')


ADAPTERS = {'entities_misc.' + fn.__name__: adapter(fn) for fn in (
    boats, minecarts, horses_mounts, leash, armor_stand_entity, item_frame_entity,
    experience_orbs, display_interaction_markers, area_effect_cloud, paintings,
    happy_ghast_harness,
)}
