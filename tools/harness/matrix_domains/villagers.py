"""Villager world-state adapters. Sources: minecraft.wiki/w/Villager.

No successful summon/open-menu is treated as proof of work, conversion or birth.
"""
from collections import Counter
from ..matrix_api import Adapter, Check, MissingCapability
from ._mob_support import arena, count, entity, exists, field, inventory, number, require_verb, string, ticks

UNEMPLOYED='12345678-1234-1234-1234-123456789b01'
NITWIT='12345678-1234-1234-1234-123456789b02'
LIGHTNING='12345678-1234-1234-1234-123456789b03'


def jobs_setup(ctx):
    arena(ctx)
    ctx.server.command('gamerule doMobSpawning false')
    ctx.server.command('time set 2000')
    ctx.server.command(f'summon villager 3 200 0 {{UUID:"{UNEMPLOYED}",VillagerData:{{profession:"minecraft:none"}}}}')
    ctx.server.command(f'summon villager 8 200 0 {{UUID:"{NITWIT}",VillagerData:{{profession:"minecraft:nitwit"}}}}')
    ctx.state['before']=[string(ctx,UNEMPLOYED,'VillagerData.profession'),string(ctx,NITWIT,'VillagerData.profession')]
    if ctx.state['before'] != ['minecraft:none','minecraft:nitwit']:
        raise AssertionError('job-site controls have wrong authored professions')


def jobs_act(ctx):
    ctx.server.command('setblock 4 200 0 composter')
    ctx.wait_until(lambda:string(ctx,UNEMPLOYED,'VillagerData.profession')=='minecraft:farmer',
                   description='unemployed villager actually claims composter',timeout_seconds=90)
    ctx.boundary()
    ctx.server.command('setblock 9 200 0 composter')
    ticks(ctx,1200)


def jobs_oracle(ctx):
    actual=[string(ctx,UNEMPLOYED,'VillagerData.profession'),string(ctx,NITWIT,'VillagerData.profession')]
    checks=[Check('unemployed_acquires_farmer',actual[0]=='minecraft:farmer',actual),
            Check('nitwit_never_acquires_job',actual[1]=='minecraft:nitwit',actual)]
    for observer in ctx.clients[1:]:
        def received():
            rows=field(observer,'entities')
            for uuid,profession in ((UNEMPLOYED,'minecraft:farmer'),(NITWIT,'minecraft:nitwit')):
                row=next((r for r in rows if r.get('uuid')==uuid),None)
                if row is None:
                    return False
                data=row.get('villager_data')
                if data is None:
                    raise MissingCapability('snapshot:entities.villager_data')
                if data.get('profession')!=profession:
                    return False
            return True
        observer.wait_until(received,description='observer receives claimed and nitwit profession metadata')
        checks.append(Check('observer_receives_profession_metadata:'+observer.name,received()))
    return checks


def lightning_setup(ctx):
    arena(ctx)
    ctx.server.command('gamerule doMobSpawning false')
    ctx.server.command(f'summon villager 4 200 0 {{UUID:"{LIGHTNING}",NoAI:1b,CustomName:"MatrixLightning"}}')
    if not exists(ctx,LIGHTNING):
        raise AssertionError('lightning victim absent before strike')
    ctx.state['witches_before']=count(ctx,'witch')
    ctx.state['observer_witches_before']=[
        [row['uuid'] for row in field(observer,'entities') if row.get('type')=='minecraft:witch']
        for observer in ctx.clients[1:]]


def lightning_act(ctx):
    ctx.server.command('summon lightning_bolt 4 200 0')
    if ctx.factors['difficulty']=='peaceful':
        ticks(ctx,20)
    else:
        ctx.wait_until(lambda:exists(ctx,'@e[type=witch,name=MatrixLightning]'),
                       description='lightning changes the named villager into a witch')
    ctx.boundary()
    ticks(ctx,20)


def lightning_oracle(ctx):
    # The subject requires a type transition, not a fresh UUID. Solaris's
    # replace_type deliberately preserves entity identity. Peaceful skips the
    # conversion altogether (weather_lightning.rs:149).
    peaceful=ctx.factors['difficulty']=='peaceful'
    expected=ctx.state['witches_before']+int(not peaceful)
    villager=exists(ctx,'@e[type=villager,name=MatrixLightning]')
    witch=exists(ctx,'@e[type=witch,name=MatrixLightning]')
    checks=[Check('named_villager_type_transition',villager==peaceful,{'villager_present':villager,'peaceful':peaceful}),
            Check('named_witch_type_transition',witch!=peaceful,{'witch_present':witch,'peaceful':peaceful}),
            Check('exact_witch_delta',count(ctx,'witch')==expected,{'expected':expected,'actual':count(ctx,'witch')})]
    for observer,before in zip(ctx.clients[1:],ctx.state['observer_witches_before'],strict=True):
        def received():
            rows=field(observer,'entities')
            if peaceful:
                return any(row.get('uuid')==LIGHTNING and row.get('type')=='minecraft:villager' for row in rows)
            return (not any(row.get('uuid')==LIGHTNING and row.get('type')=='minecraft:villager' for row in rows)
                    and len([row for row in rows if row.get('type')=='minecraft:witch' and row['uuid'] not in before])==1)
        observer.wait_until(received,description='observer receives lightning type transition')
        checks.append(Check('observer_receives_lightning_transition:'+observer.name,received()))
    return checks


def breeding_setup(ctx):
    arena(ctx)
    ctx.server.command('gamerule doMobSpawning false')
    ctx.server.command('gamerule mobGriefing true')
    ctx.server.command('fill -2 199 -3 10 204 7 glass hollow')
    ctx.server.command('time set 2000')
    for uuid,x in ((UNEMPLOYED,3),(NITWIT,6)):
        ctx.server.command(f'summon villager {x} 200 0 {{UUID:"{uuid}",Age:0}}')
    for x in (3,5,7):
        ctx.server.command(f'setblock {x} 200 3 red_bed[part=foot,facing=south]')
        ctx.server.command(f'setblock {x} 200 4 red_bed[part=head,facing=south]')
    if count(ctx,'villager')!=2:
        raise AssertionError('breeding must start with exactly two adults')


def breeding_act(ctx):
    # Food is supplied as real item entities; cannot bypass willingness by
    # summoning a baby or pre-setting the parents' breeding state.
    ctx.server.command('summon item 3 200 0 {Item:{id:"minecraft:bread",count:3},PickupDelay:0}')
    ctx.wait_until(lambda:(number(ctx,UNEMPLOYED,'Inventory[0].count',optional=True) or 0)>=3,
                   description='first adult picks up three real bread')
    ctx.boundary()
    ctx.server.command('summon item 6 200 0 {Item:{id:"minecraft:bread",count:3},PickupDelay:0}')
    ctx.wait_until(lambda:count(ctx,'villager')==3,description='willing adults use spare bed to breed',timeout_seconds=150)
    # Both authored adults have known UUIDs. A selector counts babies through
    # their received UUIDs rather than assuming nearest means newborn.
    rows=ctx.clients[0].snapshot().get('entities')
    if rows is None:
        from ..matrix_api import MissingCapability
        raise MissingCapability('snapshot:entities')
    babies=[r for r in rows if r.get('type')=='minecraft:villager' and r.get('uuid') not in (UNEMPLOYED,NITWIT)]
    if len(babies)!=1:
        raise AssertionError('birth did not publish exactly one new villager identity')
    ctx.state['baby_age']=number(ctx,babies[0]['uuid'],'Age')
    ctx.state['baby_uuid']=babies[0]['uuid']


def breeding_oracle(ctx):
    checks=[Check('one_baby',count(ctx,'villager')==3),
            Check('baby_age_negative',-24000<=ctx.state['baby_age']<0,ctx.state['baby_age']),
            Check('both_adults_survive',exists(ctx,UNEMPLOYED) and exists(ctx,NITWIT))]
    for observer in ctx.clients[1:]:
        def received():
            return any(row.get('uuid')==ctx.state['baby_uuid'] and row.get('type')=='minecraft:villager'
                       for row in field(observer,'entities'))
        observer.wait_until(received,description='observer receives newborn villager identity')
        checks.append(Check('observer_receives_newborn:'+observer.name,received()))
    return checks


JOBS={
    'Armorer':'blast_furnace','Butcher':'smoker','Cartographer':'cartography_table',
    'Cleric':'brewing_stand','Farmer':'composter','Fisherman':'barrel',
    'Fletcher':'fletching_table','Leatherworker':'cauldron','Librarian':'lectern',
    'Mason':'stonecutter','Shepherd':'loom','Toolsmith':'smithing_table',
    'Weaponsmith':'grindstone','None':None,'Nitwit':None,
}


def profession_setup(ctx):
    arena(ctx)
    ctx.server.command('gamerule doMobSpawning false')
    ctx.server.command('time set 2000')
    job=ctx.row['subject'].split('profession_',1)[1]
    ctx.state['job']=job
    block=JOBS[job]
    if block:
        ctx.server.command(f'setblock 4 200 0 {block}')
    ctx.server.command(f'summon villager 3 200 0 {{UUID:"{UNEMPLOYED}",VillagerData:{{profession:"minecraft:{job.lower()}",level:1,type:"minecraft:plains"}}}}')
    ctx.server.command(f'clear {ctx.clients[0].name}')
    ctx.wait_until(lambda:inventory(ctx.clients[0])=={},description='received empty trade fixture inventory')
    if string(ctx,UNEMPLOYED,'VillagerData.profession')!='minecraft:'+job.lower():
        raise AssertionError('authored villager profession did not survive admission')
    require_verb(ctx.clients[0],'interact_entity')


def profession_act(ctx):
    actor=ctx.clients[0]
    row=entity(ctx,UNEMPLOYED)
    if row is None:
        raise AssertionError('trading villager not published')
    actor.teleport_to(3,200,-2)
    actor.look_at(3,201,0)
    require_verb(actor,'interact_entity')(row['id'])
    no_trade=ctx.state['job'] in {'None','Nitwit'} or ctx.factors['game_mode']=='spectator'
    ctx.state['no_trade']=no_trade
    if no_trade:
        ticks(ctx,20)
        snapshot=actor.snapshot()
        if 'menu' not in snapshot:
            raise MissingCapability('snapshot:menu')
        menu=snapshot['menu']
        ctx.state['negative_menu']=menu is None or menu.get('id')==0
        ctx.boundary()
        return
    ctx.wait_until(lambda:(actor.snapshot().get('menu') or {}).get('id',0)>0,
                   description='received merchant window')
    menu=actor.snapshot()['menu']
    offers=menu.get('offers')
    if offers is None:
        raise MissingCapability('snapshot:menu.offers')
    if len(offers)<2:
        raise AssertionError('novice must expose at least two offers')
    offer=offers[0]
    for key in ('input1','input2','output','uses','max_uses'):
        if key not in offer:
            raise MissingCapability('snapshot:menu.offers.'+key)
    ctx.state['offer']=offer.copy()
    require_verb(actor,'close_container')()
    for stack in (offer['input1'],offer['input2']):
        if stack:
            ctx.server.command(f'give {actor.name} {stack["item_id"]} {stack["count"]}')
    expected=Counter()
    for stack in (offer['input1'],offer['input2']):
        if stack:
            expected[stack['item_id']]+=stack['count']
    ctx.wait_until(lambda:inventory(actor)==dict(expected),description='received exactly one trade cost')
    ctx.state['trade_before']=inventory(actor)
    require_verb(actor,'interact_entity')(row['id'])
    ctx.wait_until(lambda:(actor.snapshot().get('menu') or {}).get('id',0)>0,
                   description='received reopened merchant window')
    require_verb(actor,'select_trade')(0)
    def result_present():
        slots=(actor.snapshot().get('menu') or {}).get('slots',[])
        return any(s['slot']==2 and s['item_id']==offer['output']['item_id'] and
                   s['count']==offer['output']['count'] for s in slots)
    ctx.wait_until(result_present,description='selected recipe produces exact result stack')
    actor.click_slot(2,kind='quick_move',button=0)
    def completed():
        current=(actor.snapshot().get('menu') or {}).get('offers')
        if current is None:
            raise MissingCapability('snapshot:menu.offers')
        return current[0]['uses']==offer['uses']+1
    ctx.wait_until(completed,description='merchant acknowledges one completed trade')
    ctx.boundary()
    require_verb(actor,'close_container')()
    wanted={offer['output']['item_id']:offer['output']['count']}
    ctx.wait_until(lambda:inventory(actor)==wanted,description='one output credited, both inputs debited')
    ctx.state['trade_after']=inventory(actor)


def profession_oracle(ctx):
    checks=[Check('profession_retained',
                  string(ctx,UNEMPLOYED,'VillagerData.profession')=='minecraft:'+ctx.state['job'].lower())]
    if ctx.state['no_trade']:
        checks.extend([Check('nontrading_profession_or_spectator_no_window',ctx.state['negative_menu']),
                       Check('no_inventory_creation',inventory(ctx.clients[0])=={})])
        return checks
    output=ctx.state['offer']['output']
    checks.append(Check('advertised_recipe_exact_custody',
                        ctx.state['trade_after']=={output['item_id']:output['count']},
                        {'input':ctx.state['trade_before'],'output':ctx.state['trade_after'],
                         'offer':ctx.state['offer'],'source':'https://minecraft.wiki/w/Trading'}))
    return checks


ADAPTERS={
    'villagers.job_site_claim':Adapter(jobs_setup,jobs_act,jobs_oracle),
    'villagers.lightning_to_witch':Adapter(lightning_setup,lightning_act,lightning_oracle),
    'villagers.villager_breeding':Adapter(breeding_setup,breeding_act,breeding_oracle),
}
ADAPTERS.update({'villagers.profession_'+job:Adapter(profession_setup,profession_act,profession_oracle)
                 for job in JOBS})
