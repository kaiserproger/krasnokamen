"""Native /co extension, not vanilla. Source mc-core/src/block_log.rs:878-1019,
1290-1385. Exact pre-image custody, not only an operator feedback line.
"""
from tools.harness.matrix_api import Adapter
from . import _platform as p


def grief(ctx, chest=False):
    x,y,z = p.arena(ctx)
    pos = (x+2,y,z)
    p.admin(ctx, f'setblock {x+2} {y} {z} minecraft:stone')
    p.rich_action(ctx, 'break_block', pos)
    p.block_is(ctx, pos, 'minecraft:air')
    ctx.state['pos'] = pos
    if chest:
        c = (x+2,y,z+1)
        p.admin(ctx, f'setblock {c[0]} {c[1]} {c[2]} minecraft:chest')
        depositor = p.client(ctx, 1)
        p.admin(ctx, f'give {depositor.name} minecraft:diamond 3')
        depositor.teleport_to(x, y, z+1)
        depositor.open_container(c)
        slots = p.observe(ctx, 'menu.slots', 1)
        source = next(s['slot'] for s in slots if s.get('item_id') == 'minecraft:diamond')
        depositor.click_slot(source)
        depositor.click_slot(0)
        ctx.wait_until(lambda: any(s['slot']==0 and s['item_id']=='minecraft:diamond' and s['count']==3
                                  for s in p.observe(ctx,'menu.slots',1)), description='deposit committed')
        p.rich_action(ctx, 'close_container', index=1)
        p.client(ctx).open_container(c)
        p.client(ctx).click_slot(0, kind='quick_move')
        ctx.wait_until(lambda: not any(s['slot']==0 for s in p.observe(ctx,'menu.slots')),
                       description='removal committed')
        p.rich_action(ctx, 'close_container')
        ctx.state['chest'] = c


def co_inspect(ctx):
    grief(ctx)
    p.response(ctx, 'co inspect', r'(?i)inspect.*(enabled|on)')
    p.use(ctx, ctx.state['pos'])
    ctx.wait_until(lambda: any('broke stone' in e for e in p.observe(ctx,'events')),
                   description='inspection history shows actor stone break')
    p.test(ctx, 'inspection history', any('broke stone' in e for e in p.observe(ctx,'events')), p.observe(ctx,'events'))
    p.block_is(ctx, ctx.state['pos'], 'minecraft:air')


def co_lookup(ctx):
    grief(ctx)
    p.response(ctx, f'co lookup u:{p.client(ctx).name} t:1h r:8', r'Block log: [1-9][0-9]* result')
    lines = p.observe(ctx,'events')
    p.test(ctx, 'lookup attributed actor and change', any(p.client(ctx).name in e and 'broke stone' in e for e in lines), lines)
    p.response(ctx, 'co lookup u:matrix_absent_actor t:1h r:8', r'Block log: 0 result')


def co_rollback(ctx):
    grief(ctx, chest=True)
    # A different actor deposited the stack, so u: selects only the removal.
    p.response(ctx, f'co rollback u:{p.client(ctx).name} t:1h r:8',
               r'Rollback complete: [1-9][0-9]* change')
    c = ctx.state['chest']
    p.client(ctx).open_container(c)
    slot = next(s for s in p.observe(ctx, 'menu.slots') if s['slot']==0)
    p.check(ctx, 'chest pre-image restored', {k: slot[k] for k in ('item_id', 'count')},
            {'item_id': 'minecraft:diamond', 'count': 3})
    p.rich_action(ctx, 'close_container')
    p.block_is(ctx,ctx.state['pos'],'minecraft:stone')


def co_restore(ctx):
    grief(ctx)
    filter_text=f'u:{p.client(ctx).name} t:1h r:8'
    p.response(ctx,'co rollback '+filter_text,r'Rollback complete: 1 change')
    p.block_is(ctx,ctx.state['pos'],'minecraft:stone')
    p.response(ctx,'co restore '+filter_text,r'Restore complete: 1 change')
    p.block_is(ctx,ctx.state['pos'],'minecraft:air')


def co_history_restart(ctx):
    grief(ctx)
    p.restart(ctx)
    p.response(ctx,f'co lookup u:{p.client(ctx).name} t:1h r:8',r'Block log: [1-9][0-9]* result')
    lines=p.observe(ctx,'events')
    p.test(ctx,'history persists',any(p.client(ctx).name in e and 'broke stone' in e for e in lines),lines)
    p.response(ctx,f'co rollback u:{p.client(ctx).name} t:1h r:8',r'Rollback complete: 1 change')
    p.block_is(ctx,ctx.state['pos'],'minecraft:stone')


ADAPTERS={'block_log.'+fn.__name__:Adapter(p.setup,fn,p.oracle) for fn in (
    co_inspect,co_lookup,co_rollback,co_restore,co_history_restart)}
