"""All91 matrix mob subjects: received motion/combat, attack and finite loot.

The matrix's family classifications are insufficient to establish every
species' combat trigger. Trigger-dependent species therefore require explicit
causal evidence, not a generic summon receipt. Missing observations block.
"""
from collections import Counter
import math
from pathlib import Path
from ..matrix_api import Adapter, Check, MissingCapability
from ._mob_contracts import CONTRACTS
from ._mob_loot import bounds
from ._mob_support import arena, entity, exists, field, inventory, require_verb, ticks

UUID='12345678-1234-1234-1234-123456789d01'
# Peaceful is species-specific, not simply MobCategory::Monster.
PEACEFUL_REMOVAL=set('blaze bogged breeze cave_spider creeper drowned elder_guardian endermite evoker ghast giant guardian husk illusioner magma_cube parched phantom piglin_brute pillager ravager silverfish skeleton slime spider stray vex vindicator warden witch wither wither_skeleton zoglin zombie zombie_villager'.split())
PROJECTILES={'Arrow':{'minecraft:arrow'},'Crossbow':{'minecraft:arrow'},
 'SmallFireball':{'minecraft:small_fireball'},'LargeFireball':{'minecraft:fireball'},
 'ShulkerBullet':{'minecraft:shulker_bullet'},'Snowball':{'minecraft:snowball'},
 'EvokerFangs':{'minecraft:evoker_fangs','minecraft:vex'},'WindCharge':{'minecraft:breeze_wind_charge'},
 'ThrownPotion':{'minecraft:potion','minecraft:splash_potion'},'WitherSkull':{'minecraft:wither_skull'}}
DEFENDERS={'iron_golem','snow_golem'}
VICTIM='12345678-1234-1234-1234-123456789d02'


def require_gameplay_version(vanilla):
    import json
    version=json.loads((Path(vanilla)/'gameplay_version.json').read_text())
    if version['id']!='26.1.2':
        raise AssertionError('mob oracle requires exact26.1.2 vanilla loot data')


def observed_health(ctx, uuid):
    row=entity(ctx,uuid)
    if row is None or row.get('health') is None:
        raise MissingCapability('snapshot:entities.health')
    return row['health']


def setup(ctx):
    name=ctx.row['subject'].split('.',1)[1]
    move,combat=CONTRACTS[name]
    # Do not create a synthetic snapshot when the driver only observes players.
    field(ctx.clients[0],'entities')
    arena(ctx)
    ctx.server.command('gamerule doMobSpawning false')
    ctx.server.command('gamerule doMobLoot true')
    ctx.server.command('time set midnight')
    ctx.server.command('weather clear')
    if move in {'AquaticWander','AmphibiousWander'}:
        ctx.server.command('fill 2 200 -8 15 206 8 water')
    if name in {'piglin','piglin_brute','hoglin'} and ctx.factors['dimension']!='the_nether':
        # Replacement UUIDs/conversion are not observable as movement of the
        # original. Their distinct oracle requires the entity conversion event.
        field(ctx.clients[0],'entity_conversions')
    nbt=f'UUID:"{UUID}",PersistenceRequired:1b'
    if name in {'slime','magma_cube'}:
        nbt+=',Size:0'
    if name=='sheep':
        nbt+=',Color:0,Sheared:0b'
    ctx.server.command(f'summon minecraft:{name} 6.5 201 0.5 {{{nbt}}}')
    ctx.state['name']=name
    ctx.state['movement']=move
    ctx.state['combat']=combat
    ctx.state['peaceful']=ctx.factors['difficulty']=='peaceful' and name in PEACEFUL_REMOVAL
    ctx.state['samples']=[]
    ctx.state['seen_types']=[]
    ctx.state['checks']=[]
    if ctx.state['peaceful']:
        ctx.wait_until(lambda:not exists(ctx,UUID),description='peaceful removes summoned hostile')
        return
    ctx.wait_until(lambda:entity(ctx,UUID) is not None,description='received exact summoned mob UUID')
    for observer in ctx.clients[1:]:
        observer.wait_until(lambda:any(row.get('uuid')==UUID for row in field(observer,'entities')),
                            description='observer receives tested mob before action')
    ctx.state['health_before']=observed_health(ctx,UUID)
    ctx.state['actor_health']=field(ctx.clients[0],'health')
    ctx.state['minimum_health']=ctx.state['actor_health']
    ctx.state['inventory_before']=inventory(ctx.clients[0])
    vanilla=getattr(ctx.server,'vanilla_data',None)
    if vanilla is None:
        raise MissingCapability('vanilla_data_path')
    require_gameplay_version(vanilla)
    ctx.state['vanilla']=str(vanilla)
    if name in DEFENDERS:
        victim_type='blaze' if name=='snow_golem' else 'zombie'
        ctx.server.command(f'summon {victim_type} 10 201 0 {{UUID:"{VICTIM}",NoAI:1b}}')
        ctx.wait_until(lambda:entity(ctx,VICTIM) is not None,description='received hostile prey')
        ctx.state['victim_health']=observed_health(ctx,VICTIM)
        ctx.state['victim_minimum']=ctx.state['victim_health']


def sample(ctx):
    rows=field(ctx.clients[0],'entities')
    for row in rows:
        if 'type' not in row:
            raise MissingCapability('snapshot:entities.type')
        if row['type'] not in ctx.state['seen_types']:
            ctx.state['seen_types'].append(row['type'])
        if row.get('uuid')==UUID:
            position=row.get('position')
            if position is None:
                raise MissingCapability('snapshot:entities.position')
            ctx.state['samples'].append(list(position))
    health=field(ctx.clients[0],'health')
    ctx.state['minimum_health']=min(ctx.state['minimum_health'],health)
    if ctx.state['name'] in DEFENDERS:
        health=observed_health(ctx,VICTIM) if exists(ctx,VICTIM) else 0
        ctx.state['victim_minimum']=min(ctx.state['victim_minimum'],health)


def act(ctx):
    if ctx.state['peaceful']:
        ctx.boundary()
        ticks(ctx,40)
        return
    actor=ctx.clients[0]
    if ctx.state['name']=='creaking':
        actor.look_at(-8,200,0)
    if ctx.state['name']=='warden' and ctx.factors['game_mode'] not in {'creative','spectator'}:
        target=entity(ctx,UUID)
        actor.teleport_to(6.5,201,-1.5)
        actor.look_at(6.5,202,0.5)
        actor.attack(target['id'])
        ctx.wait_until(lambda:observed_health(ctx,UUID)<ctx.state['health_before'],
                       description='warden anger caused by actual player hit')
        actor.teleport_to(0.5,200,0.5)
        ctx.server.command('fill -1 199 -1 1 203 1 glass hollow')
    ticks(ctx,600,lambda:sample(ctx))
    before_attack=entity(ctx,UUID)
    if before_attack is not None:
        # Standard range, not an out-of-reach attack that could falsely prove
        # invulnerability. An aquatic target is approached in its own habitat.
        x,y,z=before_attack['position']
        actor.teleport_to(x,y,z-2)
        actor.look_at(x,y+0.5,z)
        ctx.state['health_before']=observed_health(ctx,UUID)
        actor.attack(before_attack['id'])
        protected_attack=ctx.factors['game_mode']=='spectator' or (
            ctx.state['name']=='armor_stand' and ctx.factors['game_mode']=='adventure')
        if ctx.state['name']=='armor_stand' and ctx.factors['game_mode']=='survival':
            actor.attack(before_attack['id'])
        if protected_attack:
            ticks(ctx,20)
            after=observed_health(ctx,UUID)
            ctx.state['checks'].append({'name':'spectator_attack_ignored','passed':after==ctx.state['health_before'],'detail':after})
        else:
            ctx.wait_until(lambda:not exists(ctx,UUID) or observed_health(ctx,UUID)<ctx.state['health_before'],
                           description='attack changes authoritative health or removes mob')
            ctx.state['player_kill']=True
            if ctx.state['name']=='armor_stand' and ctx.factors['game_mode']=='survival':
                ctx.state['stand_drop']=True
    else:
        ctx.state['checks'].append({'name':'disappearance_requires_combat_event',
            'passed':ctx.state['combat']=='CreeperFuse' and ctx.state['minimum_health']<ctx.state['actor_health'],
            'detail':{'combat':ctx.state['combat'],'minimum_player_health':ctx.state['minimum_health']}})
    ctx.boundary()
    if field(actor,'health')<=0:
        require_verb(actor,'respawn')()
        ctx.wait_until(lambda:field(actor,'health')>0,description='received respawn before loot pickup')
    if ctx.state['name'] in DEFENDERS:
        if exists(ctx,VICTIM):
            ctx.server.command('gamerule doMobLoot false')
            ctx.server.command('kill '+VICTIM)
            ctx.server.command('gamerule doMobLoot true')
        # Remove the prey's drops before measuring the tested mob's loot.
        # Ground items, not the tested mob, are fixture cleanup here.
        ctx.server.command('kill @e[type=item]')
        ctx.state['inventory_before']=inventory(actor)
    if exists(ctx,UUID):
        ctx.state['death_position']=entity(ctx,UUID)['position']
        ctx.server.command('kill '+UUID)
        ctx.wait_until(lambda:not exists(ctx,UUID),description='authoritative mob death',timeout_seconds=60)
    else:
        ctx.state['death_position']=ctx.state['samples'][-1] if ctx.state['samples'] else [6.5,201,0.5]
    ticks(ctx,40)
    ctx.state['drops']=drop_total(ctx)
    if ctx.factors['game_mode']!='spectator':
        actor.teleport_to(*ctx.state['death_position'])
        ticks(ctx,40)
    ctx.state['after_pickup']=drop_total(ctx)


def drop_total(ctx):
    total=Counter(inventory(ctx.clients[0]))
    total.subtract(ctx.state['inventory_before'])
    for row in field(ctx.clients[0],'entities'):
        if row.get('type')=='minecraft:item':
            origin=row.get('spawn_position')
            if origin is None:
                raise MissingCapability('snapshot:entities.spawn_position')
            # The loaded world can produce unrelated items far below the Y200
            # arena. Attribute drops spatially, never by expected item IDs.
            if math.dist(origin,ctx.state['death_position'])>8:
                continue
            item=row.get('item')
            if item is None:
                raise MissingCapability('snapshot:entities.item')
            total[item['item_id']]+=item['count']
    return {k:v for k,v in total.items() if v}


def oracle(ctx):
    if ctx.state['peaceful']:
        return [Check('peaceful_removes_species',not exists(ctx,UUID),ctx.state['name'])]
    name=ctx.state['name']; movement=ctx.state['movement']; combat=ctx.state['combat']
    samples=ctx.state['samples']
    checks=[Check(**check) for check in ctx.state['checks']]
    if not samples:
        raise MissingCapability('snapshot:mob_trajectory')
    displacement=max(math.dist(samples[0],p) for p in samples)
    checks.append(Check('finite_entity_trajectory',all(math.isfinite(v) for p in samples for v in p),{'samples':len(samples)}))
    if movement=='Immobile':
        # Spawn settles under gravity first. Only horizontal displacement is AI.
        distance=max(math.hypot(p[0]-samples[0][0],p[2]-samples[0][2]) for p in samples)
        checks.append(Check('no_spontaneous_horizontal_motion',distance<=0.05,distance))
    else:
        checks.append(Check('active_movement_observed',displacement>0.2,{'policy':movement,'distance':displacement}))
    if ctx.factors['game_mode'] in {'creative','spectator'}:
        checks.append(Check('protected_mode_not_damaged',ctx.state['minimum_health']==ctx.state['actor_health']))
    elif name in DEFENDERS:
        checks.append(Check('defender_attacks_hostile_prey',
                            ctx.state['victim_minimum']<ctx.state['victim_health'],
                            {'before':ctx.state['victim_health'],'minimum':ctx.state['victim_minimum']}))
    elif combat in PROJECTILES:
        checks.append(Check('species_projectile_observed',bool(PROJECTILES[combat]&set(ctx.state['seen_types'])),
                            {'expected':sorted(PROJECTILES[combat]),'seen':sorted(ctx.state['seen_types'])}))
    elif combat not in {'None'}:
        checks.append(Check('hostile_damage_observed',ctx.state['minimum_health']<ctx.state['actor_health'],
                            {'policy':combat,'before':ctx.state['actor_health'],'minimum':ctx.state['minimum_health']}))
    else:
        checks.append(Check('unprovoked_passive_no_damage',ctx.state['minimum_health']==ctx.state['actor_health']))
    limits=bounds(ctx.state['vanilla'],'entities/'+name,ctx.state.get('player_kill',False))
    if ctx.state.get('stand_drop'):
        # Vanilla ArmorStand.brokenByPlayer drops one stand separately from its
        # empty loot table. Creative and /kill call kill without that drop.
        limits['minecraft:armor_stand']=(1,1)
    actual=ctx.state['drops']
    valid=all(low<=actual.get(item,0)<=high for item,(low,high) in limits.items()) and not(set(actual)-set(limits))
    checks.append(Check('vanilla_loot_count_bounds',valid,{'expected':limits,'actual':actual,'source':'data/minecraft/loot_table/entities/'+name+'.json'}))
    checks.append(Check('pickup_conserves_ground_plus_inventory',ctx.state['after_pickup']==actual,
                        {'before':actual,'after':ctx.state['after_pickup']}))
    for observer in ctx.clients[1:]:
        def removed():
            return not any(row.get('uuid')==UUID for row in field(observer,'entities'))
        observer.wait_until(removed,description='observer receives tested mob removal')
        checks.append(Check('observer_receives_dead_uuid_removal:'+observer.name,removed()))
    checks.append(Check('dead_uuid_not_duplicated',not exists(ctx,UUID)))
    return checks


ADAPTERS={'mobs.'+name:Adapter(setup,act,oracle) for name in CONTRACTS}
