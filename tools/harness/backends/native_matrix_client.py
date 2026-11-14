"""Ordinary native MCP verbs for the shared matrix_api.MatrixClient protocol.

No protocol-bot fallback, fabricated observation, fixture teleport or hidden give.
Unknown verbs are explicit capability gaps for the calling domain adapter.
"""
from __future__ import annotations
import math
import time
from typing import Any, Callable


def missing(name: str) -> None:
    from ..matrix_api import MissingCapability
    raise MissingCapability('native_mcp:' + name)


def received_stack(row: dict[str,Any] | None) -> dict[str,Any] | None:
    """Translate only fields exposed by the received-stack MCP observation."""
    if row is None:
        return None
    result=dict(row)
    components=dict(row.get('components',{}))
    for key in ('damage','max_damage'):
        if key in row:
            components['minecraft:'+key]=row[key]
    if 'enchantments' in row:
        components['minecraft:enchantments']={entry['id']:entry['level'] for entry in row['enchantments']}
    if components:
        result['components']=components
    return result


class NativeMatrixClient:
    def __init__(self, name: str, client: Any, address: str, transcript: list[dict[str, Any]],
                 on_reconnect: Callable[[Any,str],None] | None=None,
                 fixture_command: Callable[[str],str] | None=None) -> None:
        self.name, self.client, self.address, self.transcript = name, client, address, transcript
        self.on_reconnect=on_reconnect
        self.fixture_command=fixture_command
        self.first_item_position: dict[str, list[float]]={}

    def call(self, tool: str, arguments: dict[str, Any] | None = None) -> dict[str, Any]:
        row: dict[str, Any] = {'client':self.name,'tool':tool,'arguments':arguments or {}}
        self.transcript.append(row)
        try:
            result=self.client.call_tool(tool,arguments or {})
            row['result']=result
            return result
        except Exception as error:
            row['error']=str(error)
            raise

    def _entity_row(self, row: dict[str, Any]) -> dict[str, Any]:
        result={**row,'id':row['entity_id'],'uuid':row['entity_uuid'],
                'type':row['entity_type'],'position':[row[key] for key in ('x','y','z')]}
        if row['entity_type']=='minecraft:item':
            # The MCP observation has no spawn packet position. A dropped item
            # barely moves before its first observation, so its first received
            # position is the birth position for spatial drop attribution.
            # Moving mobs get none: their first sighting is not their birth.
            result['spawn_position']=list(self.first_item_position.setdefault(
                row['entity_uuid'],result['position']))
        return result

    def snapshot(self) -> dict[str, Any]:
        raw=self.call('minecraft_observe')
        player=raw.get('player',{})
        container=raw.get('container',{})
        inventory=[]
        for row in raw.get('inventory',[]):
            slot=row['slot']
            # The native observation probes two non-player indices (wire
            # 46/47) after offhand. They are not inventory custody slots.
            if slot>40:
                continue
            wire=36+slot if slot<=8 else slot if slot<=35 else 44-slot if slot<=39 else 45
            inventory.append({**received_stack(row),'slot':wire,'native_slot':slot})
        screen_class=raw.get('screen',{}).get('class','')
        phase='play' if raw.get('in_play') else 'disconnected' if screen_class.endswith('DisconnectedScreen') else 'not_playing'
        reason=None
        if phase=='disconnected':
            reason=self.call('minecraft_wait_for_play',{'timeout_seconds':.1}).get('disconnect_reason')
        menu=None
        if container.get('container_id'):
            menu={'id':container['container_id'],'state_id':container.get('container_state_id'),
                  'slots':[received_stack(row) for row in container.get('slots',[])],
                  'slot_count':container.get('slot_count')}
            trades=container.get('trades')
            if trades is not None:
                menu['offers']=[{**offer,'input1':received_stack(offer['buy_a']),
                                  'input2':received_stack(offer['buy_b']),
                                  'output':received_stack(offer['sell'])} for offer in trades['offers']]
                for native,shared in (('villager_level','level'),('villager_xp','xp'),
                                      ('show_progress','show_progress'),('can_restock','can_restock')):
                    if native in trades:
                        menu[shared]=trades[native]
        return {**raw,'position':[player.get(key) for key in ('x','y','z')],
                'inventory':inventory,'cursor':received_stack(container.get('carried')),
                'menu':menu,
                'health':player.get('health'),'food':player.get('food'),
                'experience_level':player.get('experience_level'),
                'experience_total':player.get('total_experience'),
                'selected_hotbar':player.get('selected_hotbar_slot'),
                'events':list(reversed(raw.get('recent_chat',[]))),
                'entity_id':player.get('entity_id'),'position_origin':player.get('origin'),
                'server_position':player.get('server_position'),
                'connection':{'phase':phase,'disconnect_reason':reason},
                'entities':[self._entity_row(row) for row in self.entities()] if raw.get('in_play') else []}

    def wait_until(self, predicate: Callable[[], Any], *, description: str,
                   timeout_seconds: float = 30.0) -> Any:
        deadline=time.monotonic()+timeout_seconds
        while True:
            # Subscribe using the state before predicate queries. A publication
            # during the predicate causes the wait to return immediately.
            version=self.call('minecraft_observe')['state_version']
            result=predicate()
            if result:
                return result
            remaining=deadline-time.monotonic()
            if remaining<0.1:
                raise TimeoutError(description)
            self.call('minecraft_wait_for_state_change',{'observed_version':version,
                      'timeout_seconds':min(120.0,remaining)})

    def command(self, text: str) -> None:
        self.call('minecraft_send_chat',{'message':text.lstrip('/'),'command':True})

    def chat(self, text: str) -> None:
        self.call('minecraft_send_chat',{'message':text,'command':False})

    def teleport_to(self, x: float, y: float, z: float) -> None:
        if self.fixture_command is None:
            missing('teleport_fixture:operator_console_not_bound')
        before=self.call('minecraft_observe')['state_version']
        self.fixture_command(f'tp {self.name} {x} {y} {z}')
        def arrived() -> bool:
            state=self.call('minecraft_observe')
            position=state.get('player',{}).get('server_position')
            return (state['state_version']>before and position is not None
                    and all(abs(a-b)<.2 for a,b in zip(position,(x,y,z))))
        self.wait_until(arrived,description='received fixture teleport correction')
        self.wait_for_chunk((math.floor(x),math.floor(y),math.floor(z)))

    def wait_for_chunk(self, position: tuple[int,int,int]) -> None:
        # Y=80 is inside all three pinned vanilla dimensions; chunk residency
        # is an x/z fact. This is the real received-world block-load tool.
        self.call('minecraft_wait_for_loaded_block',{'x':math.floor(position[0]),'y':80,
                  'z':math.floor(position[2]),'timeout_seconds':60})

    def look_at(self, x: float, y: float, z: float) -> None:
        position=self.snapshot()['position']
        if any(value is None for value in position):
            missing('look_at:player_position_unavailable')
        dx,dy,dz=x-position[0], y-(position[1]+1.62), z-position[2]
        yaw=round(math.degrees(math.atan2(-dx,dz)))
        pitch=round(-math.degrees(math.atan2(dy,math.hypot(dx,dz))))
        self.call('minecraft_look',{'yaw_deg':yaw,'pitch_deg':pitch})

    def read_block_state(self, position: tuple[int,int,int]) -> dict[str, Any]:
        result=self.call('minecraft_read_block',dict(zip(('x','y','z'),position)))
        if result.get('loaded') is False or not result.get('block_id'):
            missing('read_block_state:unloaded_or_unknown')
        return result

    def use_item_on(self, position: tuple[int,int,int], face: str='up', hand: str='main',
                    cursor: tuple[float,float,float]=(.5,.5,.5)) -> None:
        if cursor != (.5,.5,.5):
            missing('use_item_on:explicit_cursor_hit_unavailable')
        if hand not in {'main','off'}:
            raise ValueError('hand must be main or off')
        self.call('minecraft_use_item_on',{**dict(zip(('x','y','z'),position)),
                                          'face':face,'hand':'main_hand' if hand=='main' else 'off_hand'})

    def entities(self, radius: float=32.0) -> list[dict[str, Any]]:
        result=self.call('minecraft_list_entities',{'radius':radius,'limit':512})
        if result.get('truncated'):
            missing('entity_query:truncated')
        return result['entities']

    def _entity(self, entity_id: int) -> dict[str, Any]:
        entity=next((row for row in self.entities() if row['entity_id']==entity_id),None)
        if entity is None:
            missing('entity_identity:not_observed')
        return {key:entity[key] for key in ('entity_id','entity_uuid','entity_type')}

    def attack(self, entity_id: int) -> None:
        self.call('minecraft_attack_entity_once',self._entity(entity_id))


    def click_slot(self, slot: int, kind: str='pickup', button: int=0) -> None:
        if kind not in {'pickup','quick_move'} or button not in (0,1):
            missing('click_mode:'+kind)
        if kind=='quick_move':
            self.call('minecraft_quick_move_container_slot',{'slot':slot})
        else:
            self.call('minecraft_click_container_slot',{'slot':slot,'button':'primary' if button==0 else 'secondary'})

    def interact_entity(self, entity_id: int, hand: str='main', secondary: bool=False,
                        location: tuple[float,float,float] | None=None) -> None:
        if secondary or location is not None:
            missing('interact_entity:secondary_or_explicit_interact_at')
        if hand not in {'main','off'}:
            raise ValueError('hand must be main or off')
        self.call('minecraft_interact_entity',{**self._entity(entity_id),
                  'hand':'main_hand' if hand=='main' else 'off_hand'})

    def open_container(self, position: tuple[int,int,int]) -> None:
        self.use_item_on(position)
        self.wait_until(lambda:self.snapshot()['menu'],description='received container menu')

    def close_container(self) -> None:
        self.call('minecraft_close_screen')

    def drop(self, all: bool=False) -> None:
        selected=self.call('minecraft_observe').get('selected_item')
        if not selected or not selected.get('count'):
            missing('drop:empty_selected_item')
        if all and selected['count']>1:
            missing('drop_whole_stack:ordinary_ctrl_Q_not_exposed')
        self.call('minecraft_drop_selected_item',{'item_id':selected['item_id'],
                  'count':selected['count'] if all else 1})

    def select_hotbar(self, slot: int) -> None:
        self.call('minecraft_set_hotbar_slot',{'slot':slot})

    def disconnect(self) -> None:
        self.call('minecraft_disconnect')

    def reconnect(self) -> None:
        self.call('minecraft_connect',{'server_addr':self.address})
        state=self.call('minecraft_wait_for_play',{'timeout_seconds':60})
        if state.get('in_play') is not True:
            raise RuntimeError('native reconnect did not reach Play: '+str(state))
        if self.on_reconnect is not None:
            self.on_reconnect(self.client,self.name)

    def wait_ticks(self, ticks: int) -> None:
        while ticks>0:
            count=min(ticks,255)
            self.call('minecraft_wait_ticks',{'ticks':count}); ticks-=count

    def press_inputs(self, *, forward: bool=False, backward: bool=False, left: bool=False,
                     right: bool=False, jump: bool=False, sneak: bool=False,
                     sprint: bool=False, ticks: int=1) -> None:
        if not 1<=ticks<=255:
            missing('press_inputs:bounded_1_255_ticks_required')
        keys=[key for key,enabled in [('forward',forward),('back',backward),('left',left),
              ('right',right),('jump',jump),('sneak',sneak),('sprint',sprint)] if enabled]
        if keys:
            self.call('minecraft_press_inputs',{'keys':keys,'ticks':ticks})
        else:
            self.wait_ticks(ticks)

    def move(self, forward: float=0, strafe: float=0, ticks: int=20, sprint: bool=False) -> None:
        if forward not in (-1,0,1) or strafe not in (-1,0,1):
            missing('move:analog_magnitude_not_exposed')
        self.press_inputs(forward=forward>0,backward=forward<0,left=strafe>0,
                          right=strafe<0,sprint=sprint,ticks=ticks)

    def move_to(self, x: float, y: float, z: float, on_ground: bool=True) -> None:
        missing('move_to:raw_position_proposal_not_exposed')

    def jump(self, ticks: int=1) -> None:
        self.press_inputs(jump=True,ticks=ticks)

    def sneak(self, enabled: bool) -> None:
        missing('sneak:persistent_input_state_not_exposed')

    def sprint(self, enabled: bool) -> None:
        missing('sprint:persistent_input_state_not_exposed')

    def use_item(self, hand: str='main', ticks: int=1) -> None:
        if hand!='main':
            missing('use_item:explicit_off_hand_not_exposed')
        self.call('minecraft_press_inputs',{'keys':['use'],'ticks':ticks})

    def release_use_item(self) -> None:
        missing('release_use_item:MCP_bounded_use_auto_releases')

    def break_block(self, position: tuple[int,int,int], face: str='up', phase: str='complete') -> None:
        if phase!='complete':
            missing('break_block:separate_start_abort_not_exposed')
        self.call('minecraft_break_block',{**dict(zip(('x','y','z'),position)),'face':face,
                  'expected_drop_item_id':'minecraft:air','expected_drop_count':0,'timeout_seconds':120})

    def open_inventory(self) -> None:
        self.call('minecraft_open_inventory')

    def container_button(self, button: int) -> None:
        self.call('minecraft_click_container_button',{'button_id':button})

    def swap_hands(self) -> None:
        self.call('minecraft_press_inputs',{'keys':['swap_offhand'],'ticks':1})

    def respawn(self) -> None:
        self.call('minecraft_respawn',{'timeout_seconds':60})

    def __getattr__(self, name: str) -> Any:
        missing('verb:'+name)
