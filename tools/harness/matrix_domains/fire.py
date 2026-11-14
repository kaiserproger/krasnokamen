"""Java 26.1.2 fire oracles.

Sources: https://minecraft.wiki/w/Fire ; local FireBlock javap and
26.1.2 tags/block/infiniburn_{overworld,end}.json. The matrix's doFireTick
spelling is obsolete in Java 26.1.2: fire_spread_radius_around_player is used.
"""
from ._blocks import (POS, actor, adapter, block, block_is, boundary, check,
    command, equip, observed, require_modes, scalar, setblock, ticks, use)


def fire_spread(ctx):
    # Disable spread with radius=0, leaving actual player context unchanged.
    command(ctx,'gamerule fire_spread_radius_around_player 0')
    setblock(ctx,'netherrack',(1,143,0)); setblock(ctx,'fire'); setblock(ctx,'oak_planks',(2,144,0))
    boundary(ctx); ticks(ctx,200)
    block(ctx,POS,'fire'); block(ctx,(2,144,0),'oak_planks')
    command(ctx,'gamerule fire_spread_radius_around_player -1')
    ctx.wait_until(lambda:not block_is(ctx,(2,144,0),'oak_planks'),
                   description='flammable block burns with enabled fire spread',timeout_seconds=120)
    check(ctx,'wood_burned',not block_is(ctx,(2,144,0),'oak_planks'))


def eternal_fire(ctx):
    command(ctx,'gamerule fire_spread_radius_around_player -1')
    for base,fire in [('netherrack','fire'),('magma_block','fire'),('soul_sand','soul_fire'),('soul_soil','soul_fire')]:
        setblock(ctx,base,(1,143,0)); setblock(ctx,fire); boundary(ctx); ticks(ctx,700)
        block(ctx,POS,fire)
    if ctx.factors['dimension']=='the_end':
        setblock(ctx,'bedrock',(1,143,0)); setblock(ctx,'fire[age=15]'); ticks(ctx,80)
        block(ctx,POS,'fire')


def lava_ignites(ctx):
    command(ctx,'gamerule fire_spread_radius_around_player -1')
    command(ctx,'gamerule random_tick_speed 128')
    setblock(ctx,'lava',(1,144,0)); setblock(ctx,'oak_planks',(1,146,0)); boundary(ctx)
    cells=[(x,145,z) for x in (0,1,2) for z in (-1,0,1)]
    ctx.wait_until(lambda:any(block_is(ctx,p,'fire') for p in cells),
                   description='lava ignites adjacent fuel air',timeout_seconds=120)
    check(ctx,'lava_ignition_observed',any(block_is(ctx,p,'fire') for p in cells))


def entity_burning(ctx):
    require_modes(ctx,'survival','adventure')
    for hazard in ('fire','lava'):
        setblock(ctx,'netherrack',(1,143,0)); setblock(ctx,hazard)
        command(ctx,f'tp {actor(ctx).name} 1.5 144 .5'); boundary(ctx); ticks(ctx,25)
        check(ctx,'burning:'+hazard,scalar(ctx,f'entity {actor(ctx).name}','Fire')>0)
        setblock(ctx,'water',(3,144,0)); command(ctx,f'tp {actor(ctx).name} 3.5 144 .5'); ticks(ctx,2)
        check(ctx,'water_extinguishes:'+hazard,scalar(ctx,f'entity {actor(ctx).name}','Fire')<=0)
        command(ctx,f'effect give {actor(ctx).name} instant_health 1 10 true')
    if ctx.factors['dimension']=='overworld' and ctx.factors['difficulty']!='peaceful':
        command(ctx,'time set noon'); command(ctx,'weather clear'); command(ctx,'summon zombie 2.5 144 .5 {NoAI:1b}')
        ticks(ctx,40); check(ctx,'undead_sunlight',scalar(ctx,'entity @e[type=zombie,limit=1]','Fire')>0)
        command(ctx,'weather rain'); ticks(ctx,40)
        check(ctx,'rain_extinguishes',scalar(ctx,'entity @e[type=zombie,limit=1]','Fire')<=0)


def candles_campfire_light(ctx):
    require_modes(ctx,'survival','creative','adventure')
    for item,light in [('candle',3),('campfire',15),('soul_campfire',10)]:
        setblock(ctx,item+'[lit=false]'); equip(ctx,'flint_and_steel'); use(ctx)
        block(ctx,POS,item+'[lit=true]')
        lights=observed(ctx,'block_light')
        check(ctx,'light_level:'+item,lights.get('1,144,0')==light,lights)
        equip(ctx,'iron_shovel' if 'campfire' in item else None); use(ctx)
        block(ctx,POS,item+'[lit=false]')


ADAPTERS={f'fire.{fn.__name__}':adapter(fn) for fn in
          (fire_spread,eternal_fire,lava_ignites,entity_burning,candles_campfire_light)}
