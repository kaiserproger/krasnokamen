"""Special blocks: Java 26.1.2.

Vanilla references: https://minecraft.wiki/w/Beacon, /Conduit, /Allay, /Bell,
/Block_of_Copper, /Composter, /Monster_Spawner, /Decorated_Pot, /Chiseled_Bookshelf.
The two final block classes have pinned local javap evidence.
"""
from ._blocks import (POS, actor, adapter, block, block_is, boundary, call, check,
    command, count, data, effects, entities, entity_health, equip, observed, require_modes, scalar,
    setblock, ticks, use)


def beacon_pyramid_effects(ctx):
    require_modes(ctx,'survival','creative','adventure')
    for level in range(1,5):
        command(ctx,'fill -4 139 -5 6 143 5 stone')
        for layer in range(1,level+1):
            command(ctx,f'fill {1-layer} {144-layer} {-layer} {1+layer} {144-layer} {layer} iron_block')
        setblock(ctx,'beacon'); equip(ctx,'emerald'); actor(ctx).open_container(POS)
        ctx.wait_until(lambda:observed(ctx,'menu')['properties'].get(0,
                       observed(ctx,'menu')['properties'].get('0'))==level,
                       description='received beacon menu pyramid level')
        check(ctx,'beacon_levels',observed(ctx,'menu')['properties'].get(0,
              observed(ctx,'menu')['properties'].get('0'))==level)
        actor(ctx).click_slot(28); actor(ctx).click_slot(0)
        call(actor(ctx),'set_beacon','minecraft:speed','minecraft:regeneration' if level==4 else None)
        boundary(ctx); call(actor(ctx),'close_container'); ticks(ctx,80)
        active=effects(ctx)
        check(ctx,'beacon_primary:'+str(level),'minecraft:speed' in active,active)
        if level==4: check(ctx,'beacon_secondary','minecraft:regeneration' in active,active)
        radius=10+level*10
        command(ctx,f'tp {actor(ctx).name} {1+radius+2}.5 144 .5'); command(ctx,f'effect clear {actor(ctx).name}'); ticks(ctx,80)
        check(ctx,'beacon_outside_range','minecraft:speed' not in effects(ctx))
        command(ctx,f'tp {actor(ctx).name} .5 144 2.5')


def conduit(ctx):
    require_modes(ctx,'survival','creative','adventure')
    command(ctx,'fill -2 143 -3 4 147 3 water')
    frame=set()
    for a in range(-2,3):
        for b in range(-2,3):
            if max(abs(a),abs(b))==2:
                frame.update({(1,145+a,b),(1+a,145,b),(1+a,145+b,0)})
    for p in sorted(frame): setblock(ctx,'prismarine',p)
    setblock(ctx,'conduit[waterlogged=true]',(1,145,0)); command(ctx,f'tp {actor(ctx).name} .5 144 .5')
    boundary(ctx); ticks(ctx,80)
    check(ctx,'full_frame_42',len(frame)==42)
    check(ctx,'conduit_power','minecraft:conduit_power' in effects(ctx))
    command(ctx,'summon drowned 2.5 144 .5 {NoAI:1b}')
    ctx.wait_until(lambda:any(e.get('health') is not None for e in entities(ctx,'drowned')),
                   description='received drowned spawn and health')
    before=entity_health(ctx,'drowned'); ticks(ctx,80)
    check(ctx,'conduit_attacks_hostile',entity_health(ctx,'drowned')<before)


def jukebox_allay(ctx):
    require_modes(ctx,'survival','creative'); setblock(ctx,'jukebox'); command(ctx,'summon allay 2.5 144 .5')
    equip(ctx,'music_disc_13'); use(ctx); ticks(ctx,20)
    allay=entities(ctx,'allay')[0]; equip(ctx,'amethyst_shard',2); call(actor(ctx),'interact_entity',allay['id']); ticks(ctx,2)
    check(ctx,'allay_duplicated',len(entities(ctx,'allay'))==2)
    shards=count(ctx,'amethyst_shard')
    check(ctx,'duplication_shard_cost',shards==(2 if ctx.factors['game_mode']=='creative' else 1))
    call(actor(ctx),'interact_entity',allay['id']); ticks(ctx,2); check(ctx,'duplication_cooldown',len(entities(ctx,'allay'))==2)
    check(ctx,'cooldown_keeps_second_shard',count(ctx,'amethyst_shard')==shards)
    equip(ctx,'diamond'); call(actor(ctx),'interact_entity',allay['id'])
    setblock(ctx,'note_block',(3,144,0)); equip(ctx); use(ctx,(3,144,0))
    command(ctx,'summon item 6.5 144 .5 {Item:{id:"minecraft:diamond",count:1}}')
    def delivered():
        return any(e['type']=='minecraft:item'
                   and (e['position'][0]-3.5)**2+(e['position'][2]-.5)**2<2.25
                   and e.get('item',{}).get('item_id')=='minecraft:diamond'
                   for e in observed(ctx,'entities'))
    ctx.wait_until(delivered,description='allay delivers to note block',timeout_seconds=30)
    check(ctx,'note_block_delivery',delivered())


def bell_ring(ctx):
    require_modes(ctx,'survival','creative','adventure')
    command(ctx,'fill 3 144 3 5 147 5 stone'); command(ctx,'fill 4 144 3 4 146 4 air')
    setblock(ctx,'red_bed[part=foot,facing=south]',(4,144,3)); setblock(ctx,'red_bed[part=head,facing=south]',(4,144,4))
    command(ctx,'summon villager 3.5 144 1.5'); command(ctx,'summon pillager 2.5 144 .5 {NoAI:1b}')
    setblock(ctx,'bell[attachment=floor]'); equip(ctx); use(ctx,POS,'south'); ticks(ctx,80)
    check(ctx,'raider_glows','minecraft:glowing' in data(ctx,'entity @e[type=pillager,limit=1]','active_effects'))
    villagers=entities(ctx,'villager'); check(ctx,'villager_flees_indoors',any(3<=e['position'][0]<=6 and 3<=e['position'][2]<=6 for e in villagers),villagers)


def copper_aging(ctx):
    command(ctx,'gamerule random_tick_speed 4096')
    for initial,next_stage in [('copper_block','exposed_copper'),('exposed_copper','weathered_copper'),('weathered_copper','oxidized_copper')]:
        setblock(ctx,initial); boundary(ctx)
        ctx.wait_until(lambda:not block_is(ctx,POS,initial),description='copper oxidation '+initial,timeout_seconds=120)
        # Capture transition, not a late query which may have advanced two stages.
        history=observed(ctx,'block_transitions')
        check(ctx,'adjacent_oxidation_stage:'+initial,any(e['before']==initial and e['after']==next_stage for e in history),history)
    setblock(ctx,'waxed_copper_block'); ticks(ctx,400); block(ctx,POS,'waxed_copper_block')


def composter_hopper(ctx):
    require_modes(ctx,'survival'); setblock(ctx,'chest',(1,143,0)); setblock(ctx,'hopper[facing=down]')
    setblock(ctx,'composter',(1,145,0)); setblock(ctx,'hopper[facing=down]',(1,146,0)); equip(ctx,'cake',7)
    actor(ctx).open_container((1,146,0)); actor(ctx).click_slot(32); actor(ctx).click_slot(0); call(actor(ctx),'close_container')
    boundary(ctx); ticks(ctx,200); block(ctx,(1,145,0),'composter[level=0]')
    actor(ctx).open_container((1,143,0)); menu=observed(ctx,'menu')
    check(ctx,'one_extracted_bone_meal',sum(s['count'] for s in menu['slots'] if s['slot']<27 and s['item_id']=='minecraft:bone_meal')==1)


def monster_spawner_entity(ctx):
    require_modes(ctx,'survival','creative'); command(ctx,'fill -3 144 -3 5 148 3 air'); command(ctx,'fill -3 143 -3 5 143 3 grass_block')
    setblock(ctx,'spawner'); equip(ctx,'pig_spawn_egg'); use(ctx)
    check(ctx,'spawner_pig_type','minecraft:pig' in data(ctx,'block 1 144 0','SpawnData'))
    ctx.wait_until(lambda:len(entities(ctx,'pig'))>0,description='pig spawner produces pig',timeout_seconds=60)
    check(ctx,'pig_spawned',len(entities(ctx,'pig'))>0)


def decorated_pot_chiseled_bookshelf(ctx):
    require_modes(ctx,'survival','creative'); setblock(ctx,'decorated_pot'); equip(ctx,'diamond',3)
    use(ctx); check(ctx,'pot_single_item_storage','minecraft:diamond' in data(ctx,'block 1 144 0','item'))
    check(ctx,'pot_item_debit',count(ctx,'diamond')==(3 if ctx.factors['game_mode']=='creative' else 2))
    setblock(ctx,'chiseled_bookshelf[facing=south]'); equip(ctx,'book',6)
    for slot,(x,y) in enumerate(((.15,.75),(.5,.75),(.85,.75),(.15,.25),(.5,.25),(.85,.25))):
        use(ctx,POS,'south',(x,y,1.0)); block(ctx,POS,f'chiseled_bookshelf[slot_{slot}_occupied=true]')
        check(ctx,'bookshelf_last_slot',scalar(ctx,'block 1 144 0','last_interacted_slot')==slot)
    setblock(ctx,'comparator[facing=north]',(1,144,-1)); ticks(ctx,2)
    check(ctx,'bookshelf_comparator_six',scalar(ctx,'block 1 144 -1','OutputSignal')==6)


ADAPTERS={f'special_blocks.{fn.__name__}':adapter(fn) for fn in (
    beacon_pyramid_effects,conduit,jukebox_allay,bell_ring,copper_aging,composter_hopper,
    monster_spawner_entity,decorated_pot_chiseled_bookshelf)}
