"""Authoritative, physical Shift-double-click variants shared by both clients."""
from __future__ import annotations

import ctypes as c
from concurrent.futures import ThreadPoolExecutor
import time
from pathlib import Path
from typing import Any

from . import native_client
from .menu_observer import observe_menu
from .native_craft_drag import Pointer, center
from .native_v2 import _click, _echo, _open, _point, _set
from .probe_steps import ProbeSteps


def capture(steps: ProbeSteps, artifact: Path, label: str, java: bool) -> str:
    if java:
        from . import java_p5
        return str(java_p5.capture(steps, label))
    return str(native_client.capture(steps.client, artifact, label))


def slot_center(steps: ProbeSteps, state: dict[str, Any], slot: int, artifact: Path,
                java: bool) -> tuple[int, int]:
    if not java:
        return center(state, slot)
    from PIL import Image
    with Image.open(capture(steps, artifact, "shift-geometry", True)) as image:
        width, height = image.size
    scale = 2
    chest = state['container']['slot_count'] == 63
    left = (width - 176 * scale) // 2
    top = (height - (168 if chest else 166) * scale) // 2
    if chest:
        if slot < 27:
            row, col = divmod(slot, 9)
            return left + (16 + 18 * col) * scale, top + (26 + 18 * row) * scale
        if slot < 54:
            row, col = divmod(slot - 27, 9)
            return left + (16 + 18 * col) * scale, top + (94 + 18 * row) * scale
        return left + (16 + 18 * (slot - 54)) * scale, top + 152 * scale
    if 9 <= slot < 36:
        row, col = divmod(slot - 9, 9)
        return left + (16 + 18 * col) * scale, top + (92 + 18 * row) * scale
    if 36 <= slot < 45:
        return left + (16 + 18 * (slot - 36)) * scale, top + 150 * scale
    raise ValueError(f'unsupported player slot {slot}')


def _keys(pointer: Pointer) -> int:
    pointer.x.XKeysymToKeycode.argtypes = [c.c_void_p, c.c_ulong]
    pointer.x.XKeysymToKeycode.restype = c.c_ubyte
    pointer.xt.XTestFakeKeyEvent.argtypes = [c.c_void_p, c.c_uint, c.c_int, c.c_ulong]
    key = pointer.x.XKeysymToKeycode(pointer.display, 0xffe1)
    if not key:
        raise RuntimeError('owned Xvfb has no Shift_L key')
    return key


def _key(pointer: Pointer, key: int, down: bool) -> None:
    if not pointer.xt.XTestFakeKeyEvent(pointer.display, key, int(down), 0):
        raise RuntimeError('owned Xvfb refused key event')
    pointer.x.XSync(pointer.display, 0)


def _total(image: dict[str, Any]) -> int:
    slots = image['slots'][1:] if image['container_id'] == 0 else image['slots']
    return sum(stack['count'] for stack in [image['carried'], *slots]
               if stack and stack.get('item') == 'minecraft:stone')


def _slot_count(image: dict[str, Any], slot: int) -> int:
    return image['slots'][slot]['count']


def _fence_input(steps: ProbeSteps, before: int, label: str, java: bool) -> None:
    if java:
        steps.call('minecraft_wait_ticks', {'ticks': 2})
    else:
        steps.wait(lambda state: state['client_tick'] >= before + 2, label + '_input_processed')
    _echo(steps, label + '_ordered')


def _player_fixture(steps: ProbeSteps, label: str, *, named: bool = False) -> dict[str, Any]:
    if steps.observe()['screen']['open']:
        steps.call('minecraft_close_screen')
    steps.fixture('clear @s')
    for slot in range(9):
        item = 'minecraft:stone[custom_name={text:"Different"}]' if named and slot == 1 else 'minecraft:stone'
        steps.fixture(f'debug give {item} 64 {slot}')
    steps.fixture('give @s minecraft:stone 10')
    _echo(steps, label + '_fixture_ordered')
    steps.call('minecraft_open_inventory')
    state = steps.wait(lambda s: s['screen']['open'] and s['container']['container_id'] == 0
                       and s['container']['slot_count'] == 46, label + '_open')
    return state


def _click_pair(steps: ProbeSteps, state: dict[str, Any], artifact: Path, window: dict[str, Any],
                label: str, *, java: bool, outside: bool = False) -> float:
    before_tick = steps.observe().get('client_tick', 0)
    pointer = Pointer(window)
    key = _keys(pointer)
    try:
        pointer.move(*slot_center(steps, state, 9, artifact, java))
        _key(pointer, key, True)
        start = time.monotonic_ns()
        pointer.button(True)
        pointer.button(False)
        if outside:
            if java:
                steps.call('minecraft_wait_ticks', {'ticks': 8})
            else:
                before_tick = steps.wait(lambda s: s['client_tick'] >= before_tick + 8,
                                         label + '_outside_wait')['client_tick']
        pointer.button(True)
        elapsed = (time.monotonic_ns() - start) / 1_000_000
        pointer.button(False)
    finally:
        _key(pointer, key, False)
        pointer.close()
    steps.report.setdefault('shift_intervals_ms', {})[label] = elapsed
    steps.check(label + '_interval', elapsed >= 250 if outside else elapsed < 250)
    _fence_input(steps, before_tick, label, java)
    return elapsed


def run_different(steps: ProbeSteps, artifact: Path, window: dict[str, Any], *, java: bool = False) -> None:
    state = _player_fixture(steps, 'different', named=True)
    before = observe_menu(steps, 'different_before')
    steps.check('different_fixture', _total(before) == 586 and
                before['slots'][37]['components']['custom_name'] == 'Different')
    capture(steps, artifact, 'shift-different-before', java)
    _click_pair(steps, state, artifact, window, 'different', java=java)
    after = observe_menu(steps, 'different_after')
    capture(steps, artifact, 'shift-different-after', java)
    steps.check('different_conserved', _total(after) == 586)
    steps.check('different_component_not_moved', _slot_count(after, 37) == 64 and
                after['slots'][37]['components']['custom_name'] == 'Different')
    steps.check('different_matching_moved', _slot_count(after, 36) == 0)


def run(steps: ProbeSteps, origin: tuple[int, int, int], artifact: Path,
        window: dict[str, Any], *, java: bool = False) -> None:
    run_different(steps, artifact, window, java=java)

    state = _player_fixture(steps, 'outside')
    before = observe_menu(steps, 'outside_before')
    steps.check('outside_fixture', _total(before) == 586 and _slot_count(before, 9) == 10)
    capture(steps, artifact, 'shift-outside-before', java)
    _click_pair(steps, state, artifact, window, 'outside', java=java, outside=True)
    after = observe_menu(steps, 'outside_after')
    capture(steps, artifact, 'shift-outside-after', java)
    steps.check('outside_conserved', _total(after) == 586)
    steps.check('outside_no_bulk', _slot_count(after, 36) == 64 and _slot_count(after, 9) == 10)

    steps.call('minecraft_close_screen')
    steps.fixture('clear @s')
    for slot in range(9):
        steps.fixture(f'debug give minecraft:stone 64 {slot}')
    steps.fixture('give @s minecraft:stone 84')
    for slot in range(9):
        steps.fixture(f'debug give minecraft:stone 0 {slot}')
    _echo(steps, 'empty_second_fixture_ordered')
    steps.call('minecraft_open_inventory')
    state = steps.wait(lambda s: s['screen']['open'] and s['container']['container_id'] == 0,
                       'empty_second_open')
    before = observe_menu(steps, 'empty_second_before')
    steps.check('empty_second_fixture', _total(before) == 84 and
                _slot_count(before, 9) == 64 and _slot_count(before, 10) == 20 and
                all(_slot_count(before, i) == 0 for i in range(36, 45)))
    capture(steps, artifact, 'shift-empty-second-before', java)
    _click_pair(steps, state, artifact, window, 'empty_second', java=java)
    after = observe_menu(steps, 'empty_second_after')
    capture(steps, artifact, 'shift-empty-second-after', java)
    steps.check('empty_second_conserved', _total(after) == 84)
    steps.check('empty_second_no_bulk', _slot_count(after, 9) == 0 and
                _slot_count(after, 10) == 20 and _slot_count(after, 36) == 64)

    run_chest(steps, origin, artifact, window, java=java)
    _menu_close(steps, artifact, window, java)


def run_java(client: Any, artifact: Path, timeout: float) -> dict[str, Any]:
    from . import native_shift_double_click
    from .native_v2 import _prepare
    name = 'java-v2-p5-shift-double-click'
    with ProbeSteps(client, artifact / name, timeout, name) as steps:
        origin = _prepare(steps)
        native_shift_double_click.run(steps, artifact, client.p4_window, java=True)
        run(steps, origin, artifact, client.p4_window, java=True)
    return steps.report


def run_chest(steps: ProbeSteps, origin: tuple[int, int, int], artifact: Path,
              window: dict[str, Any], *, java: bool = False) -> None:
    if steps.observe()['screen']['open']:
        steps.call('minecraft_close_screen')
    steps.fixture('clear @s')
    point = _point(origin)
    _set(steps, point, 'minecraft:chest')
    steps.fixture('give @s minecraft:stone 10')
    _echo(steps, 'chest_stone_fixture')
    state = _open(steps, point, 'ChestMenu', 63)
    _click(steps, 54, quick=True)
    steps.fixture('give @s minecraft:stone 20')
    _echo(steps, 'chest_second_seed_ordered')
    _click(steps, 54)
    _click(steps, 1)
    _echo(steps, 'chest_seed_ordered')
    before_seed = observe_menu(steps, 'chest_seed', slots=63)
    steps.check('chest_seed_source', _slot_count(before_seed, 0) == 10 and _slot_count(before_seed, 1) == 20)
    steps.call('minecraft_close_screen')
    steps.fixture('give @s minecraft:stone 2304')
    _echo(steps, 'chest_full_fixture_ordered')
    state = _open(steps, point, 'ChestMenu', 63)
    before = observe_menu(steps, 'chest_before', slots=63)
    steps.check('chest_fixture_full', _slot_count(before, 0) == 10 and
                all(_slot_count(before, i) == 64 for i in range(27, 63)))
    capture(steps, artifact, 'shift-chest-before', java)
    total = _total(before)
    tick = steps.observe().get('client_tick', 0)
    pointer = Pointer(window)
    key = _keys(pointer)
    try:
        pointer.move(*slot_center(steps, state, 0, artifact, java))
        _key(pointer, key, True)
        start = time.monotonic_ns()
        pointer.button(True)
        pointer.button(False)
        pointer.button(True)
        elapsed = (time.monotonic_ns() - start) / 1_000_000
        pointer.button(False)
    finally:
        _key(pointer, key, False)
        pointer.close()
    steps.report.setdefault('shift_intervals_ms', {})['chest'] = elapsed
    steps.check('chest_interval', elapsed < 250)
    _fence_input(steps, tick, 'chest', java)
    after = observe_menu(steps, 'chest_after', slots=63)
    capture(steps, artifact, 'shift-chest-after', java)
    steps.check('chest_full_destination_no_move', _slot_count(after, 0) == 10)
    steps.check('chest_conserved', _total(after) == total)

    # Set up a nonempty correction target so the existing read-only exact-slot
    # waiter can be armed before either physical click.
    steps.fixture('debug give minecraft:stone 63 0')
    _echo(steps, 'correction_fixture_ordered')
    correction_before = observe_menu(steps, 'correction_before', slots=63)
    steps.check('correction_fixture', _slot_count(correction_before, 0) == 10
                and _slot_count(correction_before, 1) == 20
                and _slot_count(correction_before, 54) == 63
                and _total(correction_before) == total - 1)
    capture(steps, artifact, 'shift-correction-before', java)
    pointer = Pointer(window)
    key = _keys(pointer)
    wire = steps.client.correction_wire
    command = 'debug give minecraft:stone 1 0'
    steps.report['fixture_commands'].append(command + ' (ordered by correction wire)')
    wire.arm(command)
    try:
        pointer.move(*slot_center(steps, state, 0, artifact, java))
        with ThreadPoolExecutor(max_workers=1) as executor:
            observed = executor.submit(steps.call, 'minecraft_wait_for_container_slot', {
                'slot': 54, 'item_id': 'minecraft:stone', 'count': 1, 'timeout_seconds': 5.0,
            })
            _key(pointer, key, True)
            start = time.monotonic_ns()
            pointer.button(True)
            pointer.button(False)
            wire.wait_sent(min(steps.remaining(), 5))
            correction_received = observed.result(timeout=5)
            pointer.button(True)
            elapsed = (time.monotonic_ns() - start) / 1_000_000
            pointer.button(False)
        steps.check('correction_received_destination', correction_received.get('matched') is True
                    and correction_received.get('slot') == 54
                    and correction_received.get('item_id') == 'minecraft:stone'
                    and correction_received.get('count') == 1)
    finally:
        _key(pointer, key, False)
        pointer.close()
    steps.report.setdefault('shift_intervals_ms', {})['correction'] = elapsed
    steps.check('correction_interval', elapsed < 250)
    tick = steps.observe().get('client_tick', 0)
    _fence_input(steps, tick, 'correction', java)
    corrected = observe_menu(steps, 'correction_after', slots=63)
    capture(steps, artifact, 'shift-correction-after', java)
    steps.check('correction_conserved_after_declared_removal',
                _total(corrected) == total - 64)
    steps.check('correction_both_chest_stacks_moved',
                _slot_count(corrected, 0) == 0 and _slot_count(corrected, 1) == 0
                and _slot_count(corrected, 54) == 30)


def _menu_close(steps: ProbeSteps, artifact: Path, window: dict[str, Any], java: bool) -> None:
    state = _player_fixture(steps, 'close')
    before = observe_menu(steps, 'close_before')
    steps.check('close_fixture', _total(before) == 586 and _slot_count(before, 9) == 10)
    capture(steps, artifact, 'shift-close-before', java)
    tick = steps.observe().get('client_tick', 0)
    pointer = Pointer(window)
    key = _keys(pointer)
    steps.client.correction_wire.arm_close()
    try:
        pointer.move(*slot_center(steps, state, 9, artifact, java))
        _key(pointer, key, True)
        start = time.monotonic_ns()
        pointer.button(True)
        pointer.button(False)
        escape = pointer.x.XKeysymToKeycode(pointer.display, 0xff1b)
        inventory = pointer.x.XKeysymToKeycode(pointer.display, ord('e'))
        if not escape or not inventory:
            raise RuntimeError('owned Xvfb lacks Escape or inventory key')
        _key(pointer, escape, True)
        _key(pointer, escape, False)
        _key(pointer, inventory, True)
        _key(pointer, inventory, False)
        pointer.button(True)
        elapsed = (time.monotonic_ns() - start) / 1_000_000
        pointer.button(False)
    finally:
        _key(pointer, key, False)
        pointer.close()
    steps.report.setdefault('shift_intervals_ms', {})['close_reopen'] = elapsed
    steps.check('close_reopen_interval', elapsed < 250)
    steps.client.correction_wire.wait_close(min(steps.remaining(), 5))
    _fence_input(steps, tick, 'close', java)
    steps.wait(lambda s: s['screen']['open'] and s['container']['container_id'] == 0,
               'close_reopened')
    after = observe_menu(steps, 'close_after')
    capture(steps, artifact, 'shift-close-after', java)
    steps.check('close_conserved', _total(after) == 586)
    steps.check('close_history_not_reused', _slot_count(after, 36) == 64 and _slot_count(after, 9) == 10)
