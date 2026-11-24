#!/usr/bin/env python3
"""Generate `player_head_registry_scene.json` (PLAYER-HEAD-FAILED-PROFILE-REGISTRY-EXHAUSTION).

The scene is a data fixture with 261 distinct malformed head properties, so it is generated rather
than hand-maintained. Run from the repository root:

    python3 tools/harness/gen_player_head_registry_scene.py

The flood mirrors the HeadBreaker saturation reproducer (`receipts/HeadBreaker/REPORT-skull.md`):
nine visible hotbar slots carry distinct malformed `textures` values (plain non-base64 strings fail
the resolver), in 29 groups of nine. **Before** the flood one valid public texture is demanded and
worn (`pre-valid`): the same decoded Notch document re-serialized, so its registry key differs while
the fetch is the same. A run whose texture fetch is broken reds `pre_flood_valid_head_renders`,
which keeps the row's red from meaning "no network". After the flood two placed blocks are demanded
in the same fixed frame: an offline name that always renders the default skin, and the public Notch
texture property. A client that refuses the valid key at its full registry renders Steve for both.
"""

from __future__ import annotations

import json
from pathlib import Path

FLOOD = 261
# The exact unsigned public Notch property used by the HeadBreaker baseline and controls.
VALID = (
    "eyJ0ZXh0dXJlcyI6eyJTS0lOIjp7InVybCI6Imh0dHA6Ly90ZXh0dXJlcy5taW5lY3JhZnQubmV0L3RleHR1cmUv"
    "MjkyMDA5YTQ5MjViNThmMDJjNzdkYWRjM2VjZWYwN2VhNGM3NDcyZjY0ZTBmZGMzMmNlNTUyMjQ4OTM2MjY4MCJ9fX0="
)
# The same document with whitespace added: a distinct registry key (the key hashes the value bytes)
# whose fetch resolves the same texture.
POSITIVE = (
    "eyJ0ZXh0dXJlcyI6eyJTS0lOIjp7InVybCI6Imh0dHA6Ly90ZXh0dXJlcy5taW5lY3JhZnQubmV0L3RleHR1cmUv"
    "MjkyMDA5YTQ5MjViNThmMDJjNzdkYWRjM2VjZWYwN2VhNGM3NDcyZjY0ZTBmZGMzMmNlNTUyMjQ4OTM2MjY4MCJ9IH19"
)
SLOTS = 9


def steps() -> list[dict]:
    out: list[dict] = [
        {"wait_play": True},
        {"cmd": "gamerule send_command_feedback false"},
        {"cmd": "gamemode creative"},
        {"cmd": "effect give @s minecraft:slow_falling 600 0 true"},
        {"cmd": "gamerule spawn_mobs false"},
        {"cmd": "gamerule advance_time false"},
        {"cmd": "time set 6000"},
        {"tp": [1000.5, 160, 1000.5, 0, 0]},
        {"wait_play": True},
        {"cmd": "fill 995 159 995 1005 159 1010 minecraft:stone"},
        {"cmd": "fill 995 160 995 1005 165 1010 minecraft:air"},
        {"tp": [1000.5, 160, 1001.5, 0, 0]},
        {"wait_ticks": 30},
        # Third person, so the worn head is framed at the avatar's head for every shot below.
        {"key": {"keys": ["key.keyboard.f5"], "ticks": 1}},
        {"wait_ticks": 10},
        {"shot": "empty"},
        # Positive network control, in this run: a distinct key whose fetch is the same public
        # texture. A broken fetch leaves the avatar on the default skin here.
        {"cmd": "give @s player_head[profile={properties:[{name:\"textures\",value:\"%s\"}]}]"
                % POSITIVE},
        {"cmd": "item replace entity @s armor.head from entity @s weapon.mainhand"},
        {"wait_ticks": 200},
        {"shot": "pre-valid"},
        {"cmd": "clear @s"},
    ]
    for index in range(FLOOD):
        out.append({"cmd": "item replace entity @s hotbar.%d with minecraft:player_head"
                           "[profile={properties:[{name:\"textures\",value:\"bad%04d\"}]}]"
                           % (index % SLOTS, index)})
        if index % SLOTS == SLOTS - 1 or index == FLOOD - 1:
            out.append({"wait_ticks": 4})
            if index // SLOTS in (0, FLOOD // SLOTS - 1):
                out.append({"shot": f"flood-{index // SLOTS}"})
    out.extend([
        {"cmd": "clear @s"},
        # 261 distinct malformed properties are registered; these two worn heads are the next NEW
        # demands. The offline name always renders the default skin and is the same-frame reference.
        {"cmd": "give @s minecraft:player_head[profile={name:\"zzunresolved\"}]"},
        {"cmd": "item replace entity @s armor.head from entity @s weapon.mainhand"},
        {"wait_ticks": 20},
        {"shot": "flood-steve"},
        {"cmd": "clear @s"},
        # Unprefixed item id: the property value alone pushes the command to the 256-char limit.
        {"cmd": "give @s player_head[profile={properties:[{name:\"textures\",value:\"%s\"}]}]"
                % VALID},
        {"cmd": "item replace entity @s armor.head from entity @s weapon.mainhand"},
        {"wait_ticks": 200},
        {"shot": "flood-valid"},
        {"observe": "flood-valid-observe"},
    ])
    return out


def main() -> None:
    scene = {
        "seed": 100,
        "client_settings": {"render_distance": 8, "graphics_preset": "low", "fov": 70},
        "steps": steps(),
    }
    path = Path(__file__).with_name("player_head_registry_scene.json")
    path.write_text(json.dumps(scene, indent=2) + "\n")
    print(f"wrote {path} ({len(scene['steps'])} steps)")


if __name__ == "__main__":
    main()
