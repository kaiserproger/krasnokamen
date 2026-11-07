"""Public interaction scenarios, Java 26.1.2.

References are the corresponding minecraft.wiki block/item Usage pages; pinned
NoteBlock/ComposterBlock/DoorBlock bytecode is retained under scenarios/domains/evidence.
"""
from ._blocks import (POS, actor, adapter, block, block_is, call, check, command,
    boundary, count, data, drops, entities, equip, field, mine, observed, query,
    require_modes, scalar, setblock, ticks, use)


def door_trapdoor_gate_toggle(ctx):
    equip(ctx)
    spectator=ctx.factors['game_mode']=='spectator'
    for kind in ('oak_door','oak_trapdoor','oak_fence_gate','iron_door','iron_trapdoor'):
        setblock(ctx,kind+'[open=false]')
        if kind.endswith('_door'): setblock(ctx,kind+'[half=upper,open=false]',(1,145,0))
        for expected in ('true','false'):
            use(ctx); opened='false' if spectator or kind.startswith('iron_') else expected
            block(ctx,POS,f'{kind}[open={opened}]')
            if kind.endswith('_door'): block(ctx,(1,145,0),f'{kind}[open={opened}]')
        setblock(ctx,'air',(1,145,0))


def lever_button(ctx):
    equip(ctx); spectator=ctx.factors['game_mode']=='spectator'
    setblock(ctx,'lever[face=floor,powered=false]')
    for power in ('true','false'):
        use(ctx); block(ctx,POS,f'lever[powered={"false" if spectator else power}]')
    for button,duration in [('stone_button',20),('oak_button',30)]:
        setblock(ctx,button+'[face=floor,powered=false]'); use(ctx)
        block(ctx,POS,button+('[powered=false]' if spectator else '[powered=true]')); ticks(ctx,duration)
        block(ctx,POS,button+'[powered=false]')
        if spectator:
            continue
        # Exact timing comes from the event history, not command round-trip time.
        transitions=observed(ctx,'block_transitions')
        times=[e['tick'] for e in transitions if tuple(e['position'])==POS and e['block']==button]
        check(ctx,'button_duration:'+button,len(times)>=2 and times[-1]-times[-2]==duration,times)


def note_block_tune(ctx):
    equip(ctx); spectator=ctx.factors['game_mode']=='spectator'
    for base,instrument in [('dirt','harp'),('stone','basedrum'),('sand','snare'),('glass','hat'),('oak_planks','bass')]:
        setblock(ctx,base,(1,143,0)); setblock(ctx,'note_block[note=0]')
        for note in range(1,26):
            use(ctx); block(ctx,POS,f'note_block[note={0 if spectator else note%25},instrument={instrument}]')


def repeater_comparator_daylight(ctx):
    equip(ctx); immutable=ctx.factors['game_mode'] in ('spectator','adventure')
    for kind,states in [('repeater',['delay=2','delay=3','delay=4','delay=1']),
                         ('comparator',['mode=subtract','mode=compare']),
                         ('daylight_detector',['inverted=true','inverted=false'])]:
        setblock(ctx,kind)
        for state in states:
            use(ctx); expected=states[-1] if immutable else state
            block(ctx,POS,f'{kind}[{expected}]')


def jukebox(ctx):
    require_modes(ctx,'survival','creative','adventure'); setblock(ctx,'jukebox')
    equip(ctx,'music_disc_13'); use(ctx); block(ctx,POS,'jukebox[has_record=true]')
    setblock(ctx,'comparator[facing=east]',(2,144,0)); ticks(ctx,2)
    block(ctx,(2,144,0),'comparator[powered=true]')
    equip(ctx); use(ctx); block(ctx,POS,'jukebox[has_record=false]')
    check(ctx,'one_disc_ejected',drops(ctx,'music_disc_13')+count(ctx,'music_disc_13')==1)


def sign_edit_decorate(ctx):
    require_modes(ctx,'survival','creative','adventure'); setblock(ctx,'oak_sign'); equip(ctx)
    use(ctx); call(actor(ctx), 'update_sign', POS, ['matrix','','','']); call(actor(ctx), 'close_container')
    check(ctx,'text_saved','matrix' in data(ctx,'block 1 144 0','front_text'))
    for item,needle in [('red_dye','red'),('glow_ink_sac','has_glowing_text:1b')]:
        equip(ctx,item); use(ctx); check(ctx,'sign_decoration:'+item,needle in data(ctx,'block 1 144 0','front_text'))
    equip(ctx,'honeycomb'); use(ctx); check(ctx,'wax_saved',scalar(ctx,'block 1 144 0','is_waxed')==1)
    equip(ctx); use(ctx); check(ctx,'wax_blocks_editor',observed(ctx,'screen')['type']!='sign_editor')


def vault_key(ctx):
    require_modes(ctx,'survival','adventure')
    for ominous,key in [('false','trial_key'),('true','ominous_trial_key')]:
        setblock(ctx,f'vault[ominous={ominous}]'); equip(ctx,key,2)
        before=count(ctx,key); use(ctx)
        ctx.wait_until(lambda:count(ctx,key)==before-1,description='vault accepted first key')
        ticks(ctx,200); reward_before=data(ctx,'block 1 144 0','server_data.rewarded_players')
        use(ctx); ticks(ctx,40)
        check(ctx,'vault_one_key_only:'+key,count(ctx,key)==before-1)
        check(ctx,'vault_rewarded_player_once:'+key,data(ctx,'block 1 144 0','server_data.rewarded_players')==reward_before)


def ignite(ctx):
    require_modes(ctx,'survival','creative')
    for tool in ('flint_and_steel','fire_charge'):
        for target,expected in [('stone','fire'),('tnt','air'),('candle[lit=false]','candle[lit=true]'),('campfire[lit=false]','campfire[lit=true]')]:
            setblock(ctx,'air',(1,145,0)); setblock(ctx,target); equip(ctx,tool,2 if tool=='fire_charge' else 1)
            use(ctx); block(ctx,(1,145,0) if target=='stone' else POS,expected)
            if target=='tnt': check(ctx,'tnt_entity_primed',len(entities(ctx,'tnt'))==1)
            if ctx.factors['game_mode']=='survival':
                if tool=='fire_charge': check(ctx,'charge_consumed',count(ctx,tool)==1)
                else: check(ctx,'flint_durability',scalar(ctx,f'entity {actor(ctx).name}','Inventory[0].components."minecraft:damage"')==1)
        command(ctx,'fill 1 144 0 4 148 0 obsidian'); command(ctx,'fill 2 145 0 3 147 0 air')
        equip(ctx,tool); use(ctx,(2,144,0)); block(ctx,(2,145,0),'nether_portal')


def axe_strip_scrape_unwax(ctx):
    allowed=ctx.factors['game_mode'] in ('survival','creative')
    for initial,expected in [('oak_log[axis=x]','stripped_oak_log[axis=x]'),
                              ('oxidized_copper','weathered_copper'),('waxed_weathered_copper','weathered_copper')]:
        setblock(ctx,initial); equip(ctx,'iron_axe'); use(ctx); block(ctx,POS,expected if allowed else initial)


def shovel_path(ctx):
    allowed=ctx.factors['game_mode'] in ('survival','creative')
    for initial,expected in [('grass_block','dirt_path'),('dirt','dirt_path'),('campfire[lit=true]','campfire[lit=false]')]:
        setblock(ctx,initial); equip(ctx,'iron_shovel'); use(ctx); block(ctx,POS,expected if allowed else initial)


def hoe_till(ctx):
    allowed=ctx.factors['game_mode'] in ('survival','creative')
    for initial,expected in [('dirt','farmland'),('grass_block','farmland'),('coarse_dirt','dirt'),('rooted_dirt','dirt')]:
        setblock(ctx,initial); equip(ctx,'iron_hoe'); use(ctx); block(ctx,POS,expected if allowed else initial)
        if initial=='rooted_dirt':
            check(ctx,'hanging_roots',drops(ctx,'hanging_roots')+count(ctx,'hanging_roots')==(1 if allowed else 0))


def bone_meal(ctx):
    require_modes(ctx,'survival','creative')
    for plant,ground,result in [('wheat[age=0]','farmland[moisture=7]','wheat[age=0]'),
                                ('oak_sapling','dirt','oak_sapling'),('short_grass','grass_block','short_grass'),
                                ('moss_block','stone','moss_block'),('sunflower[half=lower]','dirt','sunflower'),
                                ('red_mushroom','mycelium','red_mushroom')]:
        command(ctx,'fill -4 144 -4 6 158 4 air')
        command(ctx,f'fill -4 143 -4 6 143 4 {ground}')
        if plant=='moss_block': command(ctx,'fill -4 144 -4 6 144 4 stone')
        setblock(ctx,plant)
        if plant.startswith('sunflower'): setblock(ctx,'sunflower[half=upper]',(1,145,0))
        equip(ctx,'bone_meal',64); before=data(ctx,f'entity {actor(ctx).name}','Inventory')
        # Bounded stochastic trial: a failure retains the seed and observations;
        # it is not an exact proof of a deterministic growth-time mismatch.
        for _ in range(64):
            if count(ctx,'bone_meal')==0: break
            use(ctx)
            if plant.startswith('wheat') and not block_is(ctx,POS,result): break
        image=observed(ctx,'block_region')
        expected={'oak_sapling':'minecraft:oak_log','short_grass':'minecraft:tall_grass',
                  'moss_block':'minecraft:moss_carpet','red_mushroom':'minecraft:red_mushroom_block'}
        if plant.startswith('wheat'):
            grown=any(block_is(ctx,POS,f'wheat[age={age}]') for age in range(1,8))
        elif plant.startswith('sunflower'):
            grown=drops(ctx,'sunflower')+count(ctx,'sunflower')>0
        else:
            grown=any(b['block']==expected[plant] for b in image)
        check(ctx,'growth_observed:'+plant,grown)
        if ctx.factors['game_mode']=='survival': check(ctx,'bone_meal_consumed:'+plant,before!=data(ctx,f'entity {actor(ctx).name}','Inventory'))


def beehive_harvest(ctx):
    require_modes(ctx,'survival','creative')
    for smoke in (False,True):
        for item,product in [('shears','honeycomb'),('glass_bottle','honey_bottle')]:
            setblock(ctx,'campfire[lit=true]' if smoke else 'stone',(1,143,0))
            setblock(ctx,'beehive[honey_level=5]'); command(ctx,'summon bee 1.5 145 .5 {NoAI:1b}')
            equip(ctx,item); use(ctx); block(ctx,POS,'beehive[honey_level=0]')
            check(ctx,'hive_yield:'+item,count(ctx,product)+drops(ctx,product)==(3 if product=='honeycomb' else 1))
            anger=scalar(ctx,'entity @e[type=bee,limit=1,sort=nearest]','AngerTime')
            check(ctx,'smoke_prevents_anger',anger==0 if smoke else anger>0)
            command(ctx,'kill @e[type=bee]')


def cauldron(ctx):
    require_modes(ctx,'survival')
    setblock(ctx,'cauldron'); equip(ctx,'water_bucket'); use(ctx)
    block(ctx,POS,'water_cauldron[level=3]'); check(ctx,'bucket_returned',count(ctx,'bucket')==1)
    equip(ctx,'glass_bottle',3)
    for level in (2,1,0):
        use(ctx); block(ctx,POS,f'water_cauldron[level={level}]' if level else 'cauldron')
    check(ctx,'three_water_bottles',count(ctx,'potion')==3)
    # Java washes existing leather dye; it does NOT dye leather in a cauldron.
    setblock(ctx,'water_cauldron[level=3]'); equip(ctx,'leather_chestplate[dyed_color=16711680]'); use(ctx)
    block(ctx,POS,'water_cauldron[level=2]')
    check(ctx,'leather_dye_removed','minecraft:dyed_color' not in data(ctx,f'entity {actor(ctx).name}','Inventory'))


def campfire_cook(ctx):
    require_modes(ctx,'survival'); setblock(ctx,'campfire[lit=true]'); equip(ctx,'beef',4)
    for _ in range(4): use(ctx)
    check(ctx,'four_raw_debits',count(ctx,'beef')==0); ticks(ctx,600)
    check(ctx,'four_cooked_items',count(ctx,'cooked_beef')+drops(ctx,'cooked_beef')==4)
    mine(ctx); check(ctx,'no_cooked_duplication',count(ctx,'cooked_beef')+drops(ctx,'cooked_beef')==4)


def composter(ctx):
    require_modes(ctx,'survival'); setblock(ctx,'composter'); equip(ctx,'cake',7)
    # Cake composting is 100%; no random timeout can misclassify a server bug.
    for level in range(1,8): use(ctx); block(ctx,POS,f'composter[level={level}]')
    ticks(ctx,20); block(ctx,POS,'composter[level=8]'); equip(ctx); use(ctx)
    block(ctx,POS,'composter[level=0]'); check(ctx,'one_bone_meal',drops(ctx,'bone_meal')+count(ctx,'bone_meal')==1)


def armor_stand_use(ctx):
    require_modes(ctx,'survival'); equip(ctx,'armor_stand'); use(ctx,(1,143,0))
    stand=entities(ctx,'armor_stand')[0]; equip(ctx,'iron_chestplate')
    call(actor(ctx), 'interact_entity', stand['id'],location=(0,.9,0)); check(ctx,'chestplate_transferred',count(ctx,'iron_chestplate')==0)
    equip(ctx); call(actor(ctx), 'interact_entity', stand['id'],location=(0,.9,0))
    check(ctx,'chestplate_returned',count(ctx,'iron_chestplate')==1)
    equip(ctx,'name_tag[custom_name="MatrixStand"]'); call(actor(ctx), 'interact_entity', stand['id'])
    check(ctx,'stand_named','MatrixStand' in data(ctx,'entity @e[type=armor_stand,limit=1]','CustomName'))
    equip(ctx); actor(ctx).attack(stand['id']); actor(ctx).attack(stand['id'])
    check(ctx,'stand_drop_once',drops(ctx,'armor_stand')+count(ctx,'armor_stand')==1)


def item_frame_use(ctx):
    require_modes(ctx,'survival'); setblock(ctx,'stone'); equip(ctx,'item_frame'); use(ctx,POS,'south')
    frame=entities(ctx,'item_frame')[0]; equip(ctx,'diamond'); call(actor(ctx), 'interact_entity', frame['id'])
    check(ctx,'frame_displays_diamond','minecraft:diamond' in data(ctx,'entity @e[type=item_frame,limit=1]','Item'))
    setblock(ctx,'comparator[facing=north]',(1,144,-1))
    setblock(ctx,'redstone_wire',(1,144,-2))
    for rotation in range(8):
        check(ctx,'frame_rotation',scalar(ctx,'entity @e[type=item_frame,limit=1]','ItemRotation')==rotation)
        ticks(ctx,2)
        block(ctx,(1,144,-2),f'redstone_wire[power={rotation+1}]','frame_comparator_rotation:'+str(rotation))
        call(actor(ctx), 'interact_entity', frame['id'])
    actor(ctx).attack(frame['id']); check(ctx,'frame_item_once',count(ctx,'diamond')+drops(ctx,'diamond')==1)


def spawn_egg_on_spawner(ctx):
    require_modes(ctx,'survival','creative'); setblock(ctx,'spawner'); equip(ctx,'pig_spawn_egg',2)
    use(ctx); check(ctx,'spawner_type','minecraft:pig' in data(ctx,'block 1 144 0','SpawnData'))
    use(ctx,(2,143,0)); check(ctx,'ground_pig',len(entities(ctx,'pig'))==1)


def leash_fence(ctx):
    require_modes(ctx,'survival','creative'); setblock(ctx,'oak_fence'); command(ctx,'summon cow 2.5 144 .5 {NoAI:1b}')
    cow=entities(ctx,'cow')[0]; equip(ctx,'lead'); call(actor(ctx), 'interact_entity', cow['id']); use(ctx)
    check(ctx,'one_knot',len(entities(ctx,'leash_knot'))==1)
    check(ctx,'leash_persisted','leash' in data(ctx,'entity @e[type=cow,limit=1]'))


def banner_map_marker(ctx):
    require_modes(ctx,'survival','creative'); setblock(ctx,'red_banner'); equip(ctx,'map')
    call(actor(ctx),'use_item'); use(ctx)
    markers=observed(ctx,'map_markers'); check(ctx,'banner_marker_added',any(m['type']=='banner_red' for m in markers))


def shelf(ctx):
    require_modes(ctx,'survival'); setblock(ctx,'oak_shelf[facing=south]'); equip(ctx,'diamond',3)
    use(ctx,POS,'south',(.15,.5,1.0)); check(ctx,'shelf_debit',count(ctx,'diamond')==0)
    use(ctx,POS,'south',(.15,.5,1.0)); check(ctx,'shelf_roundtrip',count(ctx,'diamond')==3)


def ender_chest_personal(ctx):
    require_modes(ctx,'survival','creative')
    if len(ctx.clients)<2:
        from ..matrix_api import MissingCapability
        raise MissingCapability('client_count:2')
    setblock(ctx,'ender_chest'); a,b=ctx.clients[:2]; equip(ctx,'diamond',3,a); equip(ctx,None,client=b)
    a.open_container(POS); a.click_slot(54); a.click_slot(0); call(a, 'close_container')
    boundary(ctx)
    b.open_container(POS); check(ctx,'other_player_empty',not any(s and s.get('item_id')=='minecraft:diamond' for s in observed(ctx,'menu',b)['slots']))
    call(b, 'close_container'); a.disconnect(); a.reconnect(); a.open_container(POS)
    check(ctx,'owner_reconnect_custody',sum(s['count'] for s in observed(ctx,'menu',a)['slots'] if s['slot']<27 and s['item_id']=='minecraft:diamond')==3)


def cake_candle(ctx):
    require_modes(ctx,'survival'); command(ctx,f'effect give {actor(ctx).name} hunger 20 255 true'); ticks(ctx,40)
    setblock(ctx,'cake'); equip(ctx); before=observed(ctx,'food'); use(ctx)
    block(ctx,POS,'cake[bites=1]'); check(ctx,'cake_food_two',observed(ctx,'food')==min(20,before+2))
    setblock(ctx,'cake'); equip(ctx,'candle'); use(ctx); block(ctx,POS,'candle_cake[lit=false]')
    equip(ctx,'flint_and_steel'); use(ctx); block(ctx,POS,'candle_cake[lit=true]')


def copper_wax_oxidize(ctx):
    require_modes(ctx,'survival','creative'); setblock(ctx,'copper_block'); equip(ctx,'honeycomb'); use(ctx)
    block(ctx,POS,'waxed_copper_block'); command(ctx,'gamerule random_tick_speed 4096'); ticks(ctx,400)
    block(ctx,POS,'waxed_copper_block')
    setblock(ctx,'copper_block')
    ctx.wait_until(lambda:any(block_is(ctx,POS,stage) for stage in ('exposed_copper','weathered_copper','oxidized_copper')),
                   description='unwaxed copper ages under random ticks',timeout_seconds=120)
    check(ctx,'unwaxed_copper_ages',not block_is(ctx,POS,'copper_block'))
    setblock(ctx,'oxidized_copper'); command(ctx,'summon lightning_bolt 1.5 144 .5')
    ticks(ctx,2); block(ctx,POS,'copper_block')


def end_portal_eye(ctx):
    require_modes(ctx,'survival','creative'); equip(ctx,'ender_eye',12)
    frames=[((x,144,-2),'south') for x in (-1,0,1)]+[((x,144,2),'north') for x in (-1,0,1)]+[((-2,144,z),'east') for z in (-1,0,1)]+[((2,144,z),'west') for z in (-1,0,1)]
    for pos,facing in frames: setblock(ctx,f'end_portal_frame[facing={facing},eye=false]',pos)
    for pos,_ in frames:
        command(ctx,f'tp {actor(ctx).name} {pos[0]+.5} 145 {pos[2]+.5}'); use(ctx,pos)
        block(ctx,pos,'end_portal_frame[eye=true]')
    for x in (-1,0,1):
        for z in (-1,0,1): block(ctx,(x,144,z),'end_portal')


def end_crystal_place(ctx):
    require_modes(ctx,'survival','creative')
    for base in ('obsidian','bedrock','stone'):
        command(ctx,'kill @e[type=end_crystal]'); setblock(ctx,base); equip(ctx,'end_crystal',2); use(ctx)
        check(ctx,'crystal_support:'+base,len(entities(ctx,'end_crystal'))==(0 if base=='stone' else 1))


def respawn_anchor(ctx):
    require_modes(ctx,'survival','creative'); setblock(ctx,'respawn_anchor'); equip(ctx,'glowstone',4)
    for charges in range(1,5): use(ctx); block(ctx,POS,f'respawn_anchor[charges={charges}]')
    equip(ctx); use(ctx)
    if ctx.factors['dimension']=='the_nether':
        check(ctx,'anchor_respawn_dimension',data(ctx,f'entity {actor(ctx).name}','respawn.dimension')=='"minecraft:the_nether"')
        for axis,value in enumerate(POS):
            check(ctx,'anchor_respawn_axis:'+str(axis),scalar(ctx,f'entity {actor(ctx).name}',f'respawn.pos[{axis}]')==value)
    else: block(ctx,POS,'air')


def block_log_inspect_click(ctx):
    setblock(ctx,'stone'); actor(ctx).command('co inspect'); equip(ctx)
    before=len(observed(ctx,'events')); use(ctx); block(ctx,POS,'stone')
    fresh=observed(ctx,'events')[before:]
    check(ctx,'history_feedback_for_clicked_block',any('1,144,0' in str(e).replace(' ','') for e in fresh),fresh)


ADAPTERS = {f'block_interact.{fn.__name__}':adapter(fn) for fn in (
    door_trapdoor_gate_toggle,lever_button,note_block_tune,repeater_comparator_daylight,
    jukebox,sign_edit_decorate,vault_key,ignite,axe_strip_scrape_unwax,shovel_path,hoe_till,
    bone_meal,beehive_harvest,cauldron,campfire_cook,composter,armor_stand_use,item_frame_use,
    spawn_egg_on_spawner,leash_fence,banner_map_marker,shelf,ender_chest_personal,cake_candle,
    copper_wax_oxidize,end_portal_eye,end_crystal_place,respawn_anchor,block_log_inspect_click)}
