"""Native settlement commands, never mislabeled vanilla Minecraft behaviour.

Oracles are the independent /settlement info/list/status read surfaces from
mc-core command_execution.rs:1300-1335 and settlements/economy_commands.rs:84-104.
"""
import re
from ..matrix_api import Adapter, Check, MissingCapability
from ._mob_support import arena, exists, field, ticks

RESIDENT='12345678-1234-1234-1234-123456789c01'


def query(ctx, command):
    reply=ctx.server.query(command)
    if 'Unknown or incomplete' in reply:
        raise MissingCapability('settlement_command')
    # Console transport appends a different fence per query. It is not world
    # state and must not participate in pre/post-boundary equality.
    return '\n'.join(line for line in reply.splitlines()
                     if not re.fullmatch(r'chat\.type\.announcement: Server, MatrixConsoleFence\d+',line))


def command(ctx, text, expected, *, client=None):
    if client is None and ctx.factors['permission']=='console':
        reply=ctx.server.command(text)
        if not re.search(expected,reply):
            raise AssertionError({'command':text,'expected':expected,'reply':reply})
        return reply
    actor=client or ctx.clients[0]
    before=len(field(actor,'events'))
    actor.command(text)
    def received():
        events=field(actor,'events')[before:]
        matching=[event for event in events if re.search(expected,str(event))]
        return matching[-1] if matching else None
    return ctx.wait_until(received,description='received settlement command result: '+text)


def setup(ctx):
    arena(ctx)
    ctx.server.command('gamerule doMobSpawning false')
    subject=ctx.row['subject'].split('.',1)[1]
    if subject in {'production','commerce','payroll','caravan'}:
        # Family subjects require the operation cycle, not a zero-count status.
        raise MissingCapability('settlement_economy_snapshot')
    if subject in {'rename','remove','bind','unbind','create'} and len(ctx.clients)<2:
        raise MissingCapability('multi_client_2:nonowner_authorization')
    command(ctx,'settlement create MatrixTown 48',r'Founded MatrixTown')
    reply=query(ctx,'settlement info MatrixTown')
    if not re.search(r'MatrixTown \| owner=.+ radius=48 members=0 revision=\d+',reply):
        raise AssertionError('created settlement is not observable independently: '+reply)
    if subject in {'bind','unbind'}:
        ctx.server.command(f'summon villager 3 200 0 {{UUID:"{RESIDENT}"}}')
        if not exists(ctx,RESIDENT):
            raise AssertionError('resident fixture missing')
    if subject=='unbind':
        command(ctx,'settlement bind MatrixTown '+RESIDENT,r'Bound .* to MatrixTown')
        if 'members=1 ' not in query(ctx,'settlement info MatrixTown'):
            raise AssertionError('unbind fixture has no membership')
    ctx.state['before']=query(ctx,'settlement info MatrixTown')


OPERATIONS={
 'create':('settlement create MatrixNew 24',r'Founded MatrixNew','settlement info MatrixNew',r'MatrixNew \| owner=.+ radius=24 members=0 revision=\d+'),
 'list':('settlement list',r'Settlements: .*MatrixTown','settlement list',r'Settlements: .*MatrixTown'),
 'info':('settlement info MatrixTown',r'MatrixTown \| owner=','settlement info MatrixTown',r'MatrixTown \| owner=.+ radius=48 members=0 revision=\d+'),
 'rename':('settlement rename MatrixTown MatrixRenamed',r'Renamed MatrixTown to MatrixRenamed','settlement info MatrixRenamed',r'MatrixRenamed \| owner=.+ radius=48 members=0 revision=\d+'),
 'remove':('settlement remove MatrixTown',r'Removed MatrixTown','settlement info MatrixTown',r'Settlement not found\.'),
 'bind':('settlement bind MatrixTown '+RESIDENT,r'Bound .* to MatrixTown','settlement info MatrixTown',r'members=1 revision='),
 'unbind':('settlement unbind '+RESIDENT,r'Unbound','settlement info MatrixTown',r'members=0 revision='),
 'production':('settlement production MatrixTown status',r'Production: jobs=0 completed=0 cancelled=0','settlement production MatrixTown status',r'Production: jobs=0 completed=0 cancelled=0'),
 'commerce':('settlement commerce MatrixTown status',r'Commerce: quotes=0 mandates=0','settlement commerce MatrixTown status',r'Commerce: quotes=0 mandates=0'),
 'payroll':('settlement payroll MatrixTown status',r'Payroll: employments=0','settlement payroll MatrixTown status',r'Payroll: employments=0'),
 'caravan':('settlement caravan MatrixTown status',r'Caravan: shipments=0','settlement caravan MatrixTown status',r'Caravan: shipments=0'),
}


def act(ctx):
    subject=ctx.row['subject'].split('.',1)[1]
    text,feedback,read,expected=OPERATIONS[subject]
    if subject in {'create','rename','remove','bind','unbind'}:
        other=ctx.clients[1]
        # Context applies op levels to all actors. An operator is legitimately
        # permitted; a non-owner denial cannot be tested with that identity.
        if ctx.factors['permission'] not in {'non_op_0','console'}:
            raise MissingCapability('nonop_observer_identity')
        denial='settlement remove MatrixTown' if subject=='create' else text
        denial_action={'create':'removes','rename':'renames','remove':'removes',
                       'bind':'binds','unbind':'unbinds'}[subject]
        before=query(ctx,'settlement info MatrixTown')
        command(ctx,denial,'Only the owner or an operator '+denial_action,client=other)
        after=query(ctx,'settlement info MatrixTown')
        ctx.state['unauthorized_unchanged']=before==after
    ctx.state['action_reply']=command(ctx,text,feedback)
    current=query(ctx,read)
    if not re.search(expected,current):
        raise AssertionError({'query':read,'expected':expected,'actual':current})
    ctx.state['after_action']=current
    ctx.state['observer_replies']=[command(ctx,read,expected,client=observer)
                                   for observer in ctx.clients[1:]]
    ctx.boundary()
    # Snapshot at the semantic boundary survives any driver-owned lifecycle.
    ctx.state['after_boundary']=query(ctx,read)
    ctx.state['read']=read
    ctx.state['pattern']=expected
    for client in ctx.clients:
        client.disconnect()
    ctx.server.save_all()
    ctx.server.clean_restart()
    for client in ctx.clients:
        client.reconnect()
    ctx.state['after_restart']=query(ctx,read)
    ctx.state['observer_restart_replies']=[command(ctx,read,expected,client=observer)
                                           for observer in ctx.clients[1:]]


def oracle(ctx):
    final=query(ctx,ctx.state['read'])
    checks=[Check('action_feedback_received',bool(ctx.state['action_reply']),ctx.state['action_reply']),
            Check('authoritative_state',bool(re.search(ctx.state['pattern'],final)),final),
            Check('boundary_state_preserved',ctx.state['after_action']==ctx.state['after_boundary'],
                  {'before':ctx.state['after_action'],'after':ctx.state['after_boundary']}),
            Check('explicit_restart_state_preserved',ctx.state['after_action']==ctx.state['after_restart'],
                  {'before':ctx.state['after_action'],'after':ctx.state['after_restart']})]
    if 'unauthorized_unchanged' in ctx.state:
        checks.append(Check('nonowner_mutation_refused_without_state_change',ctx.state['unauthorized_unchanged']))
    if ctx.row['subject'] in {'settlements.bind','settlements.unbind'}:
        checks.append(Check('resident_identity_survives_restart',exists(ctx,RESIDENT),RESIDENT))
    checks.append(Check('observers_receive_authoritative_result',
                        len(ctx.state['observer_replies'])==len(ctx.clients)-1 and
                        len(ctx.state['observer_restart_replies'])==len(ctx.clients)-1 and
                        all(re.search(ctx.state['pattern'],reply) for reply in
                            ctx.state['observer_replies']+ctx.state['observer_restart_replies'])))
    return checks


ADAPTERS={'settlements.'+name:Adapter(setup,act,oracle) for name in OPERATIONS}
