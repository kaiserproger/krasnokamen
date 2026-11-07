"""Vanilla: https://minecraft.wiki/w/Advancement and https://minecraft.wiki/w/Statistics.
Advancement progress comes from received authoritative updates; granting an
advancement with /advancement does not prove its gameplay criterion.
"""
from tools.harness.matrix_api import Adapter
from . import _platform as p


def granted(ctx, identifier):
    ctx.wait_until(lambda: p.observe(ctx,'advancements.progress').get(identifier,{}).get('done'),
                   description='criterion grants '+identifier)
    p.check(ctx,identifier,p.observe(ctx,'advancements.progress')[identifier]['done'],True)


def earn_advancements(ctx):
    x,y,z=p.arena(ctx)
    p.admin(ctx,f'advancement revoke {p.client(ctx).name} everything')
    p.admin(ctx,f'give {p.client(ctx).name} minecraft:wooden_pickaxe 1')
    p.rich_action(ctx,'select_inventory_item','minecraft:wooden_pickaxe')
    p.admin(ctx,f'setblock {x+2} {y} {z} minecraft:stone')
    p.rich_action(ctx,'break_block',(x+2,y,z))
    p.client(ctx).teleport_to(x+2,y,z)
    granted(ctx,'minecraft:story/mine_stone')
    # Each domain branch uses gameplay, not /advancement grant as its oracle.
    p.rich_action(ctx,'travel_dimension','minecraft:the_nether')
    granted(ctx,'minecraft:nether/root')
    p.rich_action(ctx,'travel_dimension','minecraft:the_end')
    granted(ctx,'minecraft:end/root')
    p.rich_action(ctx,'travel_dimension','minecraft:overworld')
    p.admin(ctx,f'summon minecraft:zombie {x+2} {y} {z}')
    entity=p.rich_action(ctx,'find_entity','minecraft:zombie')
    p.rich_action(ctx,'attack_until_dead',entity)
    granted(ctx,'minecraft:adventure/kill_a_mob')
    p.admin(ctx,f'summon minecraft:wolf {x+2} {y} {z}')
    p.admin(ctx,f'give {p.client(ctx).name} minecraft:bone 64')
    p.rich_action(ctx,'select_inventory_item','minecraft:bone')
    wolf=p.rich_action(ctx,'find_entity','minecraft:wolf')
    for _ in range(64):
        p.rich_action(ctx,'interact_entity',wolf)
        if p.observe(ctx,'advancements.progress').get('minecraft:husbandry/tame_an_animal',{}).get('done'):
            break
    granted(ctx,'minecraft:husbandry/tame_an_animal')
    p.test(ctx,'toast delivered',len(p.observe(ctx,'advancements.toasts'))>=1,p.observe(ctx,'advancements.toasts'))
    p.test(ctx,'parent tree exists','minecraft:story/root' in p.observe(ctx,'advancements.definitions'),p.observe(ctx,'advancements.definitions'))


def advancement_tabs_seen(ctx):
    p.admin(ctx,f'advancement grant {p.client(ctx).name} only minecraft:story/root')
    p.admin(ctx,f'advancement grant {p.client(ctx).name} only minecraft:nether/root')
    before=len(p.observe(ctx,'advancements.toasts'))
    for tab in ('minecraft:story/root','minecraft:nether/root','minecraft:story/root'):
        p.rich_action(ctx,'open_advancements',tab)
        p.check(ctx,'selected advancement tab',p.observe(ctx,'advancements.selected_tab'),tab)
    p.rich_action(ctx,'close_container')
    p.ticks(ctx,2)
    p.check(ctx,'opening tabs emits no duplicate toasts',len(p.observe(ctx,'advancements.toasts')),before)


def recipe_unlocks(ctx):
    x,y,z=p.arena(ctx)
    p.admin(ctx,f'clear {p.client(ctx).name}')
    p.admin(ctx,f'setblock {x+2} {y} {z} minecraft:oak_log')
    p.rich_action(ctx,'break_block',(x+2,y,z))
    p.client(ctx).teleport_to(x+2,y,z)
    recipes=p.rich_action(ctx,'read_recipe_book')
    p.test(ctx,'oak planks recipe unlocked','minecraft:oak_planks' in recipes,recipes)
    p.rich_action(ctx,'place_recipe','minecraft:oak_planks')
    p.check(ctx,'recipe result',p.observe(ctx,'menu.result.item_id'),'minecraft:oak_planks')
    p.check(ctx,'recipe result count',p.observe(ctx,'menu.result.count'),4)


def statistics_request(ctx):
    p.rich_action(ctx,'request_statistics')
    before=p.observe(ctx,'statistics')
    x,y,z=p.arena(ctx)
    p.admin(ctx,f'give {p.client(ctx).name} minecraft:iron_pickaxe 1')
    p.rich_action(ctx,'select_inventory_item','minecraft:iron_pickaxe')
    p.admin(ctx,f'setblock {x+2} {y} {z} minecraft:stone')
    p.rich_action(ctx,'break_block',(x+2,y,z))
    p.rich_action(ctx,'move',forward=1,ticks=20)
    p.admin(ctx,f'summon minecraft:pig {x+2} {y} {z}')
    p.rich_action(ctx,'attack_until_dead',p.rich_action(ctx,'find_entity','minecraft:pig'))
    p.rich_action(ctx,'request_statistics')
    after=p.observe(ctx,'statistics')
    for key in ('minecraft:mined/minecraft:stone','minecraft:custom/minecraft:mob_kills'):
        p.check(ctx,key+' delta',after.get(key,0)-before.get(key,0),1)
    key='minecraft:custom/minecraft:walk_one_cm'
    p.test(ctx,'walk counter advances',after.get(key,0)>before.get(key,0),{'before':before,'after':after})
    p.rich_action(ctx,'request_statistics')
    p.check(ctx,'requests do not increment counters',p.observe(ctx,'statistics'),after)


def advancement_location(ctx):
    p.admin(ctx,f'advancement revoke {p.client(ctx).name} only minecraft:nether/find_fortress')
    p.rich_action(ctx,'travel_dimension','minecraft:the_nether')
    result=p.rich_action(ctx,'locate_structure','minecraft:fortress')
    p.client(ctx).teleport_to(*result['interior'])
    granted(ctx,'minecraft:nether/find_fortress')
    p.rich_action(ctx,'travel_dimension','minecraft:overworld')
    result=p.rich_action(ctx,'locate_biome','minecraft:deep_dark')
    p.client(ctx).teleport_to(*result['position'])
    # Local 26.1.2 advancement/adventure/adventuring_time.json:
    # criteria.minecraft:deep_dark is a minecraft:location trigger.
    ctx.wait_until(lambda: p.observe(ctx, 'advancements.progress')[
        'minecraft:adventure/adventuring_time']['criteria'].get('minecraft:deep_dark', False),
        description='deep_dark location criterion')
    p.check(ctx, 'deep_dark criterion', p.observe(ctx, 'advancements.progress')[
        'minecraft:adventure/adventuring_time']['criteria']['minecraft:deep_dark'], True)


ADAPTERS={'advancements_stats.'+fn.__name__:Adapter(p.setup,fn,p.oracle) for fn in (
    earn_advancements,advancement_tabs_seen,recipe_unlocks,statistics_request,advancement_location)}
