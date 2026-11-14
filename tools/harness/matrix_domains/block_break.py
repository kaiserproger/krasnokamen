"""Mining oracles use 26.1.2 imported loot_table/blocks and
https://minecraft.wiki/w/Breaking; tool/time modifiers must be measured in ticks.
"""
from ._blocks import (POS, actor, adapter, block, call, check, command, count, data,
    drops, entities, equip, experience, orb_experience, field, mine, observed, require_modes, scalar, setblock, ticks)


def _break_drop(ctx, initial, tool, item, expected):
    command(ctx,'kill @e[type=item]'); equip(ctx,tool); setblock(ctx,initial)
    mine(ctx); block(ctx,POS,'air')
    actual=drops(ctx,item)+count(ctx,item)
    check(ctx,'loot:'+initial+':'+str(tool),actual==expected,{'expected':expected,'actual':actual})


def instant_break(ctx):
    require_modes(ctx,'survival')
    for initial,ground,item in [('dandelion','dirt','dandelion'),('torch','stone','torch'),
                                ('wheat[age=0]','farmland','wheat_seeds'),('oak_sapling','dirt','oak_sapling')]:
        setblock(ctx,ground,(1,143,0)); _break_drop(ctx,initial,None,item,1)
        history=observed(ctx,'mining_events')
        check(ctx,'zero_hardness_one_tick',history[-1]['finish_tick']-history[-1]['start_tick']<=1)


def tool_tier_required(ctx):
    require_modes(ctx,'survival')
    for material,tier in [('wooden',0),('stone',1),('iron',2),('golden',0),('diamond',3),('netherite',4)]:
        for ore,required,item in [('stone',0,'cobblestone'),('iron_ore',1,'raw_iron'),('diamond_ore',2,'diamond'),('obsidian',3,'obsidian')]:
            _break_drop(ctx,ore,material+'_pickaxe',item,int(tier>=required))
    # No wall-clock extrapolation: captured start/finish packets + dashboard tick.
    check(ctx,'mining_tick_observations_present',len(observed(ctx,'mining_events'))>=24)


def wrong_tool_no_drop(ctx):
    require_modes(ctx,'survival'); _break_drop(ctx,'stone',None,'cobblestone',0)
    event=observed(ctx,'mining_events')[-1]
    check(ctx,'stone_hand_150_ticks',event['finish_tick']-event['start_tick']+1==150,event)


def silk_touch(ctx):
    require_modes(ctx,'survival')
    for item in ('diamond_ore','glass','grass_block','ice'):
        _break_drop(ctx,item,'diamond_pickaxe[enchantments={"minecraft:silk_touch":1}]',item,1)


def fortune_drops(ctx):
    require_modes(ctx,'survival')
    totals=[]
    for ore,item in [('coal_ore','coal'),('diamond_ore','diamond')]:
        values=[]
        for _ in range(64):
            command(ctx,'kill @e[type=item]'); equip(ctx,'diamond_pickaxe[enchantments={"minecraft:fortune":3}]')
            setblock(ctx,ore); mine(ctx); values.append(drops(ctx,item)+count(ctx,item))
        check(ctx,'fortune_allowed_support:'+ore,all(1<=v<=4 for v in values),values)
        # Mean 2.2, bounded support. This is an explicitly statistical oracle,
        # not a claim that every finite vanilla sequence must contain a bonus.
        check(ctx,'fortune_distribution:'+ore,1.5<=sum(values)/len(values)<=2.9,values)
        totals.extend(values)
    for crop,seed in [('wheat','wheat_seeds'),('carrots','carrot'),('potatoes','potato'),('beetroots','beetroot_seeds')]:
        setblock(ctx,'farmland[moisture=7]',(1,143,0)); setblock(ctx,crop+'[age='+('3' if crop=='beetroots' else '7')+']')
        equip(ctx,'diamond_hoe[enchantments={"minecraft:fortune":3}]'); mine(ctx)
        check(ctx,'fortune_crop_produce:'+crop,drops(ctx,seed)+count(ctx,seed)>=1)


def container_spill(ctx):
    require_modes(ctx,'survival')
    for kind,capacity in [('chest',27),('barrel',27),('furnace',3),('hopper',5)]:
        command(ctx,'kill @e[type=item]'); setblock(ctx,kind); equip(ctx,'diamond',7)
        actor(ctx).open_container(POS)
        menu=observed(ctx,'menu'); source=capacity+27
        actor(ctx).click_slot(source); actor(ctx).click_slot(0); call(actor(ctx), 'close_container')
        equip(ctx,'diamond_pickaxe'); mine(ctx)
        check(ctx,'contents_once:'+kind,drops(ctx,'diamond')+count(ctx,'diamond')==7)
        check(ctx,'container_once:'+kind,drops(ctx,kind)+count(ctx,kind)==1)


def shulker_keeps_contents(ctx):
    require_modes(ctx,'survival'); setblock(ctx,'shulker_box'); equip(ctx,'diamond',7)
    actor(ctx).open_container(POS); actor(ctx).click_slot(54); actor(ctx).click_slot(0); call(actor(ctx), 'close_container')
    equip(ctx,'diamond_pickaxe'); mine(ctx)
    check(ctx,'no_loose_contents',drops(ctx,'diamond')==0)
    check(ctx,'one_shulker',drops(ctx,'shulker_box')==1)
    payload=data(ctx,'entity @e[type=item,limit=1,sort=nearest]','Item')
    check(ctx,'retained_container_component','minecraft:container' in payload and 'minecraft:diamond' in payload,payload)


def double_block(ctx):
    require_modes(ctx,'survival','creative')
    for kind,first,second,other in [('oak_door','half=lower','half=upper',(1,145,0)),
                                   ('red_bed','part=foot,facing=north','part=head,facing=north',(1,144,-1)),
                                   ('sunflower','half=lower','half=upper',(1,145,0))]:
        for target in (POS,other):
            command(ctx,'kill @e[type=item]'); setblock(ctx,'dirt',(1,143,0))
            setblock(ctx,f'{kind}[{first}]'); setblock(ctx,f'{kind}[{second}]',other); equip(ctx); mine(ctx,target)
            block(ctx,POS,'air'); block(ctx,other,'air')
            check(ctx,'double_block_one_drop',drops(ctx,kind)+count(ctx,kind)==(0 if ctx.factors['game_mode']=='creative' else 1))


def support_cascade(ctx):
    require_modes(ctx,'survival')
    for attachment in ('torch','rail','redstone_wire','oak_sign'):
        command(ctx,'kill @e[type=item]'); setblock(ctx,'stone'); setblock(ctx,attachment,(1,145,0))
        equip(ctx,'diamond_pickaxe'); mine(ctx); block(ctx,(1,145,0),'air')
        item='redstone' if attachment=='redstone_wire' else attachment
        check(ctx,'unsupported_drop:'+attachment,drops(ctx,item)+count(ctx,item)==1)


def gravity_trigger(ctx):
    require_modes(ctx,'survival')
    for kind in ('sand','gravel'):
        setblock(ctx,'stone'); setblock(ctx,kind,(1,145,0)); setblock(ctx,kind,(1,146,0))
        equip(ctx,'diamond_pickaxe'); mine(ctx); ticks(ctx,40)
        block(ctx,POS,kind); block(ctx,(1,145,0),kind); block(ctx,(1,146,0),'air')


def ice_to_water(ctx):
    require_modes(ctx,'survival'); setblock(ctx,'ice'); equip(ctx,'diamond_pickaxe'); mine(ctx)
    block(ctx,POS,'air' if ctx.factors['dimension']=='the_nether' else 'water[level=0]')


def leaves_shears(ctx):
    require_modes(ctx,'survival'); _break_drop(ctx,'oak_leaves[persistent=true]','shears','oak_leaves',1)
    command(ctx,'kill @e[type=item]'); equip(ctx); setblock(ctx,'oak_leaves[persistent=true]'); mine(ctx)
    check(ctx,'hand_no_leaf',drops(ctx,'oak_leaves')==0)
    check(ctx,'hand_valid_loot',all(e['item']['item_id'] in {'minecraft:oak_sapling','minecraft:stick','minecraft:apple'} for e in entities(ctx,'item')))


def glass_no_drop(ctx):
    require_modes(ctx,'survival')
    for kind in ('glass','glass_pane'): _break_drop(ctx,kind,None,kind,0)


def unbreakable(ctx):
    require_modes(ctx,'survival'); equip(ctx,'netherite_pickaxe')
    for kind in ('bedrock','barrier','end_portal_frame'):
        setblock(ctx,kind); mine(ctx,phase='start'); ticks(ctx,200); mine(ctx,phase='abort')
        block(ctx,POS,kind); check(ctx,'unbreakable_no_drop',drops(ctx,kind)==0)


def creative_break(ctx):
    require_modes(ctx,'creative'); _break_drop(ctx,'stone',None,'cobblestone',0)
    setblock(ctx,'stone'); equip(ctx,'diamond_sword'); mine(ctx,phase='start'); ticks(ctx,2)
    block(ctx,POS,'stone'); check(ctx,'sword_no_drop',drops(ctx,'cobblestone')==0)


def ore_xp(ctx):
    require_modes(ctx,'survival')
    for ore,low,high in [('coal_ore',0,2),('lapis_ore',2,5),('redstone_ore',1,5),
                         ('diamond_ore',3,7),('emerald_ore',3,7),('nether_quartz_ore',2,5),('nether_gold_ore',0,1)]:
        for silk in (False,True):
            command(ctx,'kill @e[type=experience_orb]'); setblock(ctx,ore)
            equip(ctx,'diamond_pickaxe'+('[enchantments={"minecraft:silk_touch":1}]' if silk else ''))
            before=experience(ctx); mine(ctx)
            total=experience(ctx)-before+orb_experience(ctx)
            check(ctx,'ore_xp:'+ore+':'+str(silk),total==0 if silk else low<=total<=high,total)


def spawner_xp(ctx):
    require_modes(ctx,'survival'); equip(ctx,'diamond_pickaxe'); setblock(ctx,'spawner')
    before=experience(ctx); mine(ctx)
    xp=experience(ctx)-before+orb_experience(ctx)
    check(ctx,'spawner_xp_range',15<=xp<=43,xp); check(ctx,'no_spawner_item',drops(ctx,'spawner')==0)


def block_entity_drop(ctx):
    require_modes(ctx,'survival')
    # Item components are the vanilla supported fixture route, not unimplemented data merge.
    for item,component in [('white_banner[banner_patterns=[{pattern:"minecraft:stripe_bottom",color:"red"}]]','minecraft:banner_patterns'),
                            ('player_head[profile={name:"Notch"}]','minecraft:profile'),
                            ('bee_nest[bees=[{entity_data:{id:"minecraft:bee"},ticks_in_hive:0,min_ticks_in_hive:600}]]','minecraft:bees')]:
        setblock(ctx,'air'); equip(ctx,item); actor(ctx).use_item_on((1,143,0))
        equip(ctx,'diamond_axe[enchantments={"minecraft:silk_touch":1}]'); mine(ctx)
        payload=data(ctx,'entity @e[type=item,limit=1,sort=nearest]','Item')
        check(ctx,'block_entity_component:'+component,component in payload,payload)
        command(ctx,'kill @e[type=item]')
    setblock(ctx,'oak_sign'); equip(ctx); actor(ctx).use_item_on(POS); call(actor(ctx), 'update_sign', POS, ['matrix','','','']); call(actor(ctx), 'close_container')
    equip(ctx,'diamond_axe'); mine(ctx)
    check(ctx,'sign_text_not_preserved','matrix' not in data(ctx,'entity @e[type=item,limit=1,sort=nearest]','Item'))


def crop_drops_by_age(ctx):
    require_modes(ctx,'survival')
    for crop,seed,produce,max_age in [('wheat','wheat_seeds','wheat',7),('carrots','carrot','carrot',7),
                                     ('potatoes','potato','potato',7),('beetroots','beetroot_seeds','beetroot',3)]:
        for age in (0,max_age):
            command(ctx,'kill @e[type=item]'); setblock(ctx,'farmland[moisture=7]',(1,143,0)); equip(ctx)
            setblock(ctx,f'{crop}[age={age}]'); mine(ctx)
            if age==0: check(ctx,'immature_seed:'+crop,drops(ctx,seed)+count(ctx,seed)==1)
            else: check(ctx,'mature_produce:'+crop,drops(ctx,produce)+count(ctx,produce)>=1)


def waterlogged_break(ctx):
    require_modes(ctx,'survival')
    for kind in ('oak_slab','oak_stairs'):
        setblock(ctx,kind+'[waterlogged=true]'); equip(ctx,'diamond_axe'); mine(ctx); block(ctx,POS,'water[level=0]')


def infested_block(ctx):
    require_modes(ctx,'survival'); setblock(ctx,'infested_stone'); equip(ctx,'diamond_pickaxe'); mine(ctx)
    check(ctx,'silverfish_spawned',len(entities(ctx,'silverfish'))==1); check(ctx,'no_infested_drop',drops(ctx,'infested_stone')==0)


def mining_speed_modifiers(ctx):
    require_modes(ctx,'survival'); times={}
    for name,effect,underwater,airborne,aqua in [('normal',None,False,False,False),('haste','haste',False,False,False),
        ('fatigue','mining_fatigue',False,False,False),('water',None,True,False,False),
        ('air',None,False,True,False),('aqua',None,True,False,True)]:
        command(ctx,f'effect clear {actor(ctx).name}'); setblock(ctx,'stone'); equip(ctx,'iron_pickaxe')
        command(ctx,'fill 0 144 2 2 146 4 air')
        command(ctx,f'tp {actor(ctx).name} 1.5 144 3.5')
        if underwater: command(ctx,'fill 0 144 2 2 146 4 water')
        if airborne:
            command(ctx,f'tp {actor(ctx).name} 1.5 144.5 3.5')
            command(ctx,f'effect give {actor(ctx).name} levitation 10 0 true')
        if aqua:
            command(ctx,f'give {actor(ctx).name} diamond_helmet[enchantments={{"minecraft:aqua_affinity":1}}]')
            ctx.wait_until(lambda:count(ctx,'diamond_helmet')==1,description='aqua-affinity helmet received')
            slot=next(s['slot'] for s in observed(ctx,'inventory') if s['item_id']=='minecraft:diamond_helmet')
            call(actor(ctx),'open_inventory'); actor(ctx).click_slot(slot); actor(ctx).click_slot(5); call(actor(ctx),'close_container')
        if effect: command(ctx,f'effect give {actor(ctx).name} {effect} 300 0 true')
        mine(ctx); event=observed(ctx,'mining_events')[-1]; times[name]=event['finish_tick']-event['start_tick']+1
    check(ctx,'haste_faster',times['haste']<times['normal'],times)
    check(ctx,'fatigue_slower',times['fatigue']>times['normal'],times)
    check(ctx,'water_and_air_fivefold',times['water']>=4*times['normal'] and times['air']>=4*times['normal'],times)
    check(ctx,'aqua_removes_water_penalty',times['aqua']==times['normal'],times)


def abort_dig(ctx):
    require_modes(ctx,'survival'); setblock(ctx,'obsidian'); equip(ctx,'diamond_pickaxe')
    mine(ctx,phase='start'); ticks(ctx,10); mine(ctx,phase='abort'); actor(ctx).look_at(-5,154,-5); ticks(ctx,120)
    block(ctx,POS,'obsidian'); check(ctx,'no_aborted_drop',drops(ctx,'obsidian')==0)
    check(ctx,'crack_animation_clear',not observed(ctx,'block_cracks'))


def tool_durability_on_break(ctx):
    require_modes(ctx,'survival'); equip(ctx,'wooden_pickaxe[damage=58]'); setblock(ctx,'stone'); mine(ctx)
    block(ctx,POS,'air'); check(ctx,'one_durability_tool_removed',count(ctx,'wooden_pickaxe')==0)
    check(ctx,'tool_break_event',any('break' in str(e) for e in observed(ctx,'events')))


ADAPTERS={f'block_break.{fn.__name__}':adapter(fn) for fn in (
    instant_break,tool_tier_required,wrong_tool_no_drop,silk_touch,fortune_drops,
    container_spill,shulker_keeps_contents,double_block,support_cascade,gravity_trigger,
    ice_to_water,leaves_shears,glass_no_drop,unbreakable,creative_break,ore_xp,spawner_xp,
    block_entity_drop,crop_drops_by_age,waterlogged_break,infested_block,
    mining_speed_modifiers,abort_dig,tool_durability_on_break)}
