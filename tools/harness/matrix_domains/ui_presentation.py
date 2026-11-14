"""Vanilla UI sources: minecraft.wiki/w/{Commands/bossbar,Commands/title,
Commands/tellraw,Text_component_format,Player,Glowing,Invisibility,Dialog}.
Krasnokamen dialog extension evidence: mc-core/src/dialog.rs, mc-edge/src/boss_bars.rs.
"""
from tools.harness.matrix_api import Adapter
from . import _platform as p


def bossbar_custom(ctx):
    p.client(ctx).command('bossbar add matrix:test {"text":"Matrix"}')
    p.client(ctx).command(f'bossbar set matrix:test players {p.client(ctx).name}')
    p.client(ctx).command('bossbar set matrix:test max 100')
    p.client(ctx).command('bossbar set matrix:test value 25')
    p.client(ctx).command('bossbar set matrix:test color blue')
    p.client(ctx).command('bossbar set matrix:test style notched_10')
    bar=p.observe(ctx,'bossbars')['matrix:test']
    for key,expected in {'name':'Matrix','progress':.25,'color':'blue','style':'notched_10'}.items():
        p.check(ctx,'bossbar '+key,bar[key],expected)
    p.client(ctx).command('bossbar remove matrix:test')
    p.test(ctx,'bossbar removed','matrix:test' not in p.observe(ctx,'bossbars'),p.observe(ctx,'bossbars'))


def bossbar_entities(ctx):
    x,y,z=p.arena(ctx)
    for mob in ('minecraft:wither','minecraft:ender_dragon'):
        p.admin(ctx,f'summon {mob} {x+5} {y+2} {z}')
        entity=p.rich_action(ctx,'find_entity',mob)
        before=p.rich_action(ctx,'entity_bossbar',entity)
        p.test(ctx,mob+' bar visible',before['visible'],before)
        p.client(ctx).attack(entity)
        after=p.rich_action(ctx,'entity_bossbar',entity)
        p.check(ctx,mob+' progress follows health',after['progress'],after['health']/after['max_health'])
        p.admin(ctx,f'kill @e[type={mob}]')
        p.check(ctx,mob+' bar removed',p.rich_action(ctx,'entity_bossbar',entity)['visible'],False)
    raid=p.fixture(ctx,'vanilla_raid',center=(x,y,z))
    bar=p.rich_action(ctx,'raid_bossbar',raid['id'])
    p.check(ctx,'raid progress',bar['progress'],bar['remaining_health']/bar['wave_health'])


def title_actionbar(ctx):
    name=p.client(ctx).name
    p.client(ctx).command(f'title {name} times 2 10 2')
    p.client(ctx).command(f'title {name} subtitle {{"text":"Matrix subtitle"}}')
    p.client(ctx).command(f'title {name} title {{"text":"Matrix title"}}')
    p.check(ctx,'title text',p.observe(ctx,'hud.title'),'Matrix title')
    p.check(ctx,'subtitle text',p.observe(ctx,'hud.subtitle'),'Matrix subtitle')
    p.check(ctx,'title durations',p.observe(ctx,'hud.title_times'),[2,10,2])
    p.client(ctx).command(f'title {name} actionbar {{"text":"Matrix action"}}')
    p.check(ctx,'actionbar',p.observe(ctx,'hud.actionbar'),'Matrix action')
    p.ticks(ctx,14)
    p.check(ctx,'title expires',p.observe(ctx,'hud.title_visible'),False)
    p.client(ctx).command(f'title {name} clear')
    p.check(ctx,'clear title',p.observe(ctx,'hud.title_visible'),False)
    p.client(ctx).command(f'title {name} reset')
    p.check(ctx,'reset timing',p.observe(ctx,'hud.title_times'),[10,70,20])


def tellraw_components(ctx):
    name=p.client(ctx).name
    p.admin(ctx,'scoreboard objectives add matrix dummy')
    p.admin(ctx,f'scoreboard players set {name} matrix 7')
    # 26.1 component keys are snake_case; legacy matrix camelCase is not a source.
    component='[{"text":"Matrix","click_event":{"action":"suggest_command","command":"/help"},"hover_event":{"action":"show_text","value":{"text":"Hover"}}},{"score":{"name":"'+name+'","objective":"matrix"}},{"selector":"@s"},{"translate":"chat.type.text","with":["Matrix","translated"]}]'
    p.client(ctx).command(f'tellraw {name} {component}')
    observed=p.rich_action(ctx,'last_chat_component')
    p.check(ctx,'score resolved',observed['resolved_score'],'7')
    p.check(ctx,'selector resolved',observed['resolved_selector'],name)
    p.check(ctx,'translation rendered',observed['rendered_translation'],'<Matrix> translated')
    p.check(ctx,'click suggestion',observed['suggest_command'],'/help')
    p.check(ctx,'hover text',observed['hover_text'],'Hover')


def tab_list(ctx):
    other=p.client(ctx,1)
    p.test(ctx,'both players listed',all(c.name in p.observe(ctx,'tab_list') for c in ctx.clients[:2]),p.observe(ctx,'tab_list'))
    p.admin(ctx,f'gamemode spectator {other.name}')
    p.check(ctx,'spectator style',p.observe(ctx,'tab_list')[other.name]['spectator'],True)
    p.check(ctx,'spectator mode update',p.observe(ctx,'tab_list')[other.name]['gamemode'],'spectator')
    p.rich_action(ctx,'reply_keepalive',index=1)
    latency=p.observe(ctx,'tab_list')[other.name]['latency_ms']
    p.test(ctx,'latency supplied',isinstance(latency,int) and latency>=0,latency)
    other.disconnect()
    ctx.wait_until(lambda:other.name not in p.observe(ctx,'tab_list'),description='tab entry removed on disconnect')
    p.test(ctx,'disconnected player removed',other.name not in p.observe(ctx,'tab_list'),p.observe(ctx,'tab_list'))
    other.reconnect()
    p.test(ctx,'rejoined player listed',other.name in p.observe(ctx,'tab_list'),p.observe(ctx,'tab_list'))


def server_dialogs(ctx):
    dialog=p.fixture(ctx,'server_dialog',fields={'text':'initial','slider':5,'checkbox':False})
    p.rich_action(ctx,'set_screen_input','text','matrix')
    p.rich_action(ctx,'set_screen_input','slider',7)
    p.rich_action(ctx,'set_screen_input','checkbox',True)
    p.rich_action(ctx,'click_screen_button','submit')
    received=p.method(ctx.server,'dialog_submission')(dialog['id'])
    p.check(ctx,'custom click payload',received,{'text':'matrix','slider':7,'checkbox':True})


def nameplates_labels(ctx):
    x,y,z=p.arena(ctx)
    p.admin(ctx,f'summon minecraft:pig {x+2} {y} {z} {{CustomName:"MatrixPig",CustomNameVisible:1b}}')
    entity=p.rich_action(ctx,'find_entity','minecraft:pig')
    p.check(ctx,'visible custom label',p.rich_action(ctx,'entity_label',entity),'MatrixPig')
    p.admin(ctx,f'summon minecraft:cow {x+4} {y} {z} {{CustomName:"HiddenCow",CustomNameVisible:0b}}')
    cow=p.rich_action(ctx,'find_entity','minecraft:cow')
    p.client(ctx).look_at(x,y+4,z)
    p.check(ctx,'hidden label when not targeted',p.rich_action(ctx,'entity_label',cow),None)
    settlement=p.fixture(ctx,'settlement_label',name='Matrix Settlement')
    p.check(ctx,'settlement label',p.rich_action(ctx,'entity_label',settlement['entity_id']),'Matrix Settlement')


def entity_glow_invisibility(ctx):
    other=p.client(ctx,1)
    for effect,flag in [('glowing',0x40),('invisibility',0x20)]:
        p.admin(ctx,f'effect give {other.name} minecraft:{effect} 30 0')
        flags=p.rich_action(ctx,'entity_metadata_flags',p.observe(ctx,'entity_id',1))
        p.test(ctx,effect+' metadata flag',bool(flags&flag),flags)
        p.check(ctx,effect+' rendered',p.rich_action(ctx,'entity_effect_rendered',p.observe(ctx,'entity_id',1),effect),True)
        p.admin(ctx,f'effect clear {other.name} minecraft:{effect}')
        flags=p.rich_action(ctx,'entity_metadata_flags',p.observe(ctx,'entity_id',1))
        p.check(ctx,effect+' flag cleared',flags&flag,0)


ADAPTERS={'ui_presentation.'+fn.__name__:Adapter(p.setup,fn,p.oracle) for fn in (
    bossbar_custom,bossbar_entities,title_actionbar,tellraw_components,tab_list,
    server_dialogs,nameplates_labels,entity_glow_invisibility)}
