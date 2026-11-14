"""26.1.2 placement scenarios; sources: https://minecraft.wiki/w/Block_placement
and each block's Usage/Block_states section. No command performs the tested placement.
"""
from ._blocks import (POS, actor, adapter, block, block_is, call, check, command,
                      count, equip, fixture, mine, observed, require_modes, setblock,
                      ticks, use, wait_action_ack)


def slab_double(ctx):
    require_modes(ctx, 'survival', 'creative')
    for half, cursor in [('bottom',(.5,.25,.5)),('top',(.5,.75,.5))]:
        setblock(ctx,'air'); setblock(ctx,'stone',(0,144,0)); equip(ctx,'oak_slab',3)
        use(ctx,(0,144,0),'east',cursor)
        block(ctx,POS,f'oak_slab[type={half}]')
        check(ctx,'first_slab_debit:'+half,count(ctx,'oak_slab') == (3 if ctx.factors['game_mode']=='creative' else 2))
        use(ctx,POS,'up' if half=='bottom' else 'down')
        block(ctx,POS,'oak_slab[type=double]')
        check(ctx,'slab_debit:'+half,count(ctx,'oak_slab') == (3 if ctx.factors['game_mode']=='creative' else 1))


def stairs_shapes(ctx):
    require_modes(ctx,'survival','creative')
    # Facing north: a west-facing stair ahead is outer_left, behind inner_left.
    for neighbour, shape in [((1,144,-1),'outer_left'),((1,144,1),'inner_left')]:
        for half, cursor in [('bottom',(.5,.25,.5)),('top',(.5,.75,.5))]:
            command(ctx,'fill 0 144 -2 3 146 2 air')
            setblock(ctx,f'oak_stairs[facing=west,half={half}]',neighbour)
            setblock(ctx,'stone',(0,144,0))
            equip(ctx,'oak_stairs'); command(ctx,f'tp {actor(ctx).name} 1.5 144 2.5 180 0')
            use(ctx,(0,144,0),'east',cursor)
            block(ctx,POS,f'oak_stairs[facing=north,half={half},shape={shape}]')


def door(ctx):
    require_modes(ctx,'survival','creative')
    for side, hinge in [((0,144,0),'right'),((2,144,0),'left')]:
        command(ctx,'fill 0 144 -1 3 147 1 air'); setblock(ctx,'stone',side)
        setblock(ctx,'stone',(side[0],145,side[2])); equip(ctx,'oak_door')
        command(ctx,f'tp {actor(ctx).name} 1.5 144 2.5 180 0'); use(ctx,(1,143,0))
        block(ctx,POS,f'oak_door[half=lower,facing=north,hinge={hinge}]')
        block(ctx,(1,145,0),f'oak_door[half=upper,facing=north,hinge={hinge}]')


def trapdoor(ctx):
    require_modes(ctx,'survival','creative')
    for half, height in [('bottom',.25),('top',.75)]:
        setblock(ctx,'air'); setblock(ctx,'stone',(0,144,0)); equip(ctx,'oak_trapdoor')
        use(ctx,(0,144,0),'east',(.5,height,.5))
        block(ctx,POS,f'oak_trapdoor[half={half},facing=east]')


def fence_wall_pane_connect(ctx):
    require_modes(ctx,'survival','creative')
    for item, connection in [('oak_fence','true'),('cobblestone_wall','low'),
                             ('glass_pane','true'),('iron_bars','true')]:
        setblock(ctx,item,(0,144,0)); setblock(ctx,'air'); equip(ctx,item)
        use(ctx,(1,143,0)); block(ctx,POS,f'{item}[west={connection}]')
        block(ctx,(0,144,0),f'{item}[east={connection}]')
        setblock(ctx,'stone',(1,145,0))
        if item=='cobblestone_wall':
            block(ctx,POS,'cobblestone_wall[up=true,west=tall]')


def fence_gate(ctx):
    require_modes(ctx,'survival','creative')
    setblock(ctx,'cobblestone_wall',(0,144,0)); setblock(ctx,'cobblestone_wall',(2,144,0))
    equip(ctx,'oak_fence_gate'); command(ctx,f'tp {actor(ctx).name} 1.5 144 2.5 180 0')
    use(ctx,(1,143,0)); block(ctx,POS,'oak_fence_gate[in_wall=true,facing=north]')


def bed(ctx):
    require_modes(ctx,'survival','creative')
    equip(ctx,'red_bed',2); command(ctx,f'tp {actor(ctx).name} 1.5 144 2.5 180 0')
    use(ctx,(1,143,0)); block(ctx,POS,'red_bed[part=foot,facing=north]')
    block(ctx,(1,144,-1),'red_bed[part=head,facing=north]')
    setblock(ctx,'air'); setblock(ctx,'stone',(1,144,-1)); before=count(ctx,'red_bed')
    use(ctx,(1,143,0)); block(ctx,POS,'air'); check(ctx,'blocked_head_keeps_item',count(ctx,'red_bed')==before)
    setblock(ctx,'air',(1,144,-1)); setblock(ctx,'stone'); before=count(ctx,'red_bed')
    use(ctx,(1,143,0)); block(ctx,POS,'stone'); block(ctx,(1,144,-1),'air')
    check(ctx,'blocked_foot_keeps_item',count(ctx,'red_bed')==before)


def attached_switches(ctx):
    require_modes(ctx,'survival','creative')
    for item in ('lever','stone_button','oak_button'):
        for face, anchor, expected in [('up',(1,143,0),'floor'),('down',(1,145,0),'ceiling'),('east',(0,144,0),'wall')]:
            setblock(ctx,'air'); setblock(ctx,'stone',anchor); equip(ctx,item,2)
            use(ctx,anchor,face); block(ctx,POS,f'{item}[face={expected}]')
            setblock(ctx,'air'); setblock(ctx,'oak_slab[type=bottom]',anchor); before=count(ctx,item)
            use(ctx,anchor,face)
            # A lower slab has no sturdy full vertical face.
            if face=='east':
                block(ctx,POS,'air'); check(ctx,'unsupported_switch_keeps_item',count(ctx,item)==before)


def torch_variants(ctx):
    require_modes(ctx,'survival','creative')
    for item, wall in [('torch','wall_torch'),('soul_torch','soul_wall_torch'),('redstone_torch','redstone_wall_torch')]:
        setblock(ctx,'air'); equip(ctx,item,3); use(ctx,(1,143,0)); block(ctx,POS,item)
        setblock(ctx,'air'); setblock(ctx,'stone',(0,144,0)); use(ctx,(0,144,0),'east')
        block(ctx,POS,f'{wall}[facing=east]')
        setblock(ctx,'air'); setblock(ctx,'air',(0,144,0)); before=count(ctx,item)
        use(ctx,(0,144,0),'east'); block(ctx,POS,'air'); check(ctx,'torch_invalid_support',count(ctx,item)==before)


def signs_all_kinds(ctx):
    require_modes(ctx,'survival','creative')
    for item, expected, anchor, face in [
        ('oak_sign','oak_sign[rotation=0]',(1,143,0),'up'),
        ('oak_sign','oak_wall_sign[facing=east]',(0,144,0),'east'),
        ('oak_hanging_sign','oak_hanging_sign[rotation=0]',(1,145,0),'down'),
        ('oak_hanging_sign','oak_wall_hanging_sign[facing=south]',(0,144,0),'east')]:
        setblock(ctx,'air'); setblock(ctx,'stone',anchor); equip(ctx,item)
        command(ctx,f'tp {actor(ctx).name} 1.5 144 2.5 0 0'); use(ctx,anchor,face)
        block(ctx,POS,expected); check(ctx,'sign_editor_open',observed(ctx,'screen')['type']=='sign_editor')
        call(actor(ctx), 'update_sign', POS, ['matrix','','','']); call(actor(ctx), 'close_container')


def banner(ctx):
    require_modes(ctx,'survival','creative')
    for anchor, face, expected in [((1,143,0),'up','white_banner[rotation=0]'),((0,144,0),'east','white_wall_banner[facing=east]')]:
        setblock(ctx,'air'); setblock(ctx,'stone',anchor); equip(ctx,'white_banner')
        command(ctx,f'tp {actor(ctx).name} 1.5 144 2.5 0 0'); use(ctx,anchor,face); block(ctx,POS,expected)


def rail_shapes(ctx):
    require_modes(ctx,'survival','creative')
    for item in ('rail','powered_rail','detector_rail','activator_rail'):
        command(ctx,'fill 0 144 -2 3 146 2 air'); setblock(ctx,item,(1,144,-1))
        setblock(ctx,'stone',(1,144,1)); setblock(ctx,item,(1,145,1)); equip(ctx,item)
        use(ctx,(1,143,0)); block(ctx,POS,f'{item}[shape=ascending_south]')
        if item=='rail':
            setblock(ctx,'air'); setblock(ctx,'air',(1,145,1)); setblock(ctx,'air',(1,144,1))
            setblock(ctx,'rail',(2,144,0)); equip(ctx,'rail'); use(ctx,(1,143,0))
            block(ctx,POS,'rail[shape=north_east]')


def support_required_blocks(ctx):
    require_modes(ctx,'survival','creative')
    for item in ('white_carpet','stone_pressure_plate','dandelion','oak_sapling'):
        setblock(ctx,'air'); setblock(ctx,'air',(1,143,0)); equip(ctx,item,2)
        use(ctx,(1,143,0)); block(ctx,POS,'air'); check(ctx,'invalid_ground:'+item,count(ctx,item)==2)


def directional_blocks(ctx):
    require_modes(ctx,'survival','creative')
    for item, facing in [('observer','north'),('piston','south'),('dispenser','south'),
                         ('repeater','north'),('comparator','north'),('anvil','east'),
                         ('shulker_box','up'),('end_rod','up'),('lightning_rod','up'),('amethyst_cluster','up')]:
        setblock(ctx,'air'); equip(ctx,item); command(ctx,f'tp {actor(ctx).name} 1.5 144 3.5 180 0')
        use(ctx,(1,143,0)); block(ctx,POS,f'{item}[facing={facing}]')


def waterlogged_place(ctx):
    require_modes(ctx,'survival','creative')
    for item in ('oak_slab','oak_stairs','oak_fence'):
        setblock(ctx,'water'); equip(ctx,item); use(ctx,(1,143,0)); block(ctx,POS,f'{item}[waterlogged=true]')
        setblock(ctx,'air',(2,143,0)); ticks(ctx,10); block(ctx,(2,144,0),'water')


def sneak_place_on_interactive(ctx):
    require_modes(ctx,'survival','creative')
    for support in ('chest','crafting_table'):
        setblock(ctx,support); setblock(ctx,'air',(1,145,0)); equip(ctx,'stone',2)
        call(actor(ctx),'sneak',True)
        try:
            use(ctx)
        finally:
            call(actor(ctx),'sneak',False)
        block(ctx,(1,145,0),'stone')
        check(ctx,'no_container_open',observed(ctx,'menu')['id']==0)


def place_into_entity_refused(ctx):
    require_modes(ctx,'survival','creative')
    for kind in ('player','pig'):
        setblock(ctx,'air'); equip(ctx,'stone',2)
        if kind=='player': command(ctx,f'tp {actor(ctx).name} 1.5 144 .5')
        else: command(ctx,'summon pig 1.5 144 .5 {NoAI:1b}')
        use(ctx,(1,143,0)); block(ctx,POS,'air'); check(ctx,'collision_keeps_item:'+kind,count(ctx,'stone')==2)


def out_of_reach_refused(ctx):
    require_modes(ctx,'survival','creative')
    equip(ctx,'stone',2); setblock(ctx,'air',(7,144,0))
    token=call(actor(ctx),'use_item_on',(7,143,0),face='up')
    wait_action_ack(ctx,token)
    block(ctx,(7,144,0),'air'); check(ctx,'out_of_reach_inventory',count(ctx,'stone')==2)
    check(ctx,'out_of_reach_ack',observed(ctx,'block_ack_sequence')>=token)
    mine(ctx,(7,143,0)); block(ctx,(7,143,0),'stone')
    setblock(ctx,'stone',(1,145,1)); token=call(actor(ctx),'use_item_on',(1,143,0),face='up')
    wait_action_ack(ctx,token)
    check(ctx,'occluded_ack',observed(ctx,'block_ack_sequence')>=token)
    # Vanilla reach validation is distance-based; do not invent server raycast rejection.


def build_height_limits(ctx):
    require_modes(ctx,'survival','creative')
    low,high=(-64,319) if ctx.factors['dimension']=='overworld' else (0,255)
    for y,face in [(high,'up'),(low,'down')]:
        command(ctx,f'tp {actor(ctx).name} 1.5 {y} 2.5'); setblock(ctx,'stone',(1,y,0)); equip(ctx,'stone',2)
        use(ctx,(1,y,0),face); check(ctx,'height_item_retained',count(ctx,'stone')==2)
        check(ctx,'height_refusal_visible',bool(observed(ctx,'action_bar')))


def replaceable_targets(ctx):
    require_modes(ctx,'survival','creative')
    for target in ('short_grass','snow[layers=1]','water','lava'):
        setblock(ctx,'dirt',(1,143,0)); setblock(ctx,target); equip(ctx,'stone',2)
        use(ctx,POS,'up'); block(ctx,POS,'stone')


def rejected_resync(ctx):
    place_into_entity_refused(ctx)
    check(ctx,'predicted_state_authoritatively_removed',
          not observed(ctx,'predicted_blocks'))
    check(ctx,'received_ack',observed(ctx,'block_ack_sequence') is not None)


def adventure_can_place_on(ctx):
    require_modes(ctx,'adventure')
    for floor, expected in [('stone','cobblestone'),('dirt','air')]:
        setblock(ctx,floor,(1,143,0)); setblock(ctx,'air')
        equip(ctx,'cobblestone[can_place_on={predicates:[{blocks:"minecraft:stone"}]}]',2)
        use(ctx,(1,143,0)); block(ctx,POS,expected)
        check(ctx,'adventure_predicate_debit',count(ctx,'cobblestone')==(1 if floor=='stone' else 2))


def spawn_protection_radius(ctx):
    require_modes(ctx,'survival','creative')
    call(ctx.server,'configure_spawn_protection',16)
    command(ctx,'setworldspawn 0 144 0'); equip(ctx,'stone',2); use(ctx,(1,143,0))
    nonop=ctx.factors['permission']=='non_op_0' and ctx.factors['dimension']=='overworld'
    block(ctx,POS,'air' if nonop else 'stone')
    mine(ctx,(0,143,0)); block(ctx,(0,143,0),'stone' if nonop else 'air')


ADAPTERS = {f'block_place.{fn.__name__}': adapter(fn) for fn in (
    slab_double,stairs_shapes,door,trapdoor,fence_wall_pane_connect,
    fence_gate,bed,attached_switches,torch_variants,signs_all_kinds,banner,rail_shapes,
    support_required_blocks,directional_blocks,waterlogged_place,sneak_place_on_interactive,
    place_into_entity_refused,out_of_reach_refused,build_height_limits,replaceable_targets,
    rejected_resync,adventure_can_place_on,spawn_protection_radius)}
