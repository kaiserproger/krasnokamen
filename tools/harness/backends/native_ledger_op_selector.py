"""Ledger operator selectors: a player's own `/deop @s` really removes the level.

The row `OP-DEOP-SELECTOR-LITERAL` says `/op` and `/deop` treat a selector such as
`@s` as a literal player name.  The observable consequence is that the player's own
operator level does not change: a level command afterwards is still accepted.  This
scenario drives the commands from the player's chat and reads the received chat
lines and the client's own received game mode.
"""
from __future__ import annotations

from typing import Any

from .client_probes import probe_username
from .probe_steps import ProbeSteps


def _chat(steps: ProbeSteps, label: str, *needles: str, timeout_seconds: float = 30) -> dict[str, Any]:
    def hit(state: dict[str, Any]) -> bool:
        return any(all(needle in line for needle in needles) for line in state["recent_chat"])

    return steps.wait(hit, label, timeout_seconds=timeout_seconds)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    name = probe_username("entities")
    steps.fixture("gamerule send_command_feedback true")
    steps.fixture(f"gamemode survival {name}")
    steps.wait(lambda state: state["player"]["game_mode"] == 0, "player_starts_in_survival")

    # An empty selector is not a player: vanilla answers the no-target error.
    steps.call("minecraft_send_chat", {"message": "deop @a[name=Missing]", "command": True})
    _chat(steps, "empty_selector_is_not_a_player", "No player was found")

    # The player's own selector resolves to the player and removes the level.
    steps.call("minecraft_send_chat", {"message": "deop @s", "command": True})
    _chat(steps, "self_deop_reports_completion", "Command deop completed for @s")

    # A level-2 command is no longer accepted, so the client's own mode stays survival.
    steps.call("minecraft_send_chat", {"message": "gamemode creative", "command": True})
    steps.call("minecraft_wait_ticks", {"ticks": 20})
    state = steps.observe()
    steps.report["after_self_deop"] = {
        "game_mode": state["player"]["game_mode"],
        "recent_chat": state["recent_chat"][-4:],
    }
    steps.check("self_deop_removed_the_operator_level", state["player"]["game_mode"] == 0)

    steps.fixture(f"op {name}")
    steps.report["limits"] = [
        "One operator and the self selector; quoting and multi-target forms rest on the mc-edge "
        "profile-command refs.",
    ]
