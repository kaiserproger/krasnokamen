"""SVC is a mod extension, never vanilla behavior.
Source https://modrepo.de/minecraft/voicechat/wiki (proximity/group semantics).
No host microphone/speaker access: only synthetic encoded audio and agents-null.
"""
from tools.harness.matrix_api import Adapter
from . import _platform as p


def svc_client_join_without_server(ctx):
    p.fixture(ctx,'svc_clients',server_bridge=False,audio='synthetic',sink='agents-null')
    p.rich_action(ctx,'reconnect_profile','fabric_svc')
    p.check(ctx,'ordinary game join',p.observe(ctx,'connection.phase'),'play')
    p.check(ctx,'voice unavailable',p.observe(ctx,'svc.connected'),False)
    p.client(ctx).chat('matrix-svc-no-server')
    ctx.wait_until(lambda:any('matrix-svc-no-server' in s for s in p.observe(ctx,'events')),description='chat without voice server')
    p.test(ctx,'chat independent of missing SVC',True,'server chat echo')


def proximity_voice(ctx):
    p.client(ctx,1)
    fixture=p.fixture(ctx,'svc_clients',server_bridge=True,audio='synthetic',sink='agents-null',max_distance=48)
    a=p.position(ctx)
    p.client(ctx,1).teleport_to(a[0]+4,a[1],a[2])
    p.rich_action(ctx,'svc_send_test_frame',fixture['pcm_marker'])
    near=p.observe(ctx,'svc.received_frames',1)
    p.test(ctx,'nearby voice delivered',any(f['marker']==fixture['pcm_marker'] for f in near),near)
    p.client(ctx,1).teleport_to(a[0]+64,a[1],a[2])
    before=len(p.observe(ctx,'svc.received_frames',1))
    p.rich_action(ctx,'svc_send_test_frame',fixture['pcm_marker']+'-far')
    p.rich_action(ctx,'svc_flush_frame_boundary')
    p.check(ctx,'outside radius voice suppressed',len(p.observe(ctx,'svc.received_frames',1)),before)


def group_voice(ctx):
    p.client(ctx,1)
    fixture=p.fixture(ctx,'svc_clients',server_bridge=True,audio='synthetic',sink='agents-null')
    group=p.rich_action(ctx,'svc_create_group','matrix-private',password='matrix26520')
    p.rich_action(ctx,'svc_join_group',group,password='matrix26520',index=1)
    a=p.position(ctx)
    p.client(ctx,1).teleport_to(a[0]+128,a[1],a[2])
    p.rich_action(ctx,'svc_send_test_frame',fixture['pcm_marker'])
    frames=p.observe(ctx,'svc.received_frames',1)
    p.test(ctx,'group bypasses distance',any(f['marker']==fixture['pcm_marker'] for f in frames),frames)
    if len(ctx.clients)>2:
        p.check(ctx,'nonmember does not hear group',any(f['marker']==fixture['pcm_marker'] for f in p.observe(ctx,'svc.received_frames',2)),False)
    p.rich_action(ctx,'svc_leave_group',index=1)
    before=len(p.observe(ctx,'svc.received_frames',1))
    p.rich_action(ctx,'svc_send_test_frame',fixture['pcm_marker']+'-left')
    p.rich_action(ctx,'svc_flush_frame_boundary')
    p.check(ctx,'former member no longer receives',len(p.observe(ctx,'svc.received_frames',1)),before)


def native_proximity_voice(ctx):
    p.client(ctx, 1)
    audio = p.fixture(ctx, 'native_voice', enabled=True, range_blocks=48,
                      source='synthetic_opus', sink='agents-null')
    p.rich_action(ctx, 'voice_listen', True)
    p.rich_action(ctx, 'voice_listen', True, index=1)
    origin = p.position(ctx)
    p.client(ctx, 1).teleport_to(origin[0]+10, origin[1], origin[2])
    p.rich_action(ctx, 'voice_send_audio', sequence=1, opus=audio['opus'])
    received = p.observe(ctx, 'voice.last_audio', 1)
    p.check(ctx, 'speaker UUID', received['speaker'], p.observe(ctx, 'uuid'))
    p.check(ctx, 'sequence preserved', received['sequence'], 1)
    p.check(ctx, 'audio preserved', received['opus'], audio['opus'])
    p.check(ctx, 'never loops back', p.observe(ctx, 'voice.audio_count'), 0)
    p.client(ctx, 1).teleport_to(origin[0]+60, origin[1], origin[2])
    before = p.observe(ctx, 'voice.audio_count', 1)
    p.rich_action(ctx, 'voice_send_audio', sequence=2, opus=audio['opus'])
    p.ticks(ctx, 2)
    p.check(ctx, 'outside range silent', p.observe(ctx, 'voice.audio_count', 1), before)
    p.rich_action(ctx, 'travel_dimension', 'minecraft:the_nether', index=1)
    p.client(ctx, 1).teleport_to(*origin)
    p.rich_action(ctx, 'voice_send_audio', sequence=3, opus=audio['opus'])
    p.ticks(ctx, 2)
    p.check(ctx, 'cross dimension silent', p.observe(ctx, 'voice.audio_count', 1), before)


def native_voice_listen_and_state(ctx):
    p.client(ctx, 1)
    for enabled in (True, False):
        audio = p.fixture(ctx, 'native_voice', enabled=enabled, range_blocks=48,
                          source='synthetic_opus', sink='agents-null')
        p.check(ctx, 'join state', p.observe(ctx, 'voice.state'), {'enabled': enabled, 'range_blocks': 48})
        p.check(ctx, 'one join state', p.observe(ctx, 'voice.state_count'), 1)
        for listening in (False, True, False):
            p.rich_action(ctx, 'voice_listen', listening, index=1)
            before = p.observe(ctx, 'voice.audio_count', 1)
            p.rich_action(ctx, 'voice_send_audio', sequence=10, opus=audio['opus'])
            p.ticks(ctx, 2)
            p.check(ctx, 'listen gate', p.observe(ctx, 'voice.audio_count', 1)-before,
                    int(enabled and listening))
        for payload in (bytes([9, 0]), bytes([1, 255]), bytes([1, 1, 2]),
                        bytes([1, 0])+bytes(4), bytes([1, 0])+bytes(1005)):
            p.rich_action(ctx, 'send_custom_payload', 'solaris:voice', payload)
        p.ticks(ctx, 2)
        p.check(ctx, 'malformed voice never kicks', p.observe(ctx, 'connection.phase'), 'play')


def voice_non_native_invisible(ctx):
    speaker = p.fixture(ctx, 'native_voice_speaker', source='synthetic_opus', sink='agents-null')
    p.client(ctx).teleport_to(*speaker['near_position'])
    p.method(ctx.server, 'fixture_action')('native_voice_speaker_send', sequence=1)
    p.ticks(ctx, 2)
    p.check(ctx, 'no native voice on nonnative',
            [r for r in p.observe(ctx, 'custom_payloads') if r['channel'] == 'solaris:voice'], [])
    p.check(ctx, 'nonnative play survives', p.observe(ctx, 'connection.phase'), 'play')


ADAPTERS={'voice.'+fn.__name__:Adapter(p.setup,fn,p.oracle) for fn in (
    svc_client_join_without_server,proximity_voice,group_voice,native_proximity_voice,
    native_voice_listen_and_state,voice_non_native_invisible)}
