"""Solaris monitor extension, source scenarios/live-plugins/computer/README.md:3-19.
Vanilla clients see the carrier; this is not a vanilla custom-block feature.
"""
from tools.harness.matrix_api import Adapter
from . import _platform as p
from .plugins import deploy


def monitor_give_place_break(ctx):
    deploy(ctx,'computer')
    x,y,z=p.arena(ctx)
    p.admin(ctx,f'clear {p.client(ctx).name}')
    p.admin(ctx,f'give {p.client(ctx).name} solaris:monitor 65')
    ctx.wait_until(lambda:p.inventory(ctx,'solaris:monitor')==65,
                   description='received monitor stack-limit fixture')
    stacks=sorted(s['count'] for s in p.observe(ctx,'inventory') if s['item_id']=='solaris:monitor')
    p.admin(ctx,f'clear {p.client(ctx).name}')
    p.admin(ctx,f'give {p.client(ctx).name} solaris:monitor 2')
    p.rich_action(ctx,'select_inventory_item','solaris:monitor')
    p.client(ctx).teleport_to(x+.5,y,z-2.5)
    p.use(ctx,(x,y-1,z),'up')
    p.block_is(ctx,(x,y,z),'minecraft:black_glazed_terracotta',facing='north')
    p.check(ctx,'monitor stack limit is 64',stacks,[1,64])
    p.check(ctx,'monitor debit follows selected mode',p.inventory(ctx,'solaris:monitor'),
            2 if ctx.factors['game_mode']=='creative' else 1)
    p.rich_action(ctx,'break_block',(x,y,z))
    p.block_is(ctx,(x,y,z),'minecraft:air')
    p.client(ctx).teleport_to(x+.5,y,z+.5)
    ctx.wait_until(lambda:p.inventory(ctx,'solaris:monitor')==2,description='one monitor drop collected')
    p.check(ctx,'exact custom item custody',p.inventory(ctx,'solaris:monitor'),2)
    p.check(ctx,'no carrier duplication',p.inventory(ctx,'minecraft:black_glazed_terracotta'),0)


def monitor_native_bundle_render(ctx):
    deploy(ctx,'computer')
    x,y,z=p.arena(ctx)
    p.admin(ctx,f'setblock {x+2} {y} {z} minecraft:black_glazed_terracotta[facing=west]')
    p.admin(ctx,f'give {p.client(ctx).name} solaris:monitor 1')
    p.test(ctx,'computer bundle bound','solaris' in p.observe(ctx,'loader.active_packages'),p.observe(ctx,'loader.active_packages'))
    p.check(ctx,'monitor display name',p.rich_action(ctx,'item_display_name','solaris:monitor'),'Monitor')
    p.check(ctx,'monitor model',p.rich_action(ctx,'block_model',(x+2,y,z)),'solaris:monitor')
    p.check(ctx,'screen faces west',p.rich_action(ctx,'monitor_screen_face',(x+2,y,z)),'west')


def monitor_other_clients_fallback(ctx):
    deploy(ctx,'computer')
    x,y,z=p.arena(ctx)
    p.admin(ctx,f'setblock {x+2} {y} {z} minecraft:black_glazed_terracotta[facing=west]')
    p.check(ctx,'optional bundle leaves play available',p.observe(ctx,'connection.phase'),'play')
    p.check(ctx,'not bound to native bundle','solaris' in p.observe(ctx,'loader.active_packages'),False)
    p.check(ctx,'fallback model',p.rich_action(ctx,'block_model',(x+2,y,z)),'minecraft:black_glazed_terracotta')
    p.rich_action(ctx,'break_block',(x+2,y,z))
    p.block_is(ctx,(x+2,y,z),'minecraft:air')
    p.check(ctx,'fallback break keeps play',p.observe(ctx,'connection.phase'),'play')


def monitor_plain_terracotta_not_placeable(ctx):
    deploy(ctx,'computer')
    x,y,z=p.arena(ctx)
    for index in range(1, len(ctx.clients)):
        p.client(ctx, index).teleport_to(x, y, z+2)
        p.rich_action(ctx, 'close_container', index=index)
    # Delta-only observers need a received preimage, not an assumed air block.
    for name in ('minecraft:stone', 'minecraft:air'):
        p.admin(ctx, f'setblock {x+2} {y} {z} {name}')
        for index in range(len(ctx.clients)):
            p.wait_block(ctx, (x+2,y,z), name, index=index)
    p.admin(ctx,f'give {p.client(ctx).name} minecraft:black_glazed_terracotta 2')
    p.rich_action(ctx,'select_inventory_item','minecraft:black_glazed_terracotta')
    before=p.inventory(ctx,'minecraft:black_glazed_terracotta')
    p.block_ack(ctx, p.use(ctx,(x+2,y-1,z),'up'))
    p.ticks(ctx,2)
    p.block_is(ctx,(x+2,y,z),'minecraft:air')
    p.check(ctx,'reserved carrier keeps item',p.inventory(ctx,'minecraft:black_glazed_terracotta'),before)
    p.check(ctx,'received authoritative state remains air',p.block(ctx,(x+2,y,z))['name'],'minecraft:air')
    for index in range(1, len(ctx.clients)):
        p.check(ctx, f'observer {index} received unchanged air',
                p.block(ctx, (x+2,y,z), index=index)['name'], 'minecraft:air')


ADAPTERS={'monitor.'+fn.__name__:Adapter(p.setup,fn,p.oracle) for fn in (
    monitor_give_place_break,monitor_native_bundle_render,monitor_other_clients_fallback,
    monitor_plain_terracotta_not_placeable)}
