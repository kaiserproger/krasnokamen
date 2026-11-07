"""One executable matrix adapter contract shared by canonical harness drivers.

Adapters are ordinary Python functions, not a scenario language or runner.
Drivers apply context factors before setup, execute setup/act/oracle in order,
and fail closed if an observation or verb is unavailable.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Protocol, TypeVar

T = TypeVar('T')
Position = tuple[int, int, int]


class MissingCapability(RuntimeError):
    """The selected driver cannot execute or observe the requested scenario."""


@dataclass(frozen=True)
class Check:
    name: str
    passed: bool
    detail: Any = None


class MatrixClient(Protocol):
    """Synchronous input submission; ONLY received-state waits prove completion.

    snapshot minimum schema (unavailable values are None/absent):
    position: [x,y,z]; inventory: [{slot,item_id,count}]; cursor: stack|None;
    menu: {id,state_id,slots:[stack]}|None; health; food; events:[str].
    Stack item_id is a namespaced registry string. Empty slots are omitted.
    A menu includes its player-inventory suffix; never add both to a total.
    Driver-specific rich observations MAY add fields without inventing data.
    Input verbs return after packet submission unless explicitly documented
    as a received-state wait. Domains MUST fence command/chat/actions on a
    fresh feedback event or authoritative observation before ctx.boundary().
    An unavailable optional verb raises MissingCapability, never a no-op.
    """
    name: str
    def snapshot(self) -> dict[str, Any]: ...
    def command(self, text: str) -> None: ...
    def chat(self, text: str) -> None: ...
    def teleport_to(self, x: float, y: float, z: float) -> None: ...
    def look_at(self, x: float, y: float, z: float) -> None: ...
    def use_item_on(self, position: Position, face: str = 'up', hand: str = 'main',
                    cursor: tuple[float, float, float] = (0.5, 0.5, 0.5)) -> int | None: ...
    def attack(self, entity_id: int) -> None: ...
    def click_slot(self, slot: int, kind: str = 'pickup', button: int = 0) -> None: ...
    def open_container(self, position: Position) -> None: ...
    def drop(self, all: bool = False) -> None: ...
    def select_hotbar(self, slot: int) -> None: ...
    def disconnect(self) -> None: ...
    def reconnect(self) -> None: ...
    def set_auto_auth(self, enabled: bool) -> None: ...
    def wait_until(self, predicate: Callable[[], T], *, description: str,
                   timeout_seconds: float = 30.0) -> T: ...
    def close_container(self) -> None: ...
    def open_inventory(self) -> None: ...
    def break_block(self, position: Position, face: str = 'up', phase: str = 'complete') -> int | None: ...
    def use_item(self, hand: str = 'main', ticks: int = 1) -> None: ...
    def release_use_item(self) -> None: ...
    def swap_hands(self) -> None: ...
    def respawn(self) -> None: ...
    def interact_entity(self, entity_id: int, hand: str = 'main', secondary: bool = False,
                        location: tuple[float, float, float] | None = None) -> None: ...
    def move_to(self, x: float, y: float, z: float, on_ground: bool = True) -> None: ...
    def move(self, forward: float = 0, strafe: float = 0, ticks: int = 20,
             sprint: bool = False) -> None: ...
    def press_inputs(self, *, forward: bool = False, backward: bool = False,
                     left: bool = False, right: bool = False, jump: bool = False,
                     sneak: bool = False, sprint: bool = False, ticks: int = 1) -> None: ...
    def jump(self, ticks: int = 1) -> None: ...
    def sneak(self, enabled: bool) -> None: ...
    def sprint(self, enabled: bool) -> None: ...
    def wait_ticks(self, ticks: int) -> None: ...
    def update_sign(self, position: Position, lines: list[str], front: bool = True) -> None: ...
    def edit_book(self, slot: int, pages: list[str], title: str | None = None) -> None: ...
    def rename_item(self, text: str) -> None: ...
    def container_button(self, button: int) -> None: ...
    def select_trade(self, index: int) -> None: ...
    def set_beacon(self, primary: str | None, secondary: str | None = None) -> None: ...
    def toggle_crafter_slot(self, slot: int, disabled: bool) -> None: ...
    def set_creative_slot(self, slot: int, item_id: str, count: int,
                          components: dict | None = None) -> None: ...
    def place_recipe(self, recipe_id: str, all: bool = False) -> None: ...
    def stale_click(self, slot: int, kind: str = 'pickup', button: int = 0,
                    *, state_id: int) -> None: ...
    def set_client_information(self, view_distance: int | None = None,
                                locale: str | None = None) -> None: ...
    def configure_chat(self, visibility: str) -> None: ...
    def read_block_state(self, position: Position) -> dict[str, Any]: ...


class MatrixServer(Protocol):
    def command(self, text: str) -> str: ...
    def save_all(self) -> str: ...
    def kill9_restart(self) -> dict[str, Any]: ...
    def hot_backup(self) -> Path: ...
    def query(self, text: str) -> str: ...
    def dashboard_stats(self) -> dict[str, Any]: ...
    def clean_restart(self) -> dict[str, Any]: ...
    def restore_proof(self, backup: Path) -> dict[str, Any]: ...
    def ensure_loaded(self, area: tuple[Position, Position]) -> None: ...
    def reconfigure(self, overrides: dict[str, dict], plugins_dir: Path | None = None) -> None: ...
    def dashboard_full_stats(self) -> dict[str, Any]: ...


@dataclass
class MatrixContext:
    row: dict[str, Any]
    server: MatrixServer
    clients: list[MatrixClient]
    artifact_dir: Path
    state: dict[str, Any] = field(default_factory=dict)
    boundary_hook: Callable[['MatrixContext'], None] | None = None
    _boundary_called: bool = field(default=False, init=False)
    concurrent_hook: Callable[..., list[dict]] | None = None
    _actor_targets: set[str] = field(default_factory=set, init=False)
    _same_tick_actor_targets: set[str] = field(default_factory=set, init=False)

    def concurrently(self, actions: list[tuple[MatrixClient, str, dict]], *,
                     target: str | None = None, same_tick: bool = False) -> list[dict]:
        """Coordinate release and prove exact consumed input for every actor.

        Ordinary concurrency may span ticks. Explicit dupe/same-block triples
        additionally request same_tick=True; a wall-time barrier alone proves neither.
        """
        if target not in {None, 'own', 'shared'}:
            raise ValueError('concurrent target must be own or shared')
        if type(same_tick) is not bool:
            raise TypeError('same_tick must be bool')
        if self.concurrent_hook is None:
            raise MissingCapability('same_tick_admission_fence')
        receipts = self.concurrent_hook(self, actions, same_tick=same_tick)
        import json
        if (not isinstance(receipts, list) or len(receipts) != len(actions)
                or not all(isinstance(receipt, dict) for receipt in receipts)):
            raise MissingCapability('same_tick_admission_receipts')
        for receipt, (client, verb, _) in zip(receipts, actions):
            if (receipt.get('client') != client.name or receipt.get('verb') != verb
                    or type(receipt.get('tick')) is not int or 'input' not in receipt):
                raise MissingCapability('same_tick_admission_receipts')
        if same_tick and len({receipt['tick'] for receipt in receipts}) != 1:
            raise MissingCapability('same_tick_admission_split')
        if target is not None:
            expected = {client.name for client in self.clients}
            actual = [receipt['client'] for receipt in receipts]
            if len(actual) != len(expected) or set(actual) != expected:
                raise MissingCapability('concurrent_subject_actors')
            # A domain label only counts when it accompanies the hook's actual
            # all-actor tick receipts. Arbitrary public-state flags cannot pass.
            self._actor_targets.add(target)
            if same_tick:
                self._same_tick_actor_targets.add(target)
            self.state.setdefault('concurrent_subject_actors', {})[target] = json.loads(
                json.dumps(receipts, allow_nan=False))
        self.state.setdefault('concurrent_receipts', []).append(receipts)
        return receipts

    def boundary(self) -> None:
        """Apply context network/lifecycle after first acknowledged action step.

        Adapters call this exactly once at their semantic mid-action boundary.
        Driver reconnects retain client wrapper identities. A steady/normal
        context has no effect; all other contexts require a real driver hook.
        """
        if self._boundary_called:
            raise RuntimeError('adapter invoked action boundary more than once')
        self._boundary_called = True
        if self.boundary_hook is not None:
            self.boundary_hook(self)
        elif self.factors['network_condition'] != 'normal' or self.factors['server_lifecycle'] != 'steady':
            raise MissingCapability('action_boundary:' + self.factors['network_condition']
                                    + ':' + self.factors['server_lifecycle'])

    @property
    def factors(self) -> dict[str, str]:
        return self.row['context']

    def wait_until(self, predicate: Callable[[], T], *, description: str,
                   timeout_seconds: float = 30.0) -> T:
        if not self.clients:
            raise MissingCapability('event_wait:no_client; use server log/console event fence')
        return self.clients[0].wait_until(predicate, description=description,
                                          timeout_seconds=timeout_seconds)


@dataclass(frozen=True)
class Adapter:
    setup: Callable[[MatrixContext], None]
    act: Callable[[MatrixContext], None]
    oracle: Callable[[MatrixContext], list[Check]]
    exclude_plugins: tuple[str, ...] = ()


def execute_adapter(adapter: Adapter, context: MatrixContext) -> list[Check]:
    """Shared fail-closed execution primitive; cmd_run owns lifecycle/receipts."""
    import json
    adapter.setup(context)
    try:
        json.dumps(context.state, allow_nan=False)
    except (TypeError, ValueError) as error:
        raise TypeError('adapter setup state must be JSON-serializable: ' + str(error)) from error
    adapter.act(context)
    if (context.factors['network_condition'] != 'normal'
            or context.factors['server_lifecycle'] != 'steady') and not context._boundary_called:
        raise MissingCapability('adapter_action_boundary:' + context.row['subject'])
    checks = adapter.oracle(context)
    if not isinstance(checks, list) or not checks:
        raise RuntimeError('adapter oracle must return a non-empty list of Check')
    if not all(isinstance(check, Check) and type(check.passed) is bool for check in checks):
        raise TypeError('adapter oracle returned invalid Check values')
    try:
        json.dumps({'state': context.state, 'checks': [check.__dict__ for check in checks]}, allow_nan=False)
    except (TypeError, ValueError) as error:
        raise TypeError('adapter state/check details must be JSON-serializable: ' + str(error)) from error
    return checks


def load_adapter(subject: str) -> Adapter:
    """Only trusted domain modules; no import paths or code in matrix YAML."""
    import importlib
    domain = subject.split('.', 1)[0]
    if not domain.isidentifier() or domain.startswith('_'):
        raise ValueError('invalid subject domain: ' + domain)
    module_name = f'tools.harness.matrix_domains.{domain}'
    try:
        module = importlib.import_module(module_name)
    except ModuleNotFoundError as error:
        if error.name != module_name:
            raise
        raise MissingCapability('domain_adapter:' + subject) from error
    adapter = module.ADAPTERS.get(subject)
    if not isinstance(adapter, Adapter):
        raise MissingCapability('domain_adapter:' + subject)
    return adapter
