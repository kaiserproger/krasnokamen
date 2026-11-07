"""Deterministic spawning checks; source: minecraft.wiki/w/Mob_spawning and Breeding.

Fixtures retain the driver's context. Difficulty changes belong only to the
peaceful-removal action, whose subject explicitly requests that transition.
"""
from ..matrix_api import Adapter, Check, MissingCapability
from ._mob_support import arena, count, entity, exists, field, inventory, number, require_verb, select_item, string, tick, ticks

PARENT_A = '12345678-1234-1234-1234-123456789a01'
PARENT_B = '12345678-1234-1234-1234-123456789a02'
PERSISTENT = '12345678-1234-1234-1234-123456789a03'
ORDINARY = '12345678-1234-1234-1234-123456789a04'


def despawn_setup(ctx):
    arena(ctx)
    ctx.server.command('gamerule doMobSpawning false')
    # Peaceful cannot establish a hostile despawn-distance fixture. Use an
    # ambient bat instead: it is nonpersistent and has 128-block despawn range.
    ctx.server.command(f'summon bat 0 201 0 {{UUID:"{ORDINARY}",NoAI:1b}}')
    ctx.server.command(f'summon bat 0 201 0 {{UUID:"{PERSISTENT}",NoAI:1b,PersistenceRequired:1b,CustomName:"Keep"}}')
    if not exists(ctx, ORDINARY) or not exists(ctx, PERSISTENT):
        raise AssertionError('both despawn specimens must exist before distance changes')
    for observer in ctx.clients[1:]:
        observer.wait_until(lambda:any(row.get('uuid')==ORDINARY for row in field(observer,'entities')),
                            description='observer receives ordinary despawn control')
    # Forceload preserves simulation while every player leaves the area.
    ctx.server.command('forceload add -16 -16 16 16')

def despawn_act(ctx):
    for client in ctx.clients:
        client.teleport_to(129.5, 201, 0.5)
    ctx.wait_until(lambda: not exists(ctx, ORDINARY), description='nonpersistent bat despawn beyond128')
    ctx.boundary()
    ticks(ctx, 40)
    ctx.state['far'] = (exists(ctx, ORDINARY), exists(ctx, PERSISTENT))
    # Random-despawn branch: many specimens, no names/persistence, 33..128
    # blocks from all players, after600 idle ticks. One specimen alone is not
    # a deterministic oracle for a 1/800 trial.
    for _ in range(32):
        ctx.server.command('summon bat 64 201 0 {NoAI:1b}')
    before = count(ctx, 'bat')
    ticks(ctx, 1600)
    ctx.state['idle_before'] = before
    ctx.state['idle_after'] = count(ctx, 'bat')


def despawn_oracle(ctx):
    checks=[Check('instant_despawn_beyond128', ctx.state['far'][0] is False, ctx.state['far']),
            Check('persistent_named_retained', ctx.state['far'][1] is True, ctx.state['far']),
            Check('idle_random_despawn_population', ctx.state['idle_after'] < ctx.state['idle_before'],
                  {'before':ctx.state['idle_before'],'after':ctx.state['idle_after'],
                   'reference':'https://minecraft.wiki/w/Mob_spawning#Despawning',
                   'sampling':'32 independent specimens;1600 ticks; stochastic evidence, not exact RNG parity'})]
    for observer in ctx.clients[1:]:
        def removed():
            return not any(row.get('uuid')==ORDINARY for row in field(observer,'entities'))
        observer.wait_until(removed,description='observer no longer receives despawned control')
        checks.append(Check('observer_receives_despawn:'+observer.name,removed()))
    return checks


def peaceful_setup(ctx):
    arena(ctx)
    ctx.server.command('gamerule doMobSpawning false')
    for kind in ('zombie', 'skeleton', 'cow'):
        ctx.server.command(f'summon {kind} 4 200 0')
    if not exists(ctx, '@e[type=cow]'):
        raise AssertionError('passive control did not spawn')
    if ctx.factors['difficulty'] != 'peaceful':
        for kind in ('zombie', 'skeleton'):
            if not exists(ctx, f'@e[type={kind}]'):
                raise AssertionError('hostile removal fixture missing: '+kind)


def peaceful_act(ctx):
    # This is an explicitly operator-driven server transition, not a setup
    # override of the row's difficulty. The initial difficulty is preserved.
    ctx.server.command('difficulty peaceful')
    ctx.wait_until(lambda: not exists(ctx, '@e[type=zombie]') and not exists(ctx, '@e[type=skeleton]'),
                   description='peaceful removes existing hostile controls')
    ctx.boundary()
    ctx.server.command('gamerule doMobSpawning true')
    ticks(ctx, 600)


def peaceful_oracle(ctx):
    checks=[Check('zombies_absent', not exists(ctx, '@e[type=zombie]')),
            Check('skeletons_absent', not exists(ctx, '@e[type=skeleton]')),
            Check('passive_control_survives', exists(ctx, '@e[type=cow]'))]
    for observer in ctx.clients[1:]:
        def received():
            types={row.get('type') for row in field(observer,'entities')}
            return not types.intersection({'minecraft:zombie','minecraft:skeleton'}) and 'minecraft:cow' in types
        observer.wait_until(received,description='observer receives peaceful removal and surviving passive control')
        checks.append(Check('observer_receives_peaceful_transition:'+observer.name,received()))
    return checks


def breed_setup(ctx):
    arena(ctx)
    ctx.server.command('gamerule doMobSpawning false')
    ctx.server.command('fill -2 199 -3 8 204 3 glass hollow')
    for uuid, x in ((PARENT_A,3),(PARENT_B,4)):
        ctx.server.command(f'summon cow {x} 200 0 {{UUID:"{uuid}",Age:0,PersistenceRequired:1b}}')
    ctx.server.command(f'clear {ctx.clients[0].name}')
    ctx.server.command(f'give {ctx.clients[0].name} wheat 4')
    ctx.wait_until(lambda: any(s['item_id']=='minecraft:wheat' and s['count']==4
                   for s in ctx.clients[0].snapshot().get('inventory', [])), description='received four wheat')
    require_verb(ctx.clients[0], 'interact_entity')
    for uuid in (PARENT_A,PARENT_B):
        if number(ctx, uuid, 'Age') != 0:
            raise AssertionError('breeding parent is not adult')


def feed(ctx, uuid):
    row = entity(ctx, uuid)
    if row is None:
        raise AssertionError('parent disappeared: '+uuid)
    ctx.clients[0].teleport_to(row['position'][0], row['position'][1], row['position'][2] - 1.5)
    ctx.clients[0].look_at(*row['position'])
    require_verb(ctx.clients[0], 'interact_entity')(row['id'])


def breed_act(ctx):
    select_item(ctx, 'minecraft:wheat')
    feed(ctx, PARENT_A)
    if ctx.factors['game_mode']=='spectator':
        ticks(ctx, 20)
        ctx.boundary()
        feed(ctx, PARENT_B)
        ticks(ctx, 100)
        ctx.state['spectator'] = True
        return
    ctx.wait_until(lambda: number(ctx,PARENT_A,'InLove') > 0,
                   description='first parent received love state')
    ctx.boundary()
    feed(ctx, PARENT_B)
    ctx.wait_until(lambda: count(ctx,'cow')==3, description='two adults produce exactly one calf', timeout_seconds=60)
    rows = ctx.clients[0].snapshot().get('entities')
    if rows is None:
        raise MissingCapability('snapshot:entities')
    babies = [e for e in rows if e.get('type')=='minecraft:cow' and e.get('uuid') not in (PARENT_A,PARENT_B)]
    if len(babies)!=1:
        raise MissingCapability('snapshot:entities.baby_uuid')
    baby = babies[0]['uuid']
    ctx.state['baby'] = baby
    ctx.state['birth_age'] = number(ctx,baby,'Age')
    ctx.state['parent_cooldowns'] = [number(ctx,p,'Age') for p in (PARENT_A,PARENT_B)]
    feed(ctx,PARENT_A); feed(ctx,PARENT_B)
    ticks(ctx,100)
    ctx.state['cooldown_count'] = count(ctx,'cow')
    ticks(ctx,24000)


def breed_oracle(ctx):
    if ctx.state.get('spectator'):
        return [Check('spectator_cannot_feed', count(ctx,'cow')==2),
                Check('spectator_no_love', all(number(ctx,p,'InLove')==0 for p in (PARENT_A,PARENT_B)))]
    checks=[Check('newborn_twenty_minute_age', -24000 <= ctx.state['birth_age'] < -23800,ctx.state['birth_age']),
            Check('five_minute_parent_cooldown',all(5800 <= a <=6000 for a in ctx.state['parent_cooldowns']),ctx.state['parent_cooldowns']),
            Check('cooldown_prevents_second_birth',ctx.state['cooldown_count']==3,ctx.state['cooldown_count']),
            Check('baby_grows_after_twenty_minutes',number(ctx,ctx.state['baby'],'Age')==0),
            Check('population_conserved',count(ctx,'cow')==3)]
    for observer in ctx.clients[1:]:
        def received():
            return any(row.get('uuid')==ctx.state['baby'] for row in field(observer,'entities'))
        observer.wait_until(received,description='observer receives new calf identity')
        checks.append(Check('observer_receives_calf:'+observer.name,received()))
    return checks


def spawner_setup(ctx):
    arena(ctx)
    ctx.server.command('gamerule doMobSpawning false')
    ctx.server.command('fill -2 199 -6 10 205 6 stone hollow')
    ctx.server.command('setblock 4 200 0 spawner')
    ctx.server.command(f'clear {ctx.clients[0].name}')
    ctx.server.command(f'give {ctx.clients[0].name} zombie_spawn_egg 1')
    ctx.wait_until(lambda:inventory(ctx.clients[0]).get('minecraft:zombie_spawn_egg')==1,
                   description='received one configuring egg')
    # Vanilla's newly placed spawner is empty, not the historical pig default.
    ctx.state['empty_before']=string(ctx,'4 200 0','SpawnData.entity.id',block=True)
    ctx.state['spawner_existing_uuids']=[row['uuid'] for row in field(ctx.clients[0],'entities')
                                          if row.get('type')=='minecraft:zombie']


def spawner_act(ctx):
    actor=ctx.clients[0]
    actor.teleport_to(4,200,-2)
    select_item(ctx,'minecraft:zombie_spawn_egg')
    actor.look_at(4.5,200.5,0.5)
    actor.use_item_on((4,200,0),face='north')
    if ctx.factors['game_mode']=='spectator':
        ticks(ctx,20)
    else:
        ctx.wait_until(lambda:string(ctx,'4 200 0','SpawnData.entity.id',block=True)=='minecraft:zombie',
                       description='egg changes actual spawner entity id')
    ctx.state['configured']=string(ctx,'4 200 0','SpawnData.entity.id',block=True)
    ctx.boundary()
    if ctx.state['configured']!='minecraft:zombie' or ctx.factors['difficulty']=='peaceful':
        return
    ctx.state['last_no_spawn_tick']=tick(ctx)
    def spawned():
        observed_at=tick(ctx)
        if count(ctx,'zombie')>0:
            return True
        ctx.state['last_no_spawn_tick']=observed_at
        return False
    ctx.wait_until(spawned,description='configured spawner produces actual zombie',timeout_seconds=100)
    ctx.state['spawned']=count(ctx,'zombie')
    ctx.wait_until(lambda:any(row.get('type')=='minecraft:zombie' and
                   row.get('uuid') not in ctx.state['spawner_existing_uuids']
                   for row in field(actor,'entities')),description='received newly spawned zombie identities')
    spawned_rows=[row for row in field(actor,'entities')
                  if row.get('type')=='minecraft:zombie' and
                  row.get('uuid') not in ctx.state['spawner_existing_uuids']]
    if any('spawn_position' not in row for row in spawned_rows):
        raise MissingCapability('snapshot:entities.spawn_position')
    ctx.state['spawn_positions']=[row['spawn_position'] for row in spawned_rows]
    ctx.state['reset_delay']=number(ctx,'4 200 0','Delay',block=True)
    ctx.state['delay_query_ticks']=tick(ctx)-ctx.state['last_no_spawn_tick']
    ctx.state['spawner_observer_receipts']=[]
    for observer in ctx.clients[1:]:
        def received():
            return any(row.get('type')=='minecraft:zombie' and
                       row.get('uuid') not in ctx.state['spawner_existing_uuids']
                       for row in field(observer,'entities'))
        observer.wait_until(received,description='observer receives spawner-produced zombie')
        ctx.state['spawner_observer_receipts'].append(observer.name)
    # All clients must leave the activation sphere, not just the acting client.
    for client in ctx.clients:
        client.teleport_to(4.5,200,17.5)
    ctx.server.command('kill @e[type=zombie]')
    ctx.state['paused_delay']=number(ctx,'4 200 0','Delay',block=True)
    ticks(ctx,800)
    ctx.state['inactive_count']=count(ctx,'zombie')
    ctx.state['inactive_delay']=number(ctx,'4 200 0','Delay',block=True)


def spawner_oracle(ctx):
    expected=None if ctx.factors['game_mode']=='spectator' else 'minecraft:zombie'
    checks=[Check('new_spawner_inert',ctx.state['empty_before'] is None,ctx.state['empty_before']),
            Check('egg_configures_spawner_not_adjacent_mob',ctx.state['configured']==expected,
                  {'expected':expected,'actual':ctx.state['configured'],
                   'source':'https://minecraft.wiki/w/Mob_spawner'})]
    if expected and ctx.factors['difficulty']!='peaceful' and ctx.state['configured']==expected:
        checks.append(Check('observers_receive_spawner_birth',
                            ctx.state['spawner_observer_receipts']==[c.name for c in ctx.clients[1:]]))
        checks.extend([Check('configured_spawner_spawns',ctx.state['spawned']>0,ctx.state['spawned']),
                       Check('outside16_blocks_inactive',ctx.state['inactive_count']==0,ctx.state['inactive_count']),
                       Check('inactive_delay_paused',ctx.state['inactive_delay']==ctx.state['paused_delay'],
                             [ctx.state['paused_delay'],ctx.state['inactive_delay']]),
                       Check('spawn_inside_four_block_horizontal_range',
                             bool(ctx.state['spawn_positions']) and
                             all(abs(p[0]-4.5)<=4 and abs(p[2]-0.5)<=4 for p in ctx.state['spawn_positions']),
                             ctx.state['spawn_positions']),
                       Check('reset_delay_200_to_800_ticks',
                             0<=ctx.state['reset_delay']<=800 and
                             ctx.state['reset_delay']+ctx.state['delay_query_ticks']>=200,
                             {'delay':ctx.state['reset_delay'],'query_ticks':ctx.state['delay_query_ticks']})])
    elif ctx.factors['difficulty']=='peaceful' or expected is None:
        checks.append(Check('no_hostile_spawn_when_peaceful_or_unconfigured',count(ctx,'zombie')==0))
    return checks


ADAPTERS = {
    'spawning.despawn_rules': Adapter(despawn_setup,despawn_act,despawn_oracle),
    'spawning.peaceful_removal': Adapter(peaceful_setup,peaceful_act,peaceful_oracle),
    'spawning.breeding_and_babies': Adapter(breed_setup,breed_act,breed_oracle),
    'spawning.monster_spawner_block': Adapter(spawner_setup,spawner_act,spawner_oracle),
}
