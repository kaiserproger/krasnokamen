"""Krasnokamen plugin contracts, not vanilla 26.1.2 features.
Sources: krasnokamen-default-plugins/<package>/README.md and sources/<package>/src/lib.rs;
mc-core/src/plugin.rs:580-595; mc-edge/src/plugin_loader.rs:82-83.
Each deployment is an isolated copy of scenarios/live-plugins, never the shared directory.
"""
import json
from pathlib import Path
import re
import shutil
import tomllib
from tools.harness.matrix_api import Adapter, MissingCapability
from . import _platform as p


def deploy(ctx,*packages,**config):
    reconfigure = p.method(ctx.server, 'reconfigure')
    configured = getattr(ctx.server, 'plugins_dir', None)
    if configured is None:
        raise MissingCapability('fixture:plugin_packages')
    source = Path(configured)
    serial = ctx.state.get('plugin_fixture_serial', 0) + 1
    ctx.state['plugin_fixture_serial'] = serial
    target = ctx.artifact_dir / f'platform-plugins-{serial}'
    overrides = dict(config)
    auth = overrides.pop('require_plugin_login', None)
    used = set()
    for package in packages:
        if not (source/package/'plugin.wasm').is_file():
            raise MissingCapability('fixture:plugin_package:' + package)
    # Native startup contributions are checkpointed: preserve the exact
    # effective package set rather than replacing it in an existing world.
    shutil.copytree(source, target)
    for package in packages:
        path = target/package/'config.toml'
        if not path.exists():
            continue
        text = path.read_text()
        parsed = tomllib.loads(text)
        for key, value in overrides.items():
            if key not in parsed:
                continue
            used.add(key)
            if key == 'groups':
                if package != 'krasnokamen-permissions':
                    raise ValueError('groups override belongs to krasnokamen-permissions')
                text = text.split('[[groups]]', 1)[0].rstrip() + '\n'
                for group, nodes in value.items():
                    text += '\n[[groups]]\nname = '+json.dumps(group)+'\nnodes = '+json.dumps(nodes)+'\n'
            else:
                text, changed = re.subn(r'(?m)^'+re.escape(key)+r'\s*=.*$',
                                       key+' = '+json.dumps(value), text)
                if changed != 1:
                    raise ValueError('ambiguous package setting: '+key)
        path.write_text(text)
    if set(overrides) != used:
        raise ValueError('unknown plugin settings: '+', '.join(sorted(set(overrides)-used)))
    for actor in ctx.clients:
        actor.disconnect()
    core = {} if auth is None else {'auth': {'require_plugin_login': auth}}
    reconfigure(core, plugins_dir=target)
    for actor in ctx.clients:
        if auth:
            p.method(actor, 'set_auto_auth', 'client')(False)
        actor.reconnect()
    return target


def perm_groups_user_set(ctx):
    deploy(ctx,'krasnokamen-permissions',groups={'guest':[], 'member':['krasnokamen.essentials.use']},default_group='guest')
    p.response(ctx,'perm groups',r'Groups:.*guest.*member')
    p.response(ctx,'perm user me set member',r'Group set to member\.')
    p.response(ctx,'perm user me list',r'is member\.')
    p.restart(ctx)
    p.response(ctx,'perm user me list',r'is member\.')
    p.response(ctx,'perm check krasnokamen.essentials.use',r'Permission granted\.')


def perm_check_node(ctx):
    deploy(ctx,'krasnokamen-permissions','krasnokamen-essentials',groups={'guest':[], 'member':['krasnokamen.essentials.use','matrix.build@plot']},default_group='guest')
    p.response(ctx,'perm check krasnokamen.essentials.use',r'Permission denied\.')
    before=p.position(ctx)
    p.response(ctx,'sethome matrix',r'(?i)permission|unknown')
    p.check(ctx,'denial leaves pose',p.position(ctx),before)
    p.rich_action(ctx,'operator_plugin_command',f'perm user {p.observe(ctx,"uuid")} set member')
    p.response(ctx,'perm check krasnokamen.essentials.use',r'Permission granted\.')
    p.response(ctx,'perm check matrix.build plot',r'Permission granted\.')
    p.response(ctx,'perm check matrix.build other',r'Permission denied\.')
    p.response(ctx,'sethome matrix',r'matrix saved\.')


def essentials_homes(ctx):
    deploy(ctx,'krasnokamen-essentials')
    home=p.position(ctx)
    p.response(ctx,'sethome matrix',r'matrix saved\.')
    other = p.client(ctx, 1)
    p.response(ctx, 'home matrix', r'Location not found\.', index=1)
    other_home = [home[0]+8, home[1], home[2]]
    other.teleport_to(*other_home)
    p.response(ctx, 'sethome matrix', r'matrix saved\.', index=1)
    p.client(ctx).teleport_to(home[0]+5,home[1],home[2])
    p.client(ctx).command('home matrix')
    p.at(ctx,home)
    p.response(ctx,'delhome matrix',r'matrix removed\.')
    p.response(ctx,'home matrix',r'Location not found\.')
    other.teleport_to(home[0]+3, home[1], home[2])
    other.command('home matrix')
    p.at(ctx, other_home, index=1)


def essentials_spawn_warps(ctx):
    deploy(ctx,'krasnokamen-essentials','krasnokamen-permissions',
           groups={'guest':[], 'member':['krasnokamen.essentials.use','krasnokamen.essentials.manage']},
           default_group='member')
    home=p.position(ctx)
    p.response(ctx,'setspawn',r'saved\.')
    p.response(ctx,'setwarp matrix',r'matrix saved\.')
    for cmd in ('spawn','warp matrix'):
        p.client(ctx).teleport_to(home[0]+5,home[1],home[2])
        p.client(ctx).command(cmd)
        p.at(ctx,home)
    p.response(ctx,'delwarp matrix',r'matrix removed\.')
    p.response(ctx,'warp matrix',r'Location not found\.')
    p.rich_action(ctx,'operator_plugin_command',f'perm user {p.observe(ctx,"uuid")} set guest')
    p.response(ctx,'setwarp denied',r'(?i)permission|operator|unknown')


def essentials_back(ctx):
    deploy(ctx,'krasnokamen-essentials')
    home=p.position(ctx)
    p.response(ctx,'sethome matrix',r'matrix saved\.')
    away=[home[0]+5,home[1],home[2]]
    p.client(ctx).teleport_to(*away)
    p.client(ctx).command('home matrix')
    p.at(ctx,home)
    p.client(ctx).command('back')
    p.at(ctx,away)
    p.admin(ctx,f'kill {p.client(ctx).name}')
    p.rich_action(ctx,'respawn')
    p.client(ctx).command('back')
    p.at(ctx,away)


def essentials_tpa(ctx):
    deploy(ctx,'krasnokamen-essentials')
    other=p.client(ctx,1)
    start=p.position(ctx)
    other.teleport_to(start[0]+8,start[1],start[2])
    target=p.position(ctx,1)
    p.client(ctx).command('tpa '+other.name)
    p.ticks(ctx,2)
    p.check(ctx,'request does not teleport early',p.position(ctx),start)
    other.command('tpaccept')
    p.at(ctx,target)


def essentials_msg_reply(ctx):
    deploy(ctx,'krasnokamen-essentials')
    other=p.client(ctx,1)
    before=len(p.observe(ctx,'events',1))
    p.client(ctx).command('msg '+other.name+' matrix-private-26520')
    ctx.wait_until(lambda:any('matrix-private-26520' in s for s in p.observe(ctx,'events',1)[before:]),description='private message recipient')
    other.command('reply matrix-reply-26520')
    ctx.wait_until(lambda:any('matrix-reply-26520' in s for s in p.observe(ctx,'events')),description='reply recipient')
    p.test(ctx,'plugin reply routing',any('[reply from '+other.name+'] matrix-reply-26520' in s for s in p.observe(ctx,'events')),p.observe(ctx,'events'))
    if len(ctx.clients)>2:
        p.check(ctx,'private not broadcast',any('matrix-private-26520' in s or 'matrix-reply-26520' in s for s in p.observe(ctx,'events',2)),False)


def economy_money_pay(ctx):
    deploy(ctx,'krasnokamen-economy',starting_balance=100)
    target=p.observe(ctx,'uuid',1)
    p.response(ctx,'money',r'Balance: 100 ')
    p.response(ctx,f'pay {target} 7 matrix26520',r'Paid 7 ')
    p.response(ctx,'money',r'Balance: 93 ')
    p.response(ctx,'money',r'Balance: 107 ',index=1)
    p.response(ctx,f'pay {target} 7 matrix26520',r'token was already committed')
    p.response(ctx,'money',r'Balance: 93 ')
    p.response(ctx,'money',r'Balance: 107 ',index=1)
    p.restart(ctx)
    p.response(ctx,f'pay {target} 7 matrix26520',r'token was already committed')
    p.response(ctx,'money',r'Balance: 93 ')
    p.response(ctx,'money',r'Balance: 107 ',index=1)


def economy_admin(ctx):
    deploy(ctx,'krasnokamen-economy',starting_balance=0)
    uuid=p.observe(ctx,'uuid')
    p.client(ctx).command(f'econadmin set {uuid} 40')
    p.response(ctx,'money',r'Balance: 40 ')
    p.client(ctx).command(f'econadmin add {uuid} 2')
    p.response(ctx,'money',r'Balance: 42 ')
    p.rich_action(ctx,'plugin_command_as_nonop',f'econadmin set {uuid} 100',index=1)
    p.response(ctx,'money',r'Balance: 42 ')


def towns_create_info(ctx):
    deploy(ctx,'krasnokamen-towns')
    p.response(ctx,'town create matrix',r'Town matrix created\.')
    p.response(ctx, 'town info',
               r'matrix: 1 members, 0 claims\. 0 trusted guests, doors members-only\.')
    p.response(ctx, 'town claim', r'Chunk claimed for matrix\.')
    p.response(ctx, 'town info',
               r'matrix: 1 members, 1 claims\. 0 trusted guests, doors members-only\.')
    p.response(ctx, 'town create matrix_second', r'Cannot create that town\.')


def towns_protection(ctx):
    deploy(ctx,'krasnokamen-towns')
    x,y,z=p.arena(ctx)
    p.response(ctx,'town create matrix',r'Town matrix created\.')
    other=p.client(ctx,1)
    other.teleport_to(x,y,z+2)
    p.admin(ctx,f'give {other.name} minecraft:stone 4')
    p.rich_action(ctx,'select_inventory_item','minecraft:stone',index=1)
    before=p.inventory(ctx,'minecraft:stone',1)
    p.use(ctx,(x+1,y-1,z+2),index=1)
    p.block_is(ctx,(x+1,y,z+2),'minecraft:air')
    p.check(ctx,'denied place keeps items',p.inventory(ctx,'minecraft:stone',1),before)
    p.admin(ctx,f'setblock {x+2} {y} {z+2} minecraft:chest')
    other.open_container((x+2,y,z+2))
    p.check(ctx,'guest container refused',p.observe(ctx,'screen.kind',1),'game')
    p.admin(ctx,f'give {other.name} minecraft:flint_and_steel 1')
    p.rich_action(ctx,'select_inventory_item','minecraft:flint_and_steel',index=1)
    p.use(ctx,(x+1,y-1,z+3),index=1)
    p.block_is(ctx,(x+1,y,z+3),'minecraft:air')
    p.admin(ctx,f'summon minecraft:pig {x+3} {y} {z+2}')
    pig=p.rich_action(ctx,'find_entity','minecraft:pig',index=1)
    health=p.rich_action(ctx,'entity_state',pig)['health']
    other.attack(pig)
    p.check(ctx,'guest animal damage denied',p.rich_action(ctx,'entity_state',pig)['health'],health)
    p.response(ctx,'town trust '+other.name,r'(?i)trust')
    p.rich_action(ctx,'select_inventory_item','minecraft:stone',index=1)
    p.use(ctx,(x+1,y-1,z+2),index=1)
    p.block_is(ctx,(x+1,y,z+2),'minecraft:stone')
    p.response(ctx,'town untrust '+other.name,r'(?i)trust')
    p.admin(ctx,f'setblock {x+2} {y} {z+3} minecraft:oak_trapdoor')
    for allowed in (True,False):
        p.response(ctx,'town flag use '+('on' if allowed else 'off'),r'(?i)flag|door')
        before=p.block(ctx,(x+2,y,z+3))['properties']['open']
        p.use(ctx,(x+2,y,z+3),index=1)
        after=p.block(ctx,(x+2,y,z+3))['properties']['open']
        p.check(ctx,'door flag '+str(allowed),after!=before,allowed)


def audit_queries(ctx):
    deploy(ctx,'krasnokamen-audit')
    x,y,z=p.arena(ctx)
    p.admin(ctx,f'setblock {x+2} {y} {z} minecraft:stone')
    p.rich_action(ctx,'break_block',(x+2,y,z))
    p.block_is(ctx, (x+2,y,z), 'minecraft:air')
    p.admin(ctx, f'give {p.client(ctx).name} minecraft:stone 1')
    p.rich_action(ctx, 'select_inventory_item', 'minecraft:stone')
    p.use(ctx, (x+2,y-1,z))
    p.block_is(ctx, (x+2,y,z), 'minecraft:stone')
    p.response(ctx, 'audit 20', r'(?i)place|placed')
    for text in ('audit 20','audit here 8 20','audit since 200 20','audit actor '+p.observe(ctx,'uuid')+' 20'):
        line=p.response(ctx,text,r'(?i)break|broke|block')
        p.test(ctx,'audit actor',p.observe(ctx,'uuid') in line,line)
    # Each committed event class is exercised by the same ordinary client verbs.
    p.rich_action(ctx,'craft_recipe','minecraft:oak_planks')
    p.client(ctx).drop()
    p.rich_action(ctx,'collect_nearby_items')
    p.admin(ctx,f'summon minecraft:cow {x+2} {y} {z}')
    entity=p.rich_action(ctx,'find_entity','minecraft:cow')
    p.rich_action(ctx,'interact_entity',entity)
    p.rich_action(ctx,'attack_until_dead',entity)
    p.admin(ctx,f'kill {p.client(ctx).name}')
    p.rich_action(ctx,'respawn')
    p.client(ctx).command('audit 100')
    events='\n'.join(p.observe(ctx,'events'))
    for kind in ('craft','pickup','kill','interact','death'):
        p.test(ctx,'audit '+kind,kind in events.lower(),events)


def basic_economy_shop(ctx):
    deploy(ctx,'basic-economy')
    p.admin(ctx,f'give {p.client(ctx).name} minecraft:emerald 16')
    before=p.inventory(ctx,'minecraft:emerald')
    p.client(ctx).command('economy shop')
    offer=p.observe(ctx,'shop.offer')
    p.client(ctx).click_slot(offer['buy_slot'])
    p.rich_action(ctx,'close_container')
    p.check(ctx,'purchase debit',p.inventory(ctx,'minecraft:emerald'),before-offer['buy_cost'])
    p.check(ctx,'purchase item',p.inventory(ctx,offer['item_id']),offer['count'])
    p.client(ctx).command('economy shop')
    p.client(ctx).click_slot(offer['sell_slot'])
    p.rich_action(ctx,'close_container')
    p.check(ctx,'sale consumes purchased items',p.inventory(ctx,offer['item_id']),0)
    p.check(ctx,'sale credits exact refund',p.inventory(ctx,'minecraft:emerald'),before-offer['buy_cost']+offer['sell_credit'])
    zone=p.observe(ctx,'shop.zone')
    p.client(ctx).teleport_to(*zone['inside'])
    p.check(ctx,'market enter event',p.observe(ctx,'shop.in_market'),True)
    p.client(ctx).teleport_to(*zone['outside'])
    p.check(ctx,'market leave event',p.observe(ctx,'shop.in_market'),False)


def auth_register_login(ctx):
    deploy(ctx,'krasnokamen-auth',require_plugin_login=True)
    password='Matrix26520Secret'
    p.client(ctx).disconnect()
    p.client(ctx).reconnect()
    p.check(ctx,'join locked',p.observe(ctx,'auth.locked'),True)
    x, y, z = p.arena(ctx)
    p.admin(ctx, f'give {p.client(ctx).name} minecraft:stone 4')
    p.rich_action(ctx, 'select_inventory_item', 'minecraft:stone')
    before = p.inventory(ctx, 'minecraft:stone')
    p.block_ack(ctx, p.use(ctx, (x+2,y-1,z)))
    p.block_is(ctx, (x+2,y,z), 'minecraft:air')
    p.check(ctx, 'authentication lock denies building without item loss',
            p.inventory(ctx, 'minecraft:stone'), before)
    p.response(ctx,f'register {password} {password}',r'(?i)registered|зарегистр|logged|вошли')
    p.check(ctx,'register unlocks',p.observe(ctx,'auth.locked'),False)
    p.client(ctx).disconnect();p.client(ctx).reconnect()
    p.response(ctx,'login wrong26520',r'(?i)wrong|invalid|невер')
    p.check(ctx,'wrong password remains locked',p.observe(ctx,'auth.locked'),True)
    p.response(ctx,'login '+password,r'(?i)logged|вошли|успеш')
    p.check(ctx,'correct login unlocks',p.observe(ctx,'auth.locked'),False)
    p.response(ctx,'changepassword '+password+' Matrix26520New',r'(?i)changed|измен')
    p.client(ctx).disconnect();p.client(ctx).reconnect()
    p.response(ctx,'login '+password,r'(?i)wrong|invalid|невер')
    p.response(ctx,'login Matrix26520New',r'(?i)logged|вошли|успеш')
    for actor in range(len(ctx.clients)):
        p.check(ctx,'no secret echo '+str(actor),any(password in e or 'Matrix26520New' in e for e in p.observe(ctx,'events',actor)),False)


def auth_session_timeout(ctx):
    deploy(ctx,'krasnokamen-auth',require_plugin_login=True)
    p.client(ctx).disconnect()
    p.client(ctx).reconnect()
    p.check(ctx,'initial lock',p.observe(ctx,'auth.locked'),True)
    p.ticks(ctx,1199)
    p.check(ctx,'not kicked early',p.observe(ctx,'connection.phase'),'play')
    p.ticks(ctx,1)
    ctx.wait_until(lambda:p.observe(ctx,'connection.phase')=='disconnected',description='1200 tick authentication deadline')
    p.check(ctx,'timeout disconnect',p.observe(ctx,'connection.phase'),'disconnected')


def chisel_conversion(ctx):
    deploy(ctx,'chisel')
    x,y,z=p.arena(ctx)
    p.admin(ctx,f'give {p.client(ctx).name} chisel:chisel 1')
    p.rich_action(ctx,'select_inventory_item','chisel:chisel')
    for face,expected in [('up','minecraft:stone_brick_slab'),('north','minecraft:stone_brick_stairs')]:
        p.admin(ctx,f'setblock {x+2} {y} {z} minecraft:stone_bricks')
        before=p.inventory(ctx,'minecraft:stone_brick_slab')
        damage=p.observe(ctx,'held_item.damage')
        p.use(ctx,(x+2,y,z),face)
        p.block_is(ctx,(x+2,y,z),expected)
        p.check(ctx,'chisel wears once',p.observe(ctx,'held_item.damage'),damage+1)
        p.check(ctx,'slab grant conservation',p.inventory(ctx,'minecraft:stone_brick_slab')-before,1 if face=='up' else 0)
    p.use(ctx,(x+2,y,z),'north')
    p.block_is(ctx,(x+2,y,z),'minecraft:stone_brick_slab')


def chisel_loader_bundle(ctx):
    from .loader_content import bundle_handshake_cache, custom_items_render
    bundle_handshake_cache(ctx)
    custom_items_render(ctx)
    p.rich_action(ctx, 'reconnect_profile', 'vanilla_26_1_2')
    ctx.wait_until(lambda: p.observe(ctx, 'connection.phase') == 'play',
                   description='vanilla joins optional chisel bundle')
    p.check(ctx, 'vanilla completes Configuration', p.observe(ctx, 'connection.phase'), 'play')
    before = p.inventory(ctx, 'minecraft:stick')
    p.admin(ctx, f'give {p.client(ctx).name} chisel:chisel 1')
    ctx.wait_until(lambda: p.inventory(ctx, 'minecraft:stick') == before + 1,
                   description='vanilla receives chisel stick carrier')
    p.check(ctx, 'vanilla chisel carrier', p.inventory(ctx, 'minecraft:stick'), before + 1)


def graves_custody(ctx):
    deploy(ctx,'graves')
    p.admin(ctx,'gamerule keepInventory false')
    p.admin(ctx,f'clear {p.client(ctx).name}')
    p.admin(ctx,f'give {p.client(ctx).name} minecraft:diamond 3')
    p.admin(ctx,f'give {p.client(ctx).name} minecraft:iron_ingot 5')
    before=p.observe(ctx,'inventory')
    pos=[int(v//1) for v in p.position(ctx)]
    p.admin(ctx,f'kill {p.client(ctx).name}')
    p.rich_action(ctx,'respawn')
    p.client(ctx).teleport_to(pos[0]+1,pos[1],pos[2])
    p.block_is(ctx,pos,'minecraft:barrel')
    p.client(ctx).open_container(tuple(pos))
    stacks=[s for s in p.observe(ctx,'menu.slots') if s['slot']<27 and s.get('count',0)]
    expected=sorted((s['item_id'],s['count']) for s in before)
    p.check(ctx,'grave exact custody',sorted((s['item_id'],s['count']) for s in stacks),expected)
    p.check(ctx,'no ground copy',p.rich_action(ctx,'item_entities_in_radius',tuple(pos),4),[])


def graves_owner_protection(ctx):
    deploy(ctx,'graves',public_after_seconds=2)
    p.admin(ctx,'gamerule keepInventory false')
    p.admin(ctx,f'give {p.client(ctx).name} minecraft:diamond 1')
    pos=[int(v//1) for v in p.position(ctx)]
    p.admin(ctx,f'kill {p.client(ctx).name}')
    guest=p.client(ctx,1)
    guest.teleport_to(pos[0]+1,pos[1],pos[2])
    guest.open_container(tuple(pos))
    p.check(ctx,'private grave denies guest',p.observe(ctx,'screen.kind',1),'game')
    p.rich_action(ctx,'break_block',tuple(pos),index=1)
    p.block_is(ctx,pos,'minecraft:barrel')
    p.rich_action(ctx,'respawn')
    p.client(ctx).teleport_to(pos[0]+1,pos[1],pos[2])
    p.client(ctx).open_container(tuple(pos))
    p.check(ctx,'owner can open',p.observe(ctx,'menu.kind'),'generic_9x3')
    p.rich_action(ctx,'close_container')
    # This plugin uses wall-clock deadlines, not simulation ticks; observe its
    # explicit public notification rather than substituting a sleep/tick count.
    ctx.wait_until(lambda:any('public' in e.lower() for e in p.observe(ctx,'events')),description='grave public notification',timeout_seconds=10)
    guest.open_container(tuple(pos))
    p.check(ctx,'public grave admits guest',p.observe(ctx,'menu.kind',1),'generic_9x3')


def plugin_startup_fail_closed(ctx):
    for case,pattern in [('duplicate_death_graves','more than one package declares the death_graves'),('legacy_api_0_6','0.6')]:
        receipt=p.fixture(ctx,'plugin_invalid_startup',case=case,isolated=True)
        p.test(ctx,case+' nonzero exit',receipt['exit_code']!=0,receipt)
        p.test(ctx,case+' actionable reason',pattern in receipt['stderr'],receipt['stderr'])
        p.check(ctx,case+' no gameplay admission',receipt['play_connections'],0)


def plugin_trap_isolation(ctx):
    deploy(ctx,'harness-trapping-guest')
    before=ctx.server.dashboard_stats()['simulation_tick']
    p.client(ctx).chat('matrix-trigger-trap')
    p.ticks(ctx,20)
    after=ctx.server.dashboard_stats()['simulation_tick']
    p.test(ctx,'server ticks after guest trap',after>before,{'before':before,'after':after})
    state=p.method(ctx.server,'plugin_status')('harness-trapping-guest')
    p.check(ctx,'trap isolated',state['isolated'],True)
    p.test(ctx,'trap diagnostic',bool(state['trap_message']),state)
    p.client(ctx).chat('matrix-after-trap')
    ctx.wait_until(lambda:any('matrix-after-trap' in e for e in p.observe(ctx,'events')),description='gameplay after trap')
    p.test(ctx,'healthy session continues',True,'chat echoed after trapped guest')


def plugin_storage_persistence(ctx):
    deploy(ctx,'krasnokamen-essentials','krasnokamen-economy',starting_balance=100)
    home=p.position(ctx)
    p.response(ctx,'sethome matrix',r'matrix saved\.')
    target=p.observe(ctx,'uuid',1)
    p.response(ctx,f'pay {target} 7 persist26520',r'Paid 7 ')
    p.restart(ctx)
    p.response(ctx,'money',r'Balance: 93 ')
    p.response(ctx,'money',r'Balance: 107 ',index=1)
    p.client(ctx).teleport_to(home[0]+5,home[1],home[2])
    p.client(ctx).command('home matrix')
    p.at(ctx,home)


def plugin_command_tree(ctx):
    deploy(ctx,'krasnokamen-essentials','krasnokamen-permissions')
    for level in (0,4):
        tree=p.rich_action(ctx,'command_tree_for_permission',level)
        roots=tree['roots']
        p.test(ctx,'plugin msg advertised', 'msg' in roots,roots)
        p.check(ctx,'one owner for msg root',roots.count('msg'),1)
        p.check(ctx,'setwarp operator visibility','setwarp' in roots,level>=2)
    other=p.client(ctx,1)
    p.response(ctx,'msg '+other.name+' matrix-owned-root',r'\[to '+other.name+r'\] matrix-owned-root')


ADAPTERS={'plugins.'+fn.__name__:Adapter(p.setup,fn,p.oracle) for fn in (
    perm_groups_user_set,perm_check_node,essentials_homes,essentials_spawn_warps,essentials_back,
    essentials_tpa,essentials_msg_reply,economy_money_pay,economy_admin,towns_create_info,
    towns_protection,audit_queries,basic_economy_shop,auth_register_login,auth_session_timeout,
    chisel_conversion,chisel_loader_bundle,graves_custody,graves_owner_protection,
    plugin_startup_fail_closed,plugin_trap_isolation,plugin_storage_persistence,plugin_command_tree)}
