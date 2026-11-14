"""Falling/archaeology oracles: https://minecraft.wiki/w/Falling_Block,
https://minecraft.wiki/w/Anvil, /Pointed_Dripstone, /Dragon_Egg, /Suspicious_Sand.
Pinned BrushableBlock/DragonEggBlock javap lives in scenarios/domains/evidence.
"""
from ._blocks import (POS, actor, adapter, block, block_is, boundary, call, check,
    command, count, data, drops, entities, entity_health, equip, mine, observed, require_modes,
    setblock, ticks, use)


def sand_gravel(ctx):
    require_modes(ctx,'survival','creative')
    for material in ('sand','red_sand','gravel'):
        for support in ('stone','torch','oak_slab[type=bottom]'):
            command(ctx,'kill @e[type=item]'); setblock(ctx,'stone',(1,143,0)); setblock(ctx,support)
            setblock(ctx,'stone',(1,148,0)); setblock(ctx,material,(1,149,0))
            equip(ctx,'diamond_pickaxe'); mine(ctx,(1,148,0)); ticks(ctx,60)
            if support=='stone': block(ctx,(1,145,0),material)
            else:
                block(ctx,(1,145,0),'air'); check(ctx,'fall_break_item:'+material,drops(ctx,material)+count(ctx,material)==1)
            check(ctx,'no_falling_ghost',len(entities(ctx,'falling_block'))==0)
            command(ctx,'fill 1 144 0 1 150 0 air')


def suspicious_blocks(ctx):
    require_modes(ctx,'survival','creative')
    for material,base in [('suspicious_sand','sand'),('suspicious_gravel','gravel')]:
        setblock(ctx,'stone',(1,148,0)); setblock(ctx,material,(1,149,0)); equip(ctx,'diamond_pickaxe')
        mine(ctx,(1,148,0)); ticks(ctx,60)
        check(ctx,'archaeology_fall_no_item:'+material,drops(ctx,material)==0 and drops(ctx,'diamond')==0)
        block(ctx,POS,'air')
        equip(ctx,material+'[block_entity_data={id:"minecraft:brushable_block",item:{id:"minecraft:diamond",count:1}}]')
        use(ctx,(1,143,0)); equip(ctx,'brush'); actor(ctx).look_at(1.5,144.5,.5)
        call(actor(ctx),'use_item',ticks=200); boundary(ctx)
        block(ctx,POS,base); check(ctx,'archaeology_loot_once',count(ctx,'diamond')+drops(ctx,'diamond')==1)
        command(ctx,'kill @e[type=item]'); setblock(ctx,'air')


def anvil_fall_damage(ctx):
    require_modes(ctx,'survival','adventure')
    command(ctx,'summon iron_golem 1.5 144 .5 {NoAI:1b}')
    ctx.wait_until(lambda:any(e.get('health') is not None for e in entities(ctx,'iron_golem')),
                   description='received golem spawn and health')
    before=entity_health(ctx,'iron_golem')
    setblock(ctx,'stone',(1,151,0)); setblock(ctx,'anvil',(1,152,0)); setblock(ctx,'air',(1,151,0)); boundary(ctx)
    ticks(ctx,60); after=entity_health(ctx,'iron_golem')
    check(ctx,'anvil_damage',0<before-after<=40,{'before':before,'after':after})
    check(ctx,'anvil_can_chip',any(block_is(ctx,POS,s) for s in ('anvil','chipped_anvil')))


def dragon_egg(ctx):
    require_modes(ctx,'survival','creative'); equip(ctx); setblock(ctx,'dragon_egg'); use(ctx)
    block(ctx,POS,'air')
    image=observed(ctx,'block_region'); eggs=[b for b in image if b['block']=='minecraft:dragon_egg']
    check(ctx,'egg_teleported_once',len(eggs)==1,eggs)
    if not eggs: return
    pos=tuple(eggs[0]['position']); check(ctx,'egg_teleport_bounds',abs(pos[0]-1)<=15 and abs(pos[1]-144)<=7 and abs(pos[2])<=15,pos)
    setblock(ctx,'stone',(1,148,0)); setblock(ctx,'dragon_egg',(1,149,0)); setblock(ctx,'air',(1,148,0)); ticks(ctx,60)
    block(ctx,POS,'dragon_egg')


def pointed_dripstone(ctx):
    require_modes(ctx,'survival','adventure'); command(ctx,'summon iron_golem 1.5 144 .5 {NoAI:1b}')
    ctx.wait_until(lambda:any(e.get('health') is not None for e in entities(ctx,'iron_golem')),
                   description='received golem spawn and health')
    before=entity_health(ctx,'iron_golem')
    setblock(ctx,'stone',(1,154,0)); setblock(ctx,'pointed_dripstone[vertical_direction=down,thickness=tip]',(1,153,0))
    setblock(ctx,'air',(1,154,0)); boundary(ctx); ticks(ctx,60)
    after=entity_health(ctx,'iron_golem')
    check(ctx,'stalactite_damage',0<before-after<=40,{'before':before,'after':after})
    check(ctx,'stalactite_shattered',len(entities(ctx,'falling_block'))==0)


def falling_into_void_or_unloaded(ctx):
    minimum=-64 if ctx.factors['dimension']=='overworld' else 0
    command(ctx,f'fill 1 {minimum} 0 1 {minimum+4} 0 air')
    command(ctx,f'tp {actor(ctx).name} 3.5 {minimum+2} .5')
    setblock(ctx,'stone',(1,minimum+2,0)); setblock(ctx,'sand',(1,minimum+3,0))
    setblock(ctx,'air',(1,minimum+2,0)); boundary(ctx); ticks(ctx,120)
    check(ctx,'void_no_falling_body',len(entities(ctx,'falling_block'))==0)
    # Chunk-unload must be a real ticket release, not a simulated timeout.
    call(ctx.server,'forceload_fixture_chunk',0,0)
    setblock(ctx,'stone',(1,154,0)); setblock(ctx,'sand',(1,155,0)); setblock(ctx,'air',(1,154,0))
    call(ctx.server,'unload_fixture_chunk',0,0); ctx.server.save_all()
    call(ctx.server,'forceload_fixture_chunk',0,0); ticks(ctx,60)
    check(ctx,'unloaded_entity_no_duplicate',len(entities(ctx,'falling_block'))<=1)
    block(ctx,POS,'sand')


ADAPTERS={f'falling_blocks.{fn.__name__}':adapter(fn) for fn in
          (sand_gravel,suspicious_blocks,anvil_fall_damage,dragon_egg,pointed_dripstone,falling_into_void_or_unloaded)}
