"""Explosion oracles: https://minecraft.wiki/w/Explosion and /TNT,
/Creeper, /End_Crystal, /Minecart_with_TNT, /Wind_Charge.
Ranges/distribution checks are explicitly distinguished from exact conservation.
"""
from ._blocks import (POS, actor, adapter, block, block_is, boundary, call, check,
    command, count, data, drops, entities, equip, field, observed, require_modes,
    scalar, setblock, ticks, use)


def _summon_tnt(ctx,x=1,y=144,z=0,fuse=1):
    command(ctx,f'summon tnt {x+.5} {y} {z+.5} {{fuse:{fuse}s}}')
    boundary(ctx)
    ticks(ctx,fuse+5)


def tnt_prime_chain(ctx):
    require_modes(ctx,'survival','creative'); setblock(ctx,'tnt'); setblock(ctx,'tnt',(2,144,0))
    equip(ctx,'flint_and_steel'); use(ctx); block(ctx,POS,'air')
    first=entities(ctx,'tnt'); check(ctx,'initial_prime_one',len(first)==1)
    events=observed(ctx,'entity_lifecycle')
    ticks(ctx,120); block(ctx,(2,144,0),'air'); check(ctx,'chain_entities_expired',not entities(ctx,'tnt'))
    history=observed(ctx,'entity_lifecycle')
    tnt=[e for e in history if e['type']=='minecraft:tnt']
    check(ctx,'initial_fuse_80',any(e['removed_tick']-e['spawn_tick']==80 for e in tnt),tnt)
    check(ctx,'chain_short_fuse',any(10<=e['removed_tick']-e['spawn_tick']<=30 for e in tnt),tnt)
    check(ctx,'crater_created',not block_is(ctx,(1,143,0),'stone'))


def creeper(ctx):
    require_modes(ctx,'survival','creative')
    for grief in ('true','false'):
        command(ctx,f'gamerule mob_griefing {grief}'); setblock(ctx,'dirt')
        command(ctx,'summon creeper 1.5 145 .5 {ignited:1b,Fuse:1s}')
        boundary(ctx); ticks(ctx,10); block(ctx,POS,'air' if grief=='true' else 'dirt')
    command(ctx,'gamerule mob_griefing true'); command(ctx,'summon skeleton 1.5 144 .5 {NoAI:1b,Health:1.0f}')
    command(ctx,'summon creeper 1.5 144 1.5 {powered:1b,ignited:1b,Fuse:1s}')
    ticks(ctx,10); check(ctx,'charged_creeper_head',drops(ctx,'skeleton_skull')+count(ctx,'skeleton_skull')==1)


def bed_anchor_wrong_dimension(ctx):
    require_modes(ctx,'survival','creative','adventure')
    dimension=ctx.factors['dimension']; equip(ctx)
    if dimension!='overworld':
        setblock(ctx,'red_bed[part=foot,facing=north]'); setblock(ctx,'red_bed[part=head,facing=north]',(1,144,-1)); use(ctx)
        block(ctx,POS,'air'); block(ctx,(1,144,-1),'air')
    if dimension!='the_nether':
        setblock(ctx,'respawn_anchor[charges=1]'); use(ctx); block(ctx,POS,'air')
    check(ctx,'wrong_dimension_explosion',bool(observed(ctx,'explosions')))


def end_crystal_explosion(ctx):
    require_modes(ctx,'survival','creative'); setblock(ctx,'obsidian'); equip(ctx,'end_crystal'); use(ctx)
    crystal=entities(ctx,'end_crystal')[0]; equip(ctx,'bow'); command(ctx,f'give {actor(ctx).name} arrow 1')
    actor(ctx).look_at(1.5,145.5,.5); call(actor(ctx),'use_item',ticks=20); call(actor(ctx),'release_use_item')
    ticks(ctx,20); check(ctx,'crystal_destroyed',not entities(ctx,'end_crystal'))
    blast=observed(ctx,'explosions')[-1]
    check(ctx,'power_six',field(blast,'power','snapshot:explosions')==6,blast)


def tnt_minecart(ctx):
    require_modes(ctx,'survival','creative')
    for cause in ('activator','fall'):
        if cause=='activator':
            setblock(ctx,'redstone_block',(1,143,0)); setblock(ctx,'activator_rail[powered=true]')
            command(ctx,'summon tnt_minecart 1.5 144 .5')
        else:
            command(ctx,'summon tnt_minecart 1.5 154 .5 {Motion:[0.5d,-1.0d,0.0d]}')
        boundary(ctx); ticks(ctx,120); check(ctx,'minecart_exploded:'+cause,not entities(ctx,'tnt_minecart'))
        check(ctx,'minecart_blast_present:'+cause,bool(observed(ctx,'explosions')))


def ghast_wither_fireballs(ctx):
    for projectile in ('fireball','wither_skull'):
        setblock(ctx,'stone'); command(ctx,f'summon {projectile} 1.5 144 3.5 {{Motion:[0.0d,0.0d,-1.0d]}}')
        boundary(ctx); ticks(ctx,40); check(ctx,'projectile_consumed:'+projectile,not entities(ctx,projectile))
        blast=observed(ctx,'explosions')[-1]; check(ctx,'projectile_power_one',field(blast,'power','snapshot:explosions')==1,blast)


def blast_resistance(ctx):
    for resistant in ('obsidian','bedrock'):
        setblock(ctx,resistant); _summon_tnt(ctx,1,145,0); block(ctx,POS,resistant)
    command(ctx,'fill -1 143 -1 3 143 2 stone'); command(ctx,'fill 0 144 -1 2 146 1 water')
    _summon_tnt(ctx,1,145,0); block(ctx,(1,143,0),'stone')


def explosion_drop_decay(ctx):
    require_modes(ctx,'creative')
    for rule,source in [('tnt_explosion_drop_decay','tnt'),('mob_explosion_drop_decay','creeper'),('block_explosion_drop_decay','anchor')]:
        totals={}
        for enabled in ('false','true'):
            command(ctx,f'gamerule {rule} {enabled}'); command(ctx,'kill @e[type=item]')
            command(ctx,'fill -2 144 -2 4 146 2 dirt'); command(ctx,'setblock 1 145 0 air')
            if source=='tnt': _summon_tnt(ctx,1,145,0)
            elif source=='creeper':
                command(ctx,'summon creeper 1.5 145 .5 {ignited:1b,Fuse:1s}'); boundary(ctx); ticks(ctx,10)
            else:
                if ctx.factors['dimension']=='the_nether':
                    from ..matrix_api import MissingCapability
                    raise MissingCapability('fixture:block_explosion_in_nether')
                setblock(ctx,'respawn_anchor[charges=1]',(1,145,0)); equip(ctx); use(ctx,(1,145,0)); ticks(ctx,10)
            image=observed(ctx,'block_region')
            remaining=sum(b['block']=='minecraft:dirt' and -2<=b['position'][0]<=4 and 144<=b['position'][1]<=146 and -2<=b['position'][2]<=2 for b in image)
            broken=104-remaining; loot=drops(ctx,'dirt')+count(ctx,'dirt')
            check(ctx,'drop_decay_bounds:'+rule+':'+enabled,0<=loot<=broken,{'broken':broken,'loot':loot})
            if enabled=='false': check(ctx,'no_decay_exact:'+rule,loot==broken)
            totals[enabled]=(broken,loot)
        check(ctx,'decay_reduces_drops:'+rule,totals['true'][1]<totals['true'][0],totals)


def tnt_explodes_rule(ctx):
    command(ctx,'gamerule tnt_explodes false'); setblock(ctx,'dirt'); _summon_tnt(ctx,1,145,0)
    block(ctx,POS,'dirt'); check(ctx,'no_tnt_blast',not observed(ctx,'explosions'))


def blast_protection_knockback(ctx):
    require_modes(ctx,'survival','adventure'); results=[]
    for enchanted in (False,True):
        command(ctx,f'effect give {actor(ctx).name} instant_health 1 10 true')
        command(ctx,f'tp {actor(ctx).name} 4.5 144 .5'); equip(ctx,'diamond_chestplate'+('[enchantments={"minecraft:blast_protection":4}]' if enchanted else ''))
        call(actor(ctx),'open_inventory'); actor(ctx).click_slot(36); actor(ctx).click_slot(6); call(actor(ctx),'close_container')
        before=observed(ctx,'health'); _summon_tnt(ctx,1,144,0)
        impulse=observed(ctx,'explosion_impulses')[-1]
        results.append((before-observed(ctx,'health'),sum(v*v for v in impulse)))
    check(ctx,'blast_damage_reduced',results[1][0]<results[0][0],results)
    check(ctx,'blast_knockback_reduced',results[1][1]<results[0][1],results)


def wind_charge(ctx):
    require_modes(ctx,'survival','creative'); setblock(ctx,'oak_door[half=lower,open=false]')
    setblock(ctx,'oak_door[half=upper,open=false]',(1,145,0)); setblock(ctx,'stone_button[face=floor]',(2,144,0))
    equip(ctx,'wind_charge',2); actor(ctx).look_at(1.5,144.5,.5); call(actor(ctx),'use_item'); boundary(ctx); ticks(ctx,10)
    block(ctx,POS,'oak_door[open=true]'); block(ctx,(1,143,0),'stone')
    block(ctx,(2,144,0),'stone_button[powered=true]','wind_charge_activates_button')
    check(ctx,'wind_knockback',bool(observed(ctx,'explosion_impulses')))
    command(ctx,'summon breeze_wind_charge 1.5 144 2.5 {Motion:[0.0d,0.0d,-1.0d]}'); ticks(ctx,10)
    block(ctx,(1,143,0),'stone'); check(ctx,'breeze_no_block_damage',block_is(ctx,POS,'oak_door'))


ADAPTERS={f'explosions.{fn.__name__}':adapter(fn) for fn in (
    tnt_prime_chain,creeper,bed_anchor_wrong_dimension,end_crystal_explosion,tnt_minecart,
    ghast_wither_fireballs,blast_resistance,explosion_drop_decay,tnt_explodes_rule,
    blast_protection_knockback,wind_charge)}
