"""Ledger operator selectors: a player's own `/deop @s` really removes the level.

The row `OP-DEOP-SELECTOR-LITERAL` says `/op` and `/deop` treat a selector such as
`@s` as a literal player name.  The observable consequence is that the player's own
operator level does not change: a level-2 command is still accepted afterwards.
This scenario drives every command from the player's chat and reads the received
chat answers (the client does not expose its own game mode through the harness
observation, so the answer text is the evidence).
"""
from __future__ import annotations

from typing import Any

from .client_probes import probe_username
from .probe_steps import ProbeSteps

REFUSAL = "Unknown or incomplete command"


def _chat(steps: ProbeSteps, label: str, *needles: str, timeout_seconds: float = 30) -> dict[str, Any]:
    def hit(state: dict[str, Any]) -> bool:
        return any(all(needle in line for needle in needles) for line in state["recent_chat"])

    return steps.wait(hit, label, timeout_seconds=timeout_seconds)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    name = probe_username("entities")
    steps.fixture("gamerule send_command_feedback true")
    steps.fixture(f"gamemode survival {name}")
    steps.wait(lambda state: state["player"]["on_ground"], "player_ready")

    # Positive control: while the player is an operator a level-2 command answers.
    # (A no-op mode change answers nothing, so the control changes survival -> adventure.)
    steps.call("minecraft_send_chat", {"message": "gamemode adventure", "command": True})
    _chat(steps, "operator_level_command_accepted_before_deop",
          "Set own game mode to Adventure Mode")

    # An empty selector is not a player: vanilla answers the no-target error.
    steps.call("minecraft_send_chat", {"message": "deop @a[name=Missing]", "command": True})
    _chat(steps, "empty_selector_is_not_a_player", "No player was found")

    # The player's own selector resolves to the player and removes the level.
    steps.call("minecraft_send_chat", {"message": "deop @s", "command": True})
    _chat(steps, "self_deop_reports_completion", "Command deop completed for @s")

    # The level really went away: the same command is refused instead of answering.
    steps.call("minecraft_send_chat", {"message": "gamemode creative", "command": True})
    _chat(steps, "self_deop_removed_the_operator_level", REFUSAL)
    steps.report["after_self_deop"] = steps.observe()["recent_chat"][-4:]

    steps.fixture(f"op {name}")
    steps.report["limits"] = [
        "One operator and the self selector; the answer text is the evidence because the harness "
        "observation exposes no game mode; quoting and multi-target forms rest on the mc-edge "
        "profile-command refs.",
    ]
