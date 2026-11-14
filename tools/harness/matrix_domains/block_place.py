"""Six public block placements with exact finite-stock and world oracles."""
from ..matrix_api import Adapter, Check, MatrixContext, MissingCapability

FACES = [('down',(7,80,8)),('up',(7,82,8)),('north',(7,81,7)),
         ('south',(7,81,9)),('west',(6,81,8)),('east',(8,81,8))]


def _stone_count(client):
    inventory = client.snapshot().get('inventory')
    if inventory is None:
        raise MissingCapability('snapshot:inventory')
    return sum(stack['count'] for stack in inventory if stack['item_id']=='minecraft:stone')


def setup(ctx: MatrixContext) -> None:
    if ctx.factors['game_mode'] != 'survival':
        raise MissingCapability('placement_negative_or_creative_received_ack')
    actor = ctx.clients[0]
    for client in ctx.clients:
        client.teleport_to(4.5,80,8.5)
    ctx.server.ensure_loaded(((2,79,4),(12,84,12)))
    ctx.server.command('fill 2 79 4 12 79 12 stone')
    ctx.server.command('fill 2 80 4 12 84 12 air')
    ctx.server.command('setblock 7 81 8 stone')
    fixture = ctx.server.query('execute if block 7 81 8 stone run say MatrixPlacementFixtureReady')
    if 'MatrixPlacementFixtureReady' not in fixture:
        raise RuntimeError('placement fixture was not committed: ' + fixture)
    ctx.server.command(f'clear {actor.name}')
    ctx.server.command(f'give {actor.name} stone 16')
    ctx.wait_until(lambda: _stone_count(actor)==16,description='received finite 16-stone fixture')
    hotbar = [stack for stack in actor.snapshot()['inventory']
              if stack['item_id']=='minecraft:stone' and 36<=stack['slot']<=44]
    if not hotbar:
        raise MissingCapability('fixture_hotbar_item:stone')
    actor.select_hotbar(hotbar[0]['slot']-36)
    ctx.state['before_stone']=_stone_count(actor)
    ctx.state['placed']=[]


def act(ctx: MatrixContext) -> None:
    actor=ctx.clients[0]
    for index,(face,target) in enumerate(FACES):
        actor.look_at(7.5,81.5,8.5)
        actor.use_item_on((7,81,8),face=face)
        ctx.wait_until(lambda: _stone_count(actor)==15-index,
                       description=f'received exact item debit for {face} placement')
        ctx.state['placed'].append({'face':face,'position':target,'remaining':_stone_count(actor)})
        if index==0:
            ctx.boundary()


def oracle(ctx: MatrixContext) -> list[Check]:
    checks=[]
    for face,target in FACES:
        marker='MatrixFullBlockOracle_'+face
        x,y,z=target
        reply=ctx.server.query(f'execute if block {x} {y} {z} stone run say {marker}')
        checks.append(Check('authoritative_placement_'+face,marker in reply,
                            {'position':target,'reply':reply}))
    count=_stone_count(ctx.clients[0])
    checks.append(Check('received_six_exact_item_debits',count==10,
                        {'before':ctx.state['before_stone'],'after':count,'world_credit':6}))
    return checks


ADAPTERS={'block_place.full_block':Adapter(setup,act,oracle)}

from .block_place_extra import ADAPTERS as _EXTRA_ADAPTERS

ADAPTERS.update(_EXTRA_ADAPTERS)
