"""Krasnokamen extensions, NOT vanilla features. Source: docs/decisions/0018-in-game-computer.md.
The matrix's screen-share-is-forbidden oracle predates ADR lines 82 onward.
Only isolated synthetic Xvfb content is authorized; never capture the host desktop.
"""
from tools.harness.matrix_api import Adapter
from . import _platform as p


def carrier(ctx, enabled=True, backend='echo'):
    p.fixture(ctx, 'computer', enabled=enabled, local_only=True, backend=backend)
    x, y, z = p.arena(ctx)
    pos = (x+2, y, z)
    p.admin(ctx, f'setblock {pos[0]} {pos[1]} {pos[2]} minecraft:iron_block')
    return pos


def terminal_open_op4(ctx):
    pos = carrier(ctx)
    p.use(ctx, pos)
    opened = p.rich_action(ctx, 'computer_open', pos, session=1)
    p.check(ctx, 'open session reply', opened, {'session': 1, 'kind': 'opened'})
    reply = p.rich_action(ctx, 'computer_input', 1, 'matrix-echo-26520')
    p.check(ctx, 'echo output session', reply['session'], 1)
    p.test(ctx, 'echo bytes', 'matrix-echo-26520' in reply['text'], reply)
    p.block_is(ctx, pos, 'minecraft:iron_block')


def terminal_refusals(ctx):
    pos = carrier(ctx)
    # Separate fixture sessions exercise security identities without replacing row actor factors.
    for reason in ('op_3', 'auth_locked', 'out_of_reach', 'wrong_dimension'):
        reply = p.rich_action(ctx, 'computer_refusal_probe', pos, identity=reason)
        p.check(ctx, reason + ' refused', reply['kind'], 'closed')
        p.check(ctx, reason + ' no process', reply['spawned_children'], 0)
    p.block_is(ctx, pos, 'minecraft:iron_block')


def terminal_session_close(ctx):
    for revoke in ('break_block', 'leave_reach', 'deop', 'disconnect', 'dimension_change'):
        pos = carrier(ctx, backend='harness_line_echo')
        opened = p.rich_action(ctx, 'computer_open', pos, session=2)
        p.check(ctx, revoke + ' opened', opened['kind'], 'opened')
        # This driver verb performs exactly one authorized in-world/session revocation.
        p.rich_action(ctx, 'computer_revoke_session', revoke, pos)
        result = p.observe(ctx, 'computer.session')
        p.check(ctx, revoke + ' session closed', result['closed'], True)
        p.check(ctx, revoke + ' direct child reaped', result['child_exited'], True)
        p.check(ctx, revoke + ' no stale output', result['output_after_close'], [])


def terminal_disabled_vanilla_iron(ctx):
    pos = carrier(ctx, enabled=False)
    before = p.observe(ctx, 'custom_payloads')
    p.use(ctx, pos)
    p.ticks(ctx, 2)
    p.block_is(ctx, pos, 'minecraft:iron_block')
    p.check(ctx, 'no terminal', p.observe(ctx, 'screen.kind'), 'game')
    p.check(ctx, 'no computer replies', [r for r in p.observe(ctx, 'custom_payloads')[len(before):]
                                       if r['channel'] == 'krasnokamen:computer'], [])


def desktop_capture_local(ctx):
    isolated = p.fixture(ctx, 'synthetic_xvfb_window', pattern='matrix-26520', host_capture=False,
                         environment={'WAYLAND_DISPLAY': None, 'PIPEWIRE_NODE': 'agents-null'})
    p.rich_action(ctx, 'local_capture_consent', isolated['window_id'], allow=False)
    p.check(ctx, 'denial captures no frames', p.observe(ctx, 'capture.frame_count'), 0)
    p.rich_action(ctx, 'local_capture_consent', isolated['window_id'], allow=True)
    p.check(ctx, 'isolated window texture', p.rich_action(ctx, 'capture_texture_marker'), 'matrix-26520')
    p.check(ctx, 'terminal capture uploads', p.observe(ctx, 'capture.server_bytes'), 0)
    p.rich_action(ctx, 'local_capture_stop')
    p.check(ctx, 'capture released', p.observe(ctx, 'capture.running'), False)


def screen_share_to_others(ctx):
    p.client(ctx, 1)
    monitor = p.fixture(ctx, 'monitor_row', width=2, height=1, source='synthetic_xvfb', host_capture=False)
    p.rich_action(ctx, 'screen_share_open', monitor['id'])
    p.check(ctx, 'default viewers get no frames', p.observe(ctx, 'screen_share.frame_count', 1), 0)
    p.rich_action(ctx, 'screen_share_request', monitor['id'], index=1)
    p.rich_action(ctx, 'screen_share_answer', p.observe(ctx, 'screen_share.request_id'), False)
    p.check(ctx, 'denied viewers get no frames', p.observe(ctx, 'screen_share.frame_count', 1), 0)
    p.rich_action(ctx, 'screen_share_request', monitor['id'], index=1)
    p.rich_action(ctx, 'screen_share_answer', p.observe(ctx, 'screen_share.request_id'), True)
    p.client(ctx, 1).look_at(*monitor['center'])
    p.check(ctx, 'consented viewer receives pattern', p.rich_action(ctx, 'screen_share_marker', index=1), monitor['marker'])
    p.rich_action(ctx, 'screen_share_revoke', monitor['id'], p.observe(ctx, 'uuid', 1))
    p.check(ctx, 'revocation stops frames', p.observe(ctx, 'screen_share.stopped', 1), True)
    p.check(ctx, 'viewer cannot control owner input', p.observe(ctx, 'screen_share.input_forwarded'), 0)


def screen_share_refusals(ctx):
    p.client(ctx, 1)
    monitor = p.fixture(ctx, 'monitor_row', width=1, height=1, source='synthetic_xvfb', host_capture=False)
    for case, reason in [('non_monitor', 1), ('out_of_reach', 2), ('fifth_screen', 3),
                         ('taken_rectangle', 4), ('oversized_frame', 5), ('repeated_unsolicited', 7)]:
        # Packet construction belongs to the native channel driver; the expected
        # refusal and connection survival are independently checked here.
        reply = p.rich_action(ctx, 'screen_share_send_invalid', monitor['id'], case)
        p.check(ctx, case + ' reason', reply['reason'], reason)
        p.check(ctx, case + ' no disconnect', p.observe(ctx, 'connection.phase'), 'play')
    p.rich_action(ctx, 'screen_share_open', monitor['id'])
    p.rich_action(ctx, 'break_block', tuple(monitor['anchor']))
    p.check(ctx, 'removed monitor closes show', p.observe(ctx, 'screen_share.active'), False)
    p.check(ctx, 'block removal does not kick', p.observe(ctx, 'connection.phase'), 'play')


def screen_share_audio(ctx):
    p.client(ctx, 1)
    monitor = p.fixture(ctx, 'monitor_row', width=1, height=1, source='synthetic_xvfb', host_capture=False,
                        audio='synthetic_opus', sink='agents-null')
    p.rich_action(ctx, 'screen_share_open', monitor['id'])
    p.rich_action(ctx, 'screen_share_request', monitor['id'], index=1)
    p.rich_action(ctx, 'screen_share_answer', p.observe(ctx, 'screen_share.request_id'), True)
    p.client(ctx, 1).look_at(*monitor['center'])
    p.rich_action(ctx, 'screen_share_send_audio', monitor['id'], sequence=1, marker='matrix-tone')
    p.check(ctx, 'granted looking viewer audio', p.observe(ctx, 'screen_share.last_audio', 1),
            {'id': monitor['id'], 'sequence': 1, 'marker': 'matrix-tone'})
    center = monitor['center']
    p.client(ctx, 1).look_at(center[0], center[1]+50, center[2])
    p.ticks(ctx, 1)
    before = p.observe(ctx, 'screen_share.audio_count', 1)
    p.rich_action(ctx, 'screen_share_send_audio', monitor['id'], sequence=2, marker='matrix-hidden-tone')
    p.ticks(ctx, 1)
    p.check(ctx, 'looking away stops audio', p.observe(ctx, 'screen_share.audio_count', 1), before)
    if len(ctx.clients) > 2:
        p.check(ctx, 'ungranted observer no audio', p.observe(ctx, 'screen_share.audio_count', 2), 0)
    p.rich_action(ctx, 'screen_share_send_audio_burst', monitor['id'], count=200)
    p.test(ctx, 'token bucket drops excess', p.observe(ctx, 'screen_share.audio_dropped') > 0,
           p.observe(ctx, 'screen_share.audio_dropped'))


def screen_share_non_native_clients(ctx):
    monitor = p.fixture(ctx, 'native_screen_owner', source='synthetic_xvfb', host_capture=False)
    p.client(ctx).teleport_to(*monitor['viewer_position'])
    p.client(ctx).look_at(*monitor['center'])
    p.ticks(ctx, 2)
    p.check(ctx, 'non-native receives no screen channel',
            [r for r in p.observe(ctx, 'custom_payloads') if r['channel'] == 'krasnokamen:screen'], [])
    p.check(ctx, 'non-native play unaffected', p.observe(ctx, 'connection.phase'), 'play')
    p.check(ctx, 'non-native sees carrier', p.rich_action(ctx, 'block_model', monitor['anchor']),
            'minecraft:black_glazed_terracotta')


ADAPTERS = {'computer.' + fn.__name__: Adapter(p.setup, fn, p.oracle) for fn in (
    terminal_open_op4, terminal_refusals, terminal_session_close, terminal_disabled_vanilla_iron,
    desktop_capture_local, screen_share_to_others, screen_share_refusals,
    screen_share_audio, screen_share_non_native_clients)}
