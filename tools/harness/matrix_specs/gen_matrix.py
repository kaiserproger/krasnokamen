#!/usr/bin/env python3
"""Scenario matrix generator, full-set queue and run ledger (read-only on all repos).

  python3 gen_matrix.py                      regenerate matrix.yaml + summary.md (wave 1) and full-set counts
  python3 gen_matrix.py --full [filters]     stream scenario ids lazily in priority order (wave 1, then waves 2..6)
         filters: --wave 1,2  --client a,b  --driver protocol_bot  --domain containers  --pending  --limit N
  python3 gen_matrix.py --show ID            print one scenario (wave-1 or F: id) as JSON: steps/oracle/driver/coverage
  python3 gen_matrix.py --status             ledger progress: executed/remaining for pairwise/baseline/triples/waves/full set
  python3 gen_matrix.py --record --id ID --status pass|fail|blocked|skipped --driver D --server-sha SHA --run-dir DIR [--plugin-set FP] [--note T] [--ts T]
  python3 gen_matrix.py --record-file lines.jsonl   validate and append a batch under one lock
  python3 gen_matrix.py --plugin-fingerprint DIR    plugin-set fingerprint of a deployed [plugins].directory (ledger plugin_set)
  python3 gen_matrix.py --refreeze           regenerate accepting a changed wave-1 id set (rewrites MATRIX_FREEZE.json)

Ids: CELL = subject/client_kind/game_mode/dimension/difficulty/player_count/network_condition/server_lifecycle/permission,
plus @vanilla|@live for plugin-sensitive subjects (PLUGIN_SENSITIVE: their oracle depends on enabled live plugins);
wave-1 id = <pw|bl|tr.<triple origin>>:CELL; full-set id = F:CELL. Ids are content-derived and stable.
Ledger lines for @profile ids must carry plugin_set = the fingerprint of the plugin dir actually deployed.
MATRIX_FREEZE.json pins the wave-1 id set; a plain regenerate refuses to change it (text-only corrections pass).
This file selects and describes scenarios; execution stays in tools/harness (cmd_run), which writes ledger lines.
Deterministic (seed 20261024); reads ../solaris, ../solaris-default-plugins, ../solaris-loader, ../solaris-client.
"""
import argparse
import fcntl
import functools
import hashlib
import itertools
import json
import os
import random
import re
import subprocess
import sys
import tomllib
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

import yaml

LEDGER_KEYS = ("id", "status", "driver", "server_sha", "run_dir", "ts", "note")
LEDGER_OPTIONAL_KEYS = ("plugin_set",)  # required for @profile ids
LEDGER_STATUS = ("pass", "fail", "blocked", "skipped")


def parse_args():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--full", action="store_true", help="stream ids lazily in priority order")
    mode.add_argument("--show", metavar="ID", help="print one scenario as JSON")
    mode.add_argument("--record", action="store_true", help="append one ledger line (flock + O_APPEND + fsync)")
    mode.add_argument("--record-file", metavar="JSONL", help="validate and append a batch of ledger lines")
    mode.add_argument("--plugin-fingerprint", metavar="DIR", help="print the plugin-set fingerprint of a deployed plugins directory")
    p.add_argument("--status", nargs="?", const="__report__", metavar="STATUS",
                   help="alone: ledger progress report; with --record: the ledger status value")
    p.add_argument("--wave", help="comma list of waves (1..6) for --full")
    p.add_argument("--client", help="comma list of client_kind values for --full")
    p.add_argument("--driver", help="--full: comma list of driver kinds; --record: the driver kind")
    p.add_argument("--domain", help="comma list of domains for --full")
    p.add_argument("--pending", action="store_true", help="--full: skip ids whose latest ledger status is pass|fail")
    p.add_argument("--limit", type=int, help="--full: stop after N ids")
    p.add_argument("--id")
    p.add_argument("--server-sha")
    p.add_argument("--run-dir")
    p.add_argument("--ts", help="UTC ISO-8601 ending in Z (default: now)")
    p.add_argument("--note", default="")
    p.add_argument("--plugin-set", help="--record: plugin-set fingerprint (gen_matrix.py --plugin-fingerprint DIR); required for @profile ids")
    p.add_argument("--refreeze", action="store_true", help="generate: accept a changed wave-1 id set and rewrite MATRIX_FREEZE.json")
    a = p.parse_args()
    if a.record:
        a.mode = "record"
        if a.status not in LEDGER_STATUS:
            p.error(f"--record needs --status {'|'.join(LEDGER_STATUS)}")
        if not (a.id and a.driver and a.server_sha and a.run_dir):
            p.error("--record needs --id --driver --server-sha --run-dir")
    elif a.status not in (None, "__report__"):
        p.error("--status VALUE is only valid with --record")
    elif a.record_file:
        a.mode = "record_file"
    elif a.plugin_fingerprint:
        a.mode = "fingerprint"
    elif a.full:
        a.mode = "full"
    elif a.show:
        a.mode = "show"
    elif a.status == "__report__":
        a.mode = "status"
    else:
        a.mode = "generate"
    if a.refreeze and a.mode != "generate":
        p.error("--refreeze only applies to generation")
    if a.status == "__report__" and a.mode != "status":
        p.error("--status (report) cannot be combined with another mode")
    return a


ARGS = parse_args()

import subjects
import subjects2  # noqa: F401  (registers domains)
import subjects3  # noqa: F401
import subjects4  # noqa: F401
import codegen_subjects as cg
from existing import E2E

OUT = Path(__file__).resolve().parent
SOL = Path(__file__).resolve().parents[3]
HOME = Path(os.environ.get("SOLARIS_WORK_ROOT", "/run/media/kaiserroman/games/solaris-work"))
RUN_DATA = Path(os.environ.get("SOLARIS_MATRIX_DATA", HOME / "scenarios"))
PREFIX = {"core:": "crates/mc-core/src/", "edge:": "crates/mc-edge/src/", "script:": "crates/mc-script/src/",
          "dp:": "../solaris-default-plugins/", "ldr:": "../solaris-loader/"}

# ------------------------------------------------------------------ context dimensions
CLIENTS = {
    "vanilla_java_26_1_2": {"driver": "java_mcp", "desc": "Mojang Java 26.1.2 client driven by ../solaris-loader java-agent bridge MCP (profiles core-client/inventory/survival/entities/regression)"},
    "native_linux": {"driver": "native_mcp", "desc": "../solaris-client native Rust client on Linux, built-in MCP automation (src/automation, 50 tools)"},
    "native_windows": {"driver": "native_mcp", "desc": "../solaris-client on Windows x86-64 (COMPATIBILITY_MATRIX PLAT-WINDOWS); today only manual VNC (.analysis/qa-windows/vnc.sh)", "extra_caps": ["windows_mcp_lane"]},
    "fabric_solaris_loader": {"driver": "loader_java_mcp", "desc": "Fabric + Solaris Loader (harness loader-live --platform fabric)"},
    "neoforge_solaris_loader": {"driver": "loader_java_mcp", "desc": "NeoForge + Solaris Loader (loader-live --platform neoforge)", "extra_caps": ["loader_platform_neoforge_forge"]},
    "forge_solaris_loader": {"driver": "loader_java_mcp", "desc": "Forge + Solaris Loader (loader-live --platform forge)", "extra_caps": ["loader_platform_neoforge_forge"]},
    "fabric_svc": {"driver": "loader_java_mcp", "desc": "Fabric + Simple Voice Chat mod (no server-side SVC in Solaris)", "extra_caps": ["svc_client_profile"]},
    "fabric_distant_horizons": {"driver": "loader_java_mcp", "desc": "Fabric/NeoForge + Distant Horizons 3.3.3 protocol 16 (harness dh-gate)"},
    "protocol_bot": {"driver": "protocol_bot", "desc": "Rust protocol bots: mc-test-harness player_scale_bots (--mechanic-spec), wire_probe, quirk_probe; plugin auth_live_check"},
}
LOADER_CLIENTS = ["fabric_solaris_loader", "neoforge_solaris_loader", "forge_solaris_loader", "native_linux", "native_windows", "protocol_bot"]
GAME_MODES = ["survival", "creative", "adventure", "spectator"]
DIMS = ["overworld", "the_nether", "the_end"]
DIFFS = ["peaceful", "easy", "normal", "hard"]
PLAYERS = ["1", "2", "N8"]
NETWORK = {
    "normal": "uninterrupted connection",
    "reconnect_mid_action": "actor disconnects after the first state-changing input of the action and reconnects before it completes",
    "disconnect_during_menu": "actor disconnects (TCP close) while a container/merchant/station menu is open, optionally carrying a cursor stack",
    "duplicate_login": "a second connection with the same account/UUID logs in during the action",
    "slow_reader_backpressure": "actor reads its socket at a throttled rate (MIX-08 reader_rate_limit) during the action",
    "malformed_or_stale_packet": "actor injects a stale state_id / out-of-reach / malformed packet during the action",
}
LIFECYCLE = {
    "steady": "server keeps running",
    "save_all_checkpoint": "/save-all (checkpoint) issued during the action",
    "clean_restart_mid_action": "/stop between action steps, restart same world, rejoin and continue",
    "crash_kill9_wal_replay": "SIGKILL the server between action steps; restart replays tick WAL from last checkpoint",
    "hot_backup_during_action": "operator hot backup taken during the action; backup later restored and inspected",
    "pause_when_empty_resume": "all players leave (pause-when-empty-seconds>0), world pauses, players rejoin",
}
PERMS = {"non_op_0": 0, "op_1": 1, "op_2": 2, "op_3": 3, "op_4": 4, "console": 4}
FACTORS = ["client_kind", "domain", "game_mode", "dimension", "difficulty", "player_count", "network_condition", "server_lifecycle", "permission"]

# ------------------------------------------------------------------ driver capabilities  (A available/proven, P tool exists but unproven, M missing)
DRIVERS = ["protocol_bot", "java_mcp", "loader_java_mcp", "native_mcp"]
CAPS = {
    "join_play": ("AAAA", "connect/wait_for_play; bots walk handshake->play"),
    "walk_move": ("AAPA", "navigate_to_block/press_inputs; bot movement windows"),
    "jump_sneak_sprint_inputs": ("PAPA", "press_inputs keys; bots send PlayerCommand sprint/PlayerInput"),
    "chat_send": ("AAAA", "send_chat; bots ServerboundChat"),
    "command_send": ("AAAA", "send_chat '/'; bots ServerboundChatCommand"),
    "read_chat": ("AAAA", "observe recent chat; bots decode SystemChat"),
    "read_world_state": ("AAAA", "read_block/list_entities/observe; bots decode received chunks/entities"),
    "use_item_on_block": ("AAPA", "minecraft_use_item_on; bot use_block"),
    "place_block": ("AAPA", "use_item_on with block; bot use_block"),
    "use_item_air": ("AAPA", "press_inputs use; bot use_item"),
    "release_use_item": ("MAPA", "press_inputs hold/release (native-v2-ranged, playable-26); bots lack ReleaseUseItem"),
    "timed_mining_survival": ("MAPA", "break_block/mine_until_crack; bots send StartDestroyBlock only"),
    "creative_instant_break": ("AAPA", "break_block in creative; bot break_block"),
    "attack_entity": ("AAPA", "attack_entity_once; bot attack_entity"),
    "interact_entity": ("AAPA", "interact_entity; bot entity_interact"),
    "select_hotbar": ("AAPA", "set_hotbar_slot; bot select_slot"),
    "drop_item": ("AAPA", "drop_selected_item; bot drop_stack"),
    "swap_hands_offhand": ("MPPP", "press_inputs key F (unexercised); bots lack SwapItemWithOffhand"),
    "open_container": ("AAPA", "use_item_on container; bot open_container"),
    "click_pickup_quickmove": ("AAPA", "click/quick_move_container_slot; bot click_slot Pickup|QuickMove"),
    "click_advanced_modes": ("MMMM", "no driver issues Swap/Clone/Throw/QuickCraft/PickupAll container clicks"),
    "container_button": ("MAPP", "click_container_button (m94-08 enchanting); bots lack ContainerButtonClick"),
    "text_input_screen": ("MAPA", "set_screen_input (native-v2-p1-signs; java playable-17 sign edit); bots lack SignUpdate/RenameItem"),
    "book_edit": ("MMMM", "no tool edits BookEditScreen pages; bots lack EditBook"),
    "select_trade": ("MPPP", "merchant offer click unexercised (native-v2-p1-trader admission only); bots lack SelectTrade"),
    "set_beacon": ("MPPP", "beacon confirm via click_screen_button unexercised; bots lack SetBeacon"),
    "recipe_book_place": ("MPPP", "read_recipe_book exists, placement unexercised; bots lack PlaceRecipe"),
    "crafter_slot_toggle": ("MPPP", "slot toggle click unexercised; bots lack ContainerSlotStateChanged"),
    "creative_slot_set": ("APPP", "bots SetCreativeModeSlot (setup); MCP creative tab clicks unexercised"),
    "respawn": ("AAPA", "minecraft_respawn; bot respawn"),
    "vehicle_control": ("MPPP", "interact + press_inputs riding unexercised; bots lack MoveVehicle/PaddleBoat"),
    "elytra_glide": ("MPPP", "press_inputs jump mid-air unexercised; bots lack StartFallFlying"),
    "dialog_interaction": ("MPAA", "click_screen_button/set_screen_input (dialogs, loader settlements smoke)"),
    "read_hud_ui": ("MPPA", "observe HUD nodes (admin-commands native); bots see packets only"),
    "screenshot_visual": ("MAAA", "minecraft_screenshot + independent visual review; bots have no render"),
    "advancement_stats_ui": ("MPPA", "native-v2-p4-advancements/statistics"),
    "reconnect_mid_action": ("MAPA", "disconnect/connect at a chosen step (playable-36, native-lifecycle); bots only auto-reconnect after transport drop"),
    "duplicate_login_same_account": ("PMMM", "two bot processes with one name plausible; no harness scenario"),
    "slow_reader_backpressure": ("AMMM", "player_scale_bots MIX-08 reader_rate_limit"),
    "malformed_packet_injection": ("PAMM", "m94-03f malformed container rejection (java agent); wire_probe raw writer plausible"),
    "multi_client_2": ("AAPA", "two-client regression driver; native p1 observer pair; bots"),
    "multi_client_N": ("AMMP", "player_scale_bots N; native-scale canaries beside bots"),
    "server_clean_restart": ("AAPA", "catalog-load checkpoint-restart; regression before/after phases; native_v2 owned restart"),
    "server_save_all": ("AAPA", "console/op save-all in pause-empty, light-seam, catalog-load"),
    "server_kill9_wal_replay": ("MMMM", "no harness kills+replays and then re-verifies; only core unit persistence_crash_tests"),
    "server_hot_backup": ("MMMM", "no harness drives edge backup/hot_backup and restores"),
    "server_pause_empty": ("AMMM", "pause-empty functional-native gate (protocol join)"),
    "console_command": ("APPA", "harness stdin console (pause-empty, admin fixtures)"),
    "plugin_deploy": ("AAAA", "--plugins-dir deployment in harness/sibling gates"),
    "loader_bundle_handshake": ("AMAA", "player_scale_bots_loader; loader-live; native Loader UI (native_queue3)"),
    "loader_view_request": ("MMAA", "minecraft_request_loader_view"),
    "computer_channel": ("MMPM", "loader-fabric ComputerClient/ComputerScreen exist; no harness scenario; native client lacks solaris:computer"),
    "native_screen_share": ("MMMP", "solaris:screen relay in deploy-final edge (screen.rs/screen_service.rs); native client branches implement capture/share (screen-codec, screen-audio); no MCP tools for share consent/request/answer yet; Java/Loader clients do not speak solaris:screen"),
    "native_voice": ("MMMP", "solaris:voice relay in deploy-final edge (voice.rs/voice_service.rs); native client 'voice' branch; no MCP push-to-talk/listen tools yet; only native clients are served"),
    "svc_voice": ("MMMM", "no Simple Voice Chat server bridge (Solaris voice is native solaris:voice) and no SVC client lane"),
    "resource_pack_prompt": ("MPPP", "client accepts/declines via screen buttons unexercised; bots lack ResourcePack status reply"),
    "transfer_follow": ("MPPP", "client follows Transfer unexercised"),
    "admin_protocol_client": ("PMMM", "status/query/rcon/msmp/dashboard clients: raw protocol tooling plausible (wire_probe status), no gate"),
    "velocity_proxy": ("MMMM", "no proxy in harness"),
    "online_mode_auth": ("MPMP", "java 'client-token' prerequisite exists; no online-mode gate"),
    "natural_terrain_navigation": ("MAPA", "playable-* natural wood, native_navigation"),
    "portal_travel": ("MPPP", "stand-in-portal travel unexercised (native-v2-dimensions uses commands)"),
    # client-kind lanes
    "windows_mcp_lane": ("MMMM", "no harness lane runs native MCP on Windows (TCP transport is portable)"),
    "svc_client_profile": ("MMMM", "no Fabric+SVC client profile in loader-live/dh-gate"),
    "loader_platform_neoforge_forge": ("MMPM", "loader-live accepts --platform neoforge|forge; only fabric scenario recorded"),
}
CAP_STATUS = {c: dict(zip(DRIVERS, v[0])) for c, v in CAPS.items()}
CAP_NOTE = {c: v[1] for c, v in CAPS.items()}
DRIVER_DESC = {
    "protocol_bot": "crates/mc-test-harness player_scale_bots --mechanic-spec (actions: move, chat, command, use_block, break_block(start), click_slot Pickup|QuickMove, open/close_container, attack/entity_interact, use_item, select_slot, drop_stack, respawn, reader_rate_limit); wire_probe/quirk_probe",
    "java_mcp": "../solaris-loader java-agent bridge MCP (52 minecraft_* tools) on vanilla 26.1.2",
    "loader_java_mcp": "same java-agent MCP inside Fabric/NeoForge/Forge + Solaris Loader (harness loader-live / dh-gate)",
    "native_mcp": "../solaris-client src/automation MCP (50 minecraft_* tools), harness profiles entities/admin-commands/dialogs/...",
}

# ------------------------------------------------------------------ subject catalog
def expand_src(s):
    for k, v in PREFIX.items():
        if s.startswith(k):
            return v + s[len(k):]
    return s


TEST_TEXT = {}
for crate in ("mc-core", "mc-world-conversion", "mc-world-tools", "mc-edge", "mc-script", "mc-plugin-host", "mc-loader"):
    for p in (SOL / "crates" / crate / "src").rglob("*.rs"):
        if p.name.endswith("_tests.rs") or p.name == "tests.rs":
            TEST_TEXT[str(p.relative_to(SOL))] = p.read_text(errors="ignore")
TEST_FILES = set(TEST_TEXT)
CMD_TEST_FILES = [f for f in TEST_FILES if re.search(r"command|admin|console|operator|scoreboard|bossbar|border|forceload|block_log|tick_control|locate|summon|effects|enchant", f)]
SELECTOR_TOKENS = {"at_s": ['"@s'], "at_p": ['"@p'], "at_a": ['"@a'], "at_r": ['"@r'], "at_e": ['"@e'], "opt_xyz": ["x="], "opt_dxdydz": ["dx="],
                   "opt_type": ["type="], "opt_name": ["name="], "opt_distance": ["distance="], "opt_limit": ["limit="], "sort_nearest": ["sort=nearest"],
                   "sort_furthest": ["sort=furthest"], "sort_random": ["sort=random"], "sort_arbitrary": ["sort=arbitrary"],
                   "invalid_selector": ["Unknown selector type"], "nonop_selector_msg": ["Selector not allowed"]}


def unit_tests_for(srcs):
    """Module-owned tests: <module>_tests.rs, <dir>/tests.rs, and every *_tests.rs beside a mod.rs."""
    out = []
    for s in srcs:
        if not s.startswith("crates/") or not s.endswith(".rs"):
            continue
        if s.endswith("_tests.rs"):
            if s in TEST_FILES:
                out.append(s)
            continue
        base = s[:-3]
        cands = [base + "_tests.rs", base + "/tests.rs"]
        if base.endswith("/mod"):
            d = base.rsplit("/", 1)[0]
            cands += sorted(f for f in TEST_FILES if f.rsplit("/", 1)[0] == d)
        for cand in cands:
            if cand in TEST_FILES and cand not in out:
                out.append(cand)
    return out


def token_tests(domain_name, name):
    """Tests that name the code-enumerated value itself (exact identifier/string token)."""
    files = None
    if domain_name == "commands":
        toks = [f'"{name} ', f'"/{name} ', f'"{name}"']
    elif domain_name == "gamerules":
        aliases = dict(cg.gamerules()).get(name, [])
        toks = [f'"{name}"', f"minecraft:{name}", f"gamerule {name}"] + [f'"{a}"' for a in aliases] + [f"gamerule {a}" for a in aliases]
    elif domain_name == "effects":
        snake = re.sub(r"(?<!^)([A-Z])", r"_\1", name).lower()
        toks = [f"MobEffect::{name}", f"effect give @s {snake}", f"effect give @s minecraft:{snake}"]
        files = [f for f in TEST_FILES if "MobEffect" in TEST_TEXT[f] or re.search(r"effect|potion|beacon|conduit|consum|brewing", f)]
        toks += [f'"minecraft:{snake}"', f'"{snake}"', f"fn {snake}_", f"_{snake}_"]
    elif domain_name == "damage" and not name.startswith("vanilla_"):
        toks = [f"DamageKind::{name}"]
    elif domain_name == "mobs":
        toks = [f'"minecraft:{name}"']
    elif domain_name == "villagers" and name.startswith("profession_"):
        toks = [f"VillagerProfession::{name[len('profession_'):]}"]
    elif domain_name == "spawning" and name.startswith("natural_"):
        toks = [f"MobCategory::{name[len('natural_'):]}"]
    elif domain_name == "settlements":
        toks = [f"settlement {name.replace('_', ' ')}"]
    elif domain_name == "selectors":
        toks, files = SELECTOR_TOKENS.get(name, []), CMD_TEST_FILES
    elif domain_name == "scoreboard_teams":
        toks = ["ScoreboardCommand"]
    else:
        return []
    return sorted(f for f in (files or TEST_FILES) if any(t in TEST_TEXT[f] for t in toks))


CG = cg.build()
SUBJ = {}
TOKEN_DOMAINS = {"commands", "gamerules", "effects", "damage", "mobs", "selectors", "settlements", "scoreboard_teams"}
CONSOLE_DOMAINS = {"commands", "selectors", "gamerules", "world_border", "block_log"}
for dname, d in subjects.D.items():
    for name, do, expect, extra in d["subjects"]:
        sid = f"{dname}.{name}"
        src = [expand_src(x) for x in (extra.get("src") or [])] + [expand_src(x) for x in d["sources"]]
        src = list(dict.fromkeys(src))
        caps = list(dict.fromkeys(d["caps"] + list(extra.get("caps") or [])))
        if dname in TOKEN_DOMAINS or name.startswith(("profession_", "natural_")):
            unit = unit_tests_for([expand_src(x) for x in (extra.get("src") or [])])
            unit += [f for f in token_tests(dname, name) if f not in unit]
        else:
            unit = unit_tests_for(src)
        SUBJ[sid] = {
            "id": sid, "domain": dname, "name": name, "do": do, "expect": expect, "caps": caps, "sources": src,
            "unit_tests": unit, "menu": bool(extra.get("menu")), "custody": bool(extra.get("custody")),
            "mp": bool(extra.get("mp")), "dims": extra.get("dims") or DIMS, "clients": extra.get("clients") or list(CLIENTS),
            "op": extra.get("op", 0), "status": extra.get("status", "implemented"), "visual": bool(extra.get("visual")),
            "plugins": extra.get("plugins") or [], "inflight": bool(extra.get("inflight")),
            "console": bool(extra.get("console", dname in CONSOLE_DOMAINS)),
            **{k: extra[k] for k in ("ai_goals", "category", "move_policy", "combat_policy", "natural_dims", "second_player") if k in extra},
        }
for s in SUBJ.values():  # loader content needs a Loader-capable client except the vanilla-compat probes
    if s["domain"] == "loader_content" and s["name"] not in ("optional_bundle_vanilla", "required_bundle_vanilla_refused"):
        s["clients"] = LOADER_CLIENTS
    if s["domain"] == "loader_content" and s["name"] in ("optional_bundle_vanilla", "required_bundle_vanilla_refused"):
        s["clients"] = ["vanilla_java_26_1_2"]
    if s["domain"] == "load":
        s["clients"] = ["protocol_bot"]
# Reference has a part only a second client can check: an observer (held item / particles / leave message seen by
# others) or a second participant (PvP victim, sleep quorum, second mob target, second op) — codegen sets second_player.
OBSERVER_RE = re.compile(r"observer|others see|other players|visible to (?:others|all)|invisible to others|outline visible", re.I)
for s in SUBJ.values():
    s["observer"] = not s["mp"] and (bool(OBSERVER_RE.search(s["expect"])) or bool(s.get("second_player")))
DOMAINS = list(subjects.D)

# ------------------------------------------------------------------ plugin profiles
LIVE_PLUGINS = RUN_DATA / "live-plugins"
# Subjects whose oracle depends on which live plugins are enabled: (plugin dirs that change it, live-set oracle).
# Each runs in two profiles that are part of its id (CELL@vanilla, CELL@live): "vanilla" = live set minus those
# plugins, "live" = the full live set. Every other subject has no profile suffix and runs on the live set.
PLUGIN_SENSITIVE = {
    "death_respawn.keep_inventory_false": (["graves"], "live set (graves): the drops are moved into a barrel grave at the death spot, exactly once; no ground copy (plugins.graves_custody)"),
    "death_respawn.death_while_menu_open": (["graves"], "live set (graves): the dropped cursor/inventory items end in the barrel grave exactly once"),
    "death_respawn.death_while_riding": (["graves"], "live set (graves): drops go to the barrel grave"),
    "death_respawn.void_death": (["graves"], "live set (graves): no grave contents expected for void-lost items unless the plugin places one at the last safe spot; record the observed behaviour"),
    "commands.msg": (["solaris-essentials"], "live set: solaris-essentials owns the msg root (core root not advertised); expect the plugin's private message behaviour"),
    "chat.private_message": (["solaris-essentials"], "live set: /msg is solaris-essentials; /tell and /w stay core"),
    "selectors.nonop_selector_msg": (["solaris-essentials"], "live set: /msg is solaris-essentials, so its selector handling for non-ops is the plugin's; /tp and /effect stay core"),
    "session.mute_warn": (["solaris-essentials"], "live set: /msg is solaris-essentials; record whether a native mute also refuses the plugin's /msg"),
    "session.join_fresh_spawn": (["solaris-auth"], "live set (solaris-auth): the player is locked (no build/damage/commands) until /register or /login succeeds"),
}
PLUGIN_PROFILES = ("vanilla", "live")


def plugin_dirs(root):
    return sorted(p.name for p in Path(root).iterdir() if (p / "plugin.toml").is_file())


def plugin_set_fingerprint(root, dirs=None):
    """sha256 over sorted '<plugin id> <sha256 plugin.toml> <sha256 entry>' lines of the deployed plugin dirs.
    State files a plugin writes next to them do not change it; plugin.toml already pins client bundle hashes."""
    lines = []
    for d in plugin_dirs(root) if dirs is None else dirs:
        manifest = (Path(root) / d / "plugin.toml").read_bytes()
        meta = tomllib.loads(manifest.decode())
        entry = (Path(root) / d / meta["entry"]).read_bytes()
        lines.append(f"{meta['id']} {hashlib.sha256(manifest).hexdigest()} {hashlib.sha256(entry).hexdigest()}")
    return hashlib.sha256("\n".join(sorted(lines)).encode()).hexdigest()


def profile_plugins(sid, profile):
    dirs = plugin_dirs(LIVE_PLUGINS)
    return dirs if profile == "live" else [d for d in dirs if d not in PLUGIN_SENSITIVE[sid][0]]


@functools.cache
def profile_fingerprint(sid, profile):
    return plugin_set_fingerprint(LIVE_PLUGINS, profile_plugins(sid, profile))


assert set(PLUGIN_SENSITIVE) <= set(SUBJ), set(PLUGIN_SENSITIVE) - set(SUBJ)
assert all(set(deps) <= set(plugin_dirs(LIVE_PLUGINS)) for deps, _ in PLUGIN_SENSITIVE.values()), "unknown live plugin dir"
for _sid, _s in SUBJ.items():
    _s["profiles"] = list(PLUGIN_PROFILES) if _sid in PLUGIN_SENSITIVE else []
if ARGS.mode == "fingerprint":
    print(plugin_set_fingerprint(ARGS.plugin_fingerprint))
    sys.exit(0)

# ------------------------------------------------------------------ constraints
CONSTRAINTS = [
    {"id": "C01", "kind": "invalid", "rule": "player_count == '1' and subject.mp", "reason": "multiplayer-only subject (PvP, /msg, tpa, shared observation) needs >= 2 players"},
    {"id": "C02", "kind": "invalid", "rule": "dimension not in subject.dims", "reason": "dimension-bound mechanic (End fight/gateway in the_end, nether-portal pair overworld<->nether, weather/raid patrols in overworld, anchor spawn in nether, water-evaporation in nether)"},
    {"id": "C03", "kind": "invalid", "rule": "network_condition == 'disconnect_during_menu' and not subject.menu", "reason": "no menu is open during the action"},
    {"id": "C04", "kind": "invalid", "rule": "client_kind not in subject.clients", "reason": "client-specific surface: voice needs fabric_svc; computer terminal/desktop capture only Fabric Loader (ADR 0018, native client lacks solaris:computer); DH LOD needs DH client; Loader bundles need a Loader-capable client; protocol-mismatch/illegal-move probes need a raw protocol bot; load scenarios are protocol-bot fleets"},
    {"id": "C05", "kind": "invalid", "rule": "permission == 'console' and not subject.console", "reason": "console has no entity: it cannot perform gameplay actions, only commands"},
    {"id": "C06", "kind": "negative_oracle", "rule": "permission level < subject.op", "reason": "command refused/hidden (vanilla permission gate) — valid negative scenario"},
    {"id": "C07", "kind": "negative_oracle", "rule": "game_mode == 'spectator' and domain in interaction domains", "reason": "spectators cannot break/place/use/attack; may view container contents only"},
    {"id": "C08", "kind": "negative_oracle", "rule": "game_mode == 'adventure' and domain in (block_break, block_place)", "reason": "refused without can_break/can_place_on components"},
    {"id": "C09", "kind": "negative_oracle", "rule": "game_mode == 'creative' and domain in (damage, death_respawn, combat-received)", "reason": "creative is invulnerable except void and /kill; no item debit"},
    {"id": "C10", "kind": "negative_oracle", "rule": "difficulty == 'peaceful' and subject is hostile-mob/hunger/raid related", "reason": "peaceful removes hostiles, no hunger damage, no raids"},
    {"id": "C11", "kind": "server_gap", "rule": "subject.status in (no_code_hit, server_missing, not_deployable)", "reason": "vanilla oracle documented; scenario is expected to FAIL until the server implements it (or must fail closed)"},
    {"id": "C12", "kind": "procedure", "rule": "lifecycle == 'pause_when_empty_resume' requires every connected player to leave first", "reason": "pause only triggers at zero players; actors rejoin before the observe step"},
]
SPECTATOR_NEG = {"block_break", "block_place", "block_interact", "containers", "crafting", "inventory", "items", "combat", "farming", "redstone",
                 "fluids", "falling_blocks", "explosions", "fire", "special_blocks", "entities_misc", "villagers"}
HOSTILE_HINT = re.compile(r"hostile|zombie|creeper|raid|patrol|phantom|warden|Starvation|monster|Monster|witch|pillager|skeleton")


def subject_ok(s, row):
    if row.get("player_count") == "1" and s["mp"]:
        return False
    if "dimension" in row and row["dimension"] not in s["dims"]:
        return False
    if row.get("network_condition") == "disconnect_during_menu" and not s["menu"]:
        return False
    if "client_kind" in row and row["client_kind"] not in s["clients"]:
        return False
    if row.get("permission") == "console" and not s["console"]:
        return False
    return True


CF = ["client_kind", "player_count", "network_condition", "permission", "dimension"]  # factors constrained per subject
VALUES = {"client_kind": list(CLIENTS), "domain": DOMAINS, "game_mode": GAME_MODES, "dimension": DIMS, "difficulty": DIFFS,
          "player_count": PLAYERS, "network_condition": list(NETWORK), "server_lifecycle": list(LIFECYCLE), "permission": list(PERMS)}
# subject_ok checks each factor independently, so a subject's allowed rows are a product of per-factor sets.
ALLOWED_PRODUCTS = defaultdict(set)
for s in SUBJ.values():
    prof = tuple(frozenset(v for v in VALUES[f] if subject_ok(s, {f: v})) for f in CF)
    ALLOWED_PRODUCTS[s["domain"]].add(prof)
    ALLOWED_PRODUCTS["*"].add(prof)


def valid(row):
    checks = [(i, row[f]) for i, f in enumerate(CF) if f in row]
    return any(all(v in prof[i] for i, v in checks) for prof in ALLOWED_PRODUCTS[row.get("domain", "*")])

# ------------------------------------------------------------------ cells, ids, ledger (no wave-1 generation needed)
CELL_FACTORS = ["client_kind", "game_mode", "dimension", "difficulty", "player_count", "network_condition", "server_lifecycle", "permission"]
assert not [k for k in SUBJ if "/" in k or ":" in k], "subject ids must not contain '/' or ':'"
LEDGER = Path(os.environ.get("SCENARIO_LEDGER", RUN_DATA / "ledger.jsonl"))
TAG_RE = re.compile(r"F|pw|bl|tr\.[a-z0-9_.]+")


def cell_of(subject_id, row):
    """Stable cell key: subject/client/mode/dim/difficulty/players/network/lifecycle/permission[@plugin_profile]."""
    cell = "/".join([subject_id] + [row[f] for f in CELL_FACTORS])
    return f"{cell}@{row['plugin_profile']}" if SUBJ[subject_id]["profiles"] else cell


def profile_rows(s, row):
    """The row once per plugin profile for plugin-sensitive subjects, else the row itself."""
    return [{**row, "plugin_profile": p} for p in s["profiles"]] or [row]


def parse_cell(cell):
    """(subject, row) for a valid cell, else None (unknown subject/value/profile or a C01-C05 violation)."""
    cell, at, profile = cell.partition("@")
    parts = cell.split("/")
    if len(parts) != 1 + len(CELL_FACTORS) or parts[0] not in SUBJ:
        return None
    s = SUBJ[parts[0]]
    if bool(s["profiles"]) != bool(at) or (at and profile not in s["profiles"]):
        return None
    row = dict(zip(CELL_FACTORS, parts[1:]))
    if any(row[f] not in VALUES[f] for f in CELL_FACTORS):
        return None
    if not subject_ok(s, row):
        return None
    return s, {**row, "domain": s["domain"], **({"plugin_profile": profile} if at else {})}


def parse_id(sid):
    """(tag, cell, subject, row) or None."""
    tag, sep, cell = sid.partition(":") if isinstance(sid, str) else ("", "", "")
    if not sep or not TAG_RE.fullmatch(tag):
        return None
    parsed = parse_cell(cell)
    return (tag, cell, *parsed) if parsed else None


def legacy_profileless(sid):
    """True for an id recorded before its subject became plugin-sensitive (no @profile suffix)."""
    tag, sep, cell = sid.partition(":") if isinstance(sid, str) else ("", "", "")
    return bool(sep) and "@" not in cell and parse_id(f"{tag}:{cell}@live") is not None


def check_ledger_line(obj, strict=False):
    """Error text for a ledger object, or None when it satisfies the contract.
    strict (new appends): a plugin_set must equal the current fingerprint of the id's profile."""
    if not isinstance(obj, dict) or set(LEDGER_KEYS) - set(obj) or set(obj) - set(LEDGER_KEYS) - set(LEDGER_OPTIONAL_KEYS):
        return f"keys must be exactly {list(LEDGER_KEYS)} plus optional {list(LEDGER_OPTIONAL_KEYS)}"
    if any(not isinstance(v, str) for v in obj.values()):
        return "all values must be strings"
    parsed = parse_id(obj["id"])
    if not parsed:
        if legacy_profileless(obj["id"]):
            return f"plugin-sensitive subject needs a profile: record {obj['id']}@vanilla or @live with plugin_set"
        return f"unknown or constraint-violating id {obj['id']!r}"
    if obj["status"] not in LEDGER_STATUS:
        return f"status must be one of {LEDGER_STATUS}"
    expected = CLIENTS[parsed[3]["client_kind"]]["driver"]
    if obj["driver"] != expected:
        return f"driver {obj['driver']!r} != {expected!r} required by the id's client_kind"
    if not re.fullmatch(r"[0-9A-Za-z._+-]{7,128}", obj["server_sha"]):
        return "server_sha must be the solaris-v2 binary sha256 (or git sha), no spaces"
    if not obj["run_dir"].startswith("/"):
        return "run_dir must be absolute"
    if not obj["ts"].endswith("Z"):
        return "ts must be UTC ISO-8601 ending in Z"
    try:
        datetime.fromisoformat(obj["ts"][:-1] + "+00:00")
    except ValueError:
        return "ts is not ISO-8601"
    if obj["status"] in ("blocked", "fail") and not obj["note"].strip():
        return "blocked needs the missing capability and fail needs finding+evidence in note"
    profile = parsed[3].get("plugin_profile")
    if profile and "plugin_set" not in obj:
        return "@profile ids need plugin_set (gen_matrix.py --plugin-fingerprint <deployed plugins dir>)"
    if "plugin_set" in obj and not re.fullmatch(r"[0-9a-f]{64}", obj["plugin_set"]):
        return "plugin_set must be the 64-hex fingerprint printed by --plugin-fingerprint"
    if strict and profile and obj["plugin_set"] != (want := profile_fingerprint(parsed[2]["id"], profile)):
        return (f"plugin_set {obj['plugin_set'][:16]}… is not the {profile} profile ({', '.join(profile_plugins(parsed[2]['id'], profile))}) "
                f"= {want[:16]}…; deploy that set and fingerprint it")
    return None


def read_ledger():
    """(latest object per id in file order, errors, legacy profile-less lines, outdated profile lines).
    An @profile line whose plugin_set is not the current fingerprint of its profile ran on an older
    plugin build: it stays in the file as history but no longer counts as coverage."""
    latest, errors, legacy, outdated = {}, [], [], []
    if not LEDGER.exists():
        return latest, errors, legacy, outdated
    with LEDGER.open() as f:
        for n, line in enumerate(f, 1):
            if not line.strip():
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError as e:
                errors.append(f"line {n}: invalid JSON ({e.msg})")
                continue
            if isinstance(obj, dict) and legacy_profileless(obj.get("id")):
                legacy.append(f"line {n}: {obj['id']}")
                continue
            err = check_ledger_line(obj)
            if err:
                errors.append(f"line {n}: {err}")
                continue
            if check_ledger_line(obj, strict=True):
                outdated.append(f"line {n}: {obj['id']} plugin_set {obj['plugin_set'][:16]}…")
                continue
            latest[obj["id"]] = obj
    return latest, errors, legacy, outdated


def append_ledger(objs):
    """Validate all, then one locked O_APPEND write + fsync (safe for concurrent writers)."""
    errors = [f"{o.get('id') if isinstance(o, dict) else o}: {e}" for o in objs if (e := check_ledger_line(o, strict=True))]
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 2
    data = "".join(json.dumps(o, ensure_ascii=False, separators=(",", ":")) + "\n" for o in objs).encode()
    fd = os.open(LEDGER, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o664)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        view = memoryview(data)
        while view:
            view = view[os.write(fd, view):]
        os.fsync(fd)
    finally:
        fcntl.flock(fd, fcntl.LOCK_UN)
        os.close(fd)
    print(f"recorded {len(objs)} line(s) in {LEDGER}")
    return 0


if ARGS.mode == "record":
    sys.exit(append_ledger([{"id": ARGS.id, "status": ARGS.status, "driver": ARGS.driver, "server_sha": ARGS.server_sha,
                             "run_dir": ARGS.run_dir, "ts": ARGS.ts or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                             "note": ARGS.note, **({"plugin_set": ARGS.plugin_set} if ARGS.plugin_set else {})}]))
if ARGS.mode == "record_file":
    _objs = []
    for _n, _line in enumerate(Path(ARGS.record_file).read_text().splitlines(), 1):
        if _line.strip():
            try:
                _objs.append(json.loads(_line))
            except json.JSONDecodeError as _e:
                sys.exit(f"{ARGS.record_file} line {_n}: invalid JSON ({_e.msg})")
    sys.exit(append_ledger(_objs))



# ------------------------------------------------------------------ pairwise (greedy AETG, fixed seed)
def pairwise(seed=20261024, candidates=40):
    rng = random.Random(seed)
    idx = {f: i for i, f in enumerate(FACTORS)}
    uncovered = set()
    for a, b in itertools.combinations(FACTORS, 2):
        for va in VALUES[a]:
            for vb in VALUES[b]:
                if valid({a: va, b: vb}):
                    uncovered.add((a, va, b, vb))
    total = len(uncovered)
    rows = []

    def gain(row, f, v):
        g = 0
        for o, ov in row.items():
            p = (f, v, o, ov) if idx[f] < idx[o] else (o, ov, f, v)
            if p in uncovered:
                g += 1
        return g

    while uncovered:
        best, best_score = None, -1
        pool = sorted(uncovered)
        for _ in range(candidates):
            a, va, b, vb = rng.choice(pool)
            row = {a: va, b: vb}
            rest = [f for f in FACTORS if f not in row]
            rng.shuffle(rest)
            for f in rest:
                opts = [v for v in VALUES[f] if valid({**row, f: v})]
                scored = [(gain(row, f, v), rng.random(), v) for v in opts]
                row[f] = max(scored)[2]
            score = sum(1 for a2, b2 in itertools.combinations(FACTORS, 2)
                        if ((a2, row[a2], b2, row[b2]) in uncovered))
            if score > best_score:
                best, best_score = row, score
        rows.append(best)
        for a2, b2 in itertools.combinations(FACTORS, 2):
            uncovered.discard((a2, best[a2], b2, best[b2]))
    return rows, total


# ------------------------------------------------------------------ scenario composition
def perm_level(p):
    return PERMS[p]


def driver_for(row, s, extra_caps=()):
    client = row["client_kind"]
    drv = CLIENTS[client]["driver"]
    caps = list(s["caps"])
    if "break_block" in caps:
        caps.remove("break_block")
        caps.append("creative_instant_break" if row["game_mode"] == "creative" else "timed_mining_survival")
    caps += {"2": ["multi_client_2"], "N8": ["multi_client_N"]}.get(row["player_count"], [])
    caps += {"reconnect_mid_action": ["reconnect_mid_action"], "disconnect_during_menu": ["reconnect_mid_action"],
             "duplicate_login": ["duplicate_login_same_account"], "slow_reader_backpressure": ["slow_reader_backpressure"],
             "malformed_or_stale_packet": ["malformed_packet_injection"]}.get(row["network_condition"], [])
    caps += {"save_all_checkpoint": ["server_save_all"], "clean_restart_mid_action": ["server_clean_restart"],
             "crash_kill9_wal_replay": ["server_kill9_wal_replay"], "hot_backup_during_action": ["server_hot_backup"],
             "pause_when_empty_resume": ["server_pause_empty"]}.get(row["server_lifecycle"], [])
    if row["permission"] == "console":
        caps.append("console_command")
    if s["plugins"]:
        caps.append("plugin_deploy")
    if s["visual"]:
        caps.append("screenshot_visual")
    caps += CLIENTS[client].get("extra_caps", [])
    caps += list(extra_caps)
    caps = list(dict.fromkeys(caps))
    missing = [c for c in caps if CAP_STATUS[c][drv] == "M"]
    unproven = [c for c in caps if CAP_STATUS[c][drv] == "P"]
    status = "ready" if not missing and not unproven else ("plausible" if not missing else "missing_capability")
    out = {"kind": drv, "status": status, "required_capabilities": caps}
    if missing:
        out["missing"] = missing
    if unproven:
        out["unproven"] = unproven
    if s["status"] != "implemented":
        out["server_gap"] = s["status"]
    if missing and drv != "protocol_bot":
        alt = [c for c in caps if CAP_STATUS[c]["protocol_bot"] == "M" and c not in ("screenshot_visual",) and c not in CLIENTS[client].get("extra_caps", [])]
        if not alt:
            out["fallback"] = "protocol_bot can drive the server-side oracle (no client-presentation evidence)"
    return out


E2E_BY_SUBJ = defaultdict(list)
for t in E2E:
    for raw in t["subjects"]:
        partial = raw.startswith("~")
        E2E_BY_SUBJ[raw.lstrip("~")].append((t, partial))
_unknown = sorted({k for k in E2E_BY_SUBJ if k not in SUBJ})
assert not _unknown, f"existing.py cites unknown subjects: {_unknown}"
LIFE_MAP = {"steady": "steady", "save_all_checkpoint": "save_all_checkpoint", "clean_restart_mid_action": "clean_restart",
            "crash_kill9_wal_replay": "kill9", "hot_backup_during_action": "hot_backup", "pause_when_empty_resume": "pause_when_empty_resume"}


def coverage(row, s):
    hits = E2E_BY_SUBJ.get(s["id"], [])
    exact, near = [], []
    for t, partial in hits:
        ok = (row["client_kind"] in t["clients"] and t["players"] == row["player_count"]
              and t["network"] == row["network_condition"] and LIFE_MAP[row["server_lifecycle"]] in t["lifecycle"] and not partial)
        (exact if ok else near).append(t["id"])
    if exact:
        st = "covered"
    elif near:
        st = "partial"
    elif s["unit_tests"]:
        st = "unit_only"
    else:
        st = "none"
    out = {"status": st}
    if exact:
        out["e2e"] = exact
    if near:
        out["e2e_other_context"] = sorted(set(near))
    if s["unit_tests"]:
        out["unit"] = s["unit_tests"][:4]
    return out


def mode_note(row, s):
    m, d = row["game_mode"], s["domain"]
    if m == "spectator" and d in SPECTATOR_NEG:
        return "NEGATIVE (spectator): action refused; world/inventory unchanged; spectator may only view container contents; client resynced"
    if m == "adventure" and d in ("block_break", "block_place"):
        return "NEGATIVE (adventure): refused unless item has can_break/can_place_on for the target"
    if m == "creative" and d in ("damage", "death_respawn") and s["name"] not in ("Void", "void_death"):
        return "creative: no damage/death except void and /kill"
    if m == "creative" and s["custody"]:
        return "creative: placements/uses do not debit items; creative break yields no drops"
    return None


def perm_note(row, s):
    lvl = perm_level(row["permission"])
    if s["op"] and lvl < s["op"]:
        return f"NEGATIVE (permission {row['permission']} < op {s['op']}): command not advertised in the command tree, execution answers 'Unknown or incomplete command', no state change"
    return None


def diff_note(row, s):
    text = s["id"] + " " + s["expect"]
    if row["difficulty"] == "peaceful" and HOSTILE_HINT.search(text):
        return "peaceful: hostile mobs despawn/do not spawn, no starvation damage, raids cannot start"
    if row["difficulty"] == "hard" and s["domain"] in ("mobs", "damage", "villagers") and HOSTILE_HINT.search(text):
        return "hard: higher mob damage; zombies may break doors and always convert villagers; starvation can kill"
    return None


NET_ORACLE = {
    "reconnect_mid_action": "after reconnect the client receives exactly the server-committed state; the interrupted action is either fully applied or not at all; item totals across player/containers/entities unchanged except by the committed part",
    "disconnect_during_menu": "server closes the menu on Leave; carried cursor stack returns to the player inventory (dropped at feet if full) exactly once; container contents reflect only acknowledged clicks; reopening after rejoin shows the same contents",
    "duplicate_login": "vanilla 26.1.2 newcomer wins (owner requirement, BugTriage DUPLICATE-LOGIN): the existing session is disconnected ('You logged in from another location'), the new connection reaches Play with the same persisted player state; "
                       "exactly one live player entity for the UUID, no inventory copy or loss, no join-wait timeout. Current Solaris (core tick/input/session.rs:93-107) refuses the newcomer instead and live runs time out in join-wait — expected to fail until ServerFixes lands",
    "slow_reader_backpressure": "server tick (MSPT) and other players unaffected; slow client either catches up with consistent state or is disconnected by bounded queue/timeout; no unbounded memory growth",
    "malformed_or_stale_packet": "out-of-reach/illegal/malformed input is rejected (BlockChangedAck + block update / correction teleport / protocol disconnect) without state change; a stale container state_id is applied once and followed by a full-state resync (vanilla handleContainerClick); never duplicated; server keeps running",
}
LIFE_ORACLE = {
    "save_all_checkpoint": "save-all completes without stalling the tick; post-save state equals pre-save state",
    "clean_restart_mid_action": "after /stop + restart the world, block entities, entities (same UUIDs), player inventories/positions and partially finished processes equal the last committed pre-stop state; no duplication or loss",
    "crash_kill9_wal_replay": "after SIGKILL + restart (no wait for a published WAL fence) the world equals a consistent prefix of the committed transactions: every transaction is all-or-nothing (no torn/split/half-moved stacks), "
                              "no item duplication, nothing that was durable before the action (setup baseline) is lost, and later transactions only appear after earlier ones; surviving every pre-kill ACK is NOT required (latest-ACK durability needs a separate fenced proof); "
                              "world opens or fails closed on corruption (never silently reset); where the reference says 'identical/restored after restart', read it as identical to that recovered prefix",
    "hot_backup_during_action": "server keeps ticking during backup; the backup restores to a single consistent tick (no torn state between player and containers)",
    "pause_when_empty_resume": "while empty the world does not advance (no growth/smelting/mob movement); after rejoin it resumes from the same tick and the action completes normally",
}
PC_ORACLE = {"2": "second client observes the same result (block/entity/menu/inventory updates) in the next publication; no desync between observers",
             "N8": "8 concurrent actors: results conserved, server MSPT stays below 50 ms, no correction storms or protocol errors"}


SCENARIO_TEMPLATE = {
    "execution_path": "tools/harness (python3 -m tools.harness run ...; single cmd_run, shared lifecycle/receipts) and catalog_mixes.py for composite bot scenarios; this generator only selects/describes scenarios and validates ledger lines",
    "run_wrapper": "every run goes through /run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh -m 6G <harness cmd>; no direct server launches",
    "setup": [
        "Server: the release solaris-v2 binary (its full sha256 is the ledger server_sha) on a fresh isolated world per run; harness owns process/Xvfb lifecycle and receipts",
        f"Plugins: [plugins].directory = {LIVE_PLUGINS} (live VPS set of 10: basic-economy, chisel, computer(id solaris), graves, solaris-audit, solaris-auth, solaris-economy, solaris-essentials, solaris-permissions, solaris-towns); copy into the run dir when a plugin writes state. "
        "Plugin-sensitive subjects (ids ending @vanilla/@live) run on the profile their id names: @live = the full set, @vanilla = the set minus the plugins that change their oracle (context.plugin_profile, steps)",
        "Live-set effects on every scenario: solaris-auth locks each player until /register then /login (bots: --auth-password); graves takes death drops into a barrel grave; solaris-essentials owns the /msg root; chisel/computer offer optional Loader bundles",
        "Ledger: every @profile line carries plugin_set = `gen_matrix.py --plugin-fingerprint <deployed plugins dir>`; --record rejects a fingerprint that is not the id's profile. "
        "Harness runners select and verify the profile with the shared helper tools/harness matrix_execution.select_plugin_profile / plugin_set_fingerprint (bots-runproto 24fcc5c1), not a lane-local convention; adapters branch on context.plugin_profile",
        "Console sets context.difficulty; connect context.player_count clients of context.client_kind through driver.kind; console grants context.permission and context.game_mode; actors stand on a prepared pad in context.dimension",
        "Console fixture supplies the blocks/items/entities the action needs; fixture commands are not part of the oracle",
    ],
    "observe": "Client-received state through the driver plus server-side truth (console query, or world/store probe after a clean stop)",
    "always_oracle": "server process alive; no unexpected disconnect; no ERROR/panic in server log; MSPT < 50 ms outside declared load scenarios",
}


def steps_for(row, s, hook=None):
    st = []
    if row.get("plugin_profile"):
        p = row["plugin_profile"]
        st.append(f"Plugin profile {p}: [plugins].directory = copy of {LIVE_PLUGINS.name}/ with {', '.join(profile_plugins(s['id'], p))} "
                  f"(fingerprint {profile_fingerprint(s['id'], p)}); ledger plugin_set = gen_matrix.py --plugin-fingerprint <that dir>")
    if s["plugins"]:
        st.append(f"Deploy plugins: {', '.join(s['plugins'])}")
    st.append(("Console issues: " if row["permission"] == "console" else "Action: ") + s["do"])
    if hook:
        st += hook
    else:
        if row["network_condition"] != "normal":
            st.append(f"Network ({row['network_condition']}): {NETWORK[row['network_condition']]}")
        if row["server_lifecycle"] != "steady":
            st.append(f"Lifecycle ({row['server_lifecycle']}): {LIFECYCLE[row['server_lifecycle']]}")
    if row["player_count"] != "1" and not hook:
        st.append("Second client observes the actor's target" if row["player_count"] == "2" else "All 8 clients repeat the action concurrently on own targets and on one shared target")
    return st


def oracle_for(row, s, extra=None):
    o = [f"reference: {s['expect']}"]
    for note in (mode_note(row, s), perm_note(row, s), diff_note(row, s)):
        if note:
            o.append(note)
    if extra:
        o += extra
    else:
        if row["network_condition"] != "normal":
            o.append(NET_ORACLE[row["network_condition"]])
        if row["server_lifecycle"] != "steady":
            o.append(LIFE_ORACLE[row["server_lifecycle"]])
    if row["player_count"] != "1":
        o.append(PC_ORACLE[row["player_count"]])
    elif s["observer"]:
        o.append("player_count 1: the second-player part of the reference (observer, victim, quorum or second target) cannot be checked without a second client — "
                 "verify the actor-side part only; that part is proven by this subject's multi-player wave-1 row (tr.observer_clause or another player_count 2/N8 row)")
    if row["dimension"] != "overworld":
        o.append(f"holds in {row['dimension']} (dimension-isolated world)")
    if s["custody"]:
        o.append("conservation: item totals over inventories/containers/item entities as expected (no dupe, no loss)")
    if row.get("plugin_profile") == "live":
        o.append(PLUGIN_SENSITIVE[s["id"]][1])
    elif row.get("plugin_profile") == "vanilla":
        deps = ", ".join(PLUGIN_SENSITIVE[s["id"]][0])
        o.append(f"plugin profile vanilla ({deps} not deployed): plain server behaviour per reference; no {deps} side effects")
    if s["status"] != "implemented":
        o.append(f"SERVER GAP ({s['status']}): expected to fail until implemented; must not crash or corrupt state")
    return o


SCEN = []


def short_title(row, s):
    ctx = [row["client_kind"], row["game_mode"], row["dimension"], f"p{row['player_count']}", row["permission"]]
    ctx += [v for v in (row["network_condition"], row["server_lifecycle"]) if v not in ("normal", "steady")]
    return f"{s['id']} [{', '.join(ctx)}]"


def origin_tag(origin):
    head, _, rest = origin.partition(":")
    tag = {"pairwise": "pw", "baseline": "bl", "triple": "tr", "full": "F"}[head]
    return tag + ("." + rest.replace(":", ".") if rest else "")


def build_scenario(origin, row, s, hook=None, extra_oracle=None, extra_caps=(), title=None):
    cell = cell_of(s["id"], row)
    return {
        "id": f"{origin_tag(origin)}:{cell}", "cell": cell, "origin": origin,
        "title": (title or short_title(row, s)) + (f" @{row['plugin_profile']}" if "plugin_profile" in row else ""),
        "context": {f: row[f] for f in FACTORS if f != "domain"} | ({"plugin_profile": row["plugin_profile"]} if "plugin_profile" in row else {}),
        "domain": s["domain"], "subject": s["id"],
        "steps": steps_for(row, s, hook), "oracle": oracle_for(row, s, extra_oracle),
        "driver": driver_for(row, s, extra_caps), "existing_coverage": coverage(row, s),
    }


def add(origin, row, s, hook=None, extra_oracle=None, extra_caps=(), title=None):
    for r in profile_rows(s, row):
        SCEN.append(build_scenario(origin, r, s, hook, extra_oracle, extra_caps, title))


USE = Counter()


def pick_subject(row):
    cands = [s for s in SUBJ.values() if s["domain"] == row["domain"] and subject_ok(s, row)]
    cands.sort(key=lambda s: (USE[s["id"]], s["status"] != "implemented", s["id"]))
    s = cands[0]
    USE[s["id"]] += 1
    return s


PW_ROWS, PAIR_TOTAL = pairwise()
for row in PW_ROWS:
    add("pairwise", row, pick_subject(row))
N_PAIRWISE = len(SCEN)

# baseline sweep: every subject not yet used gets an ordinary-context scenario
for s in SUBJ.values():
    if USE[s["id"]]:
        continue
    client = "vanilla_java_26_1_2" if "vanilla_java_26_1_2" in s["clients"] else s["clients"][0]
    row = {"client_kind": client, "domain": s["domain"], "game_mode": "survival", "dimension": (s.get("natural_dims") or s["dims"])[0],
           "difficulty": "normal", "player_count": "2" if s["mp"] else "1", "network_condition": "normal", "server_lifecycle": "steady",
           "permission": ("console" if s["console"] and s["op"] >= 4 else f"op_{s['op']}" if s["op"] else "non_op_0")}
    USE[s["id"]] += 1
    add("baseline", row, s)
N_BASELINE = len(SCEN) - N_PAIRWISE

# ------------------------------------------------------------------ explicit high-risk triples
TRIPLE_CLIENTS = ["protocol_bot", "vanilla_java_26_1_2", "native_linux"]
tc = itertools.count()


def trow(s, **kw):
    clients = [c for c in TRIPLE_CLIENTS if c in s["clients"]] or s["clients"]
    row = {"client_kind": clients[next(tc) % len(clients)], "domain": s["domain"], "game_mode": "survival",
           "dimension": s["dims"][0], "difficulty": "normal", "player_count": "1", "network_condition": "normal",
           "server_lifecycle": "steady", "permission": f"op_{s['op']}" if s["op"] else "non_op_0"}
    row.update(kw)
    if s["mp"] and row["player_count"] == "1":
        row["player_count"] = "2"
    if not subject_ok(s, row):
        row["client_kind"] = s["clients"][0]
    return row


menus = [s for s in SUBJ.values() if s["domain"] == "containers"]
# T1 dupes: custody surfaces x dupe vectors
DUPE_MENU = {
    "two_players_same_slot_same_tick": (dict(player_count="2"), ["Two players open the same container (or own copies for per-player menus) and click the same slot in the same tick (bot timing; MCP best effort)"],
                                        ["exactly one player receives the stack; the other gets an authoritative resync; total count unchanged"], ["multi_client_2"]),
    "break_container_while_open": (dict(player_count="2"), ["Player B breaks/explodes/moves (piston, vehicle kill) the container while A has it open and clicks"],
                                   ["A's menu closes; contents spill exactly once; A's later clicks are rejected; no item both in A's inventory and on the ground"], ["multi_client_2"]),
    "kill9_between_click_and_checkpoint": (dict(server_lifecycle="crash_kill9_wal_replay"), ["Move items between container and inventory, SIGKILL server before next checkpoint, restart"],
                                           ["after WAL replay the moved stack exists in exactly one place (container XOR inventory) — either the pre-move or the post-move state, never both or neither; "
                                            "an unfenced kill may roll back the move"], []),
}
for s in menus:
    for vec, (kw, hook, orc, caps) in DUPE_MENU.items():
        row = trow(s, **kw)
        add(f"triple:dupe:{vec}", row, s, hook=hook, extra_oracle=orc, extra_caps=caps, title=f"DUPE {vec} x {s['id']}")
custody_other = [s for s in SUBJ.values() if s["custody"] and not s["menu"] and s["domain"] in
                 ("block_interact", "entities_misc", "inventory", "items", "block_break", "death_respawn", "plugins", "farming", "redstone", "fluids", "block_log")]
DUPE_OTHER = {
    "two_players_take_same_tick": (dict(player_count="2"), ["Two players take/use the same custody object (item frame, armor stand, shelf, jukebox, item entity, grave, cauldron, campfire, bucket source) in the same tick"],
                                   ["exactly one player gains the item; the other is refused/resynced"], ["multi_client_2"]),
    "destroy_during_interaction": (dict(), ["Destroy the holder (break block / kill entity / explosion) in the same tick as the take/insert"],
                                   ["item ends up either dropped or in inventory, never both"], []),
    "restart_after_interaction": (dict(server_lifecycle="clean_restart_mid_action"), ["Complete the custody step (take/insert/break-and-pick-up), then /stop and restart before any further input"],
                                  ["persisted holder and player inventory agree; no copy"], []),
}
# Subjects whose output is not a single shared finite object: renewable sources (cows/mooshrooms
# stay milkable), per-player rewards (vault), personal inventory moves and per-player transfers.
PER_PLAYER_CUSTODY = {"farming.milk_cow", "block_interact.vault_key", "inventory.swap_hands", "inventory.creative_inventory",
                      "inventory.armor_equip", "items.drink_potion_milk_honey", "death_respawn.keep_inventory_true", "plugins.economy_money_pay"}
PER_PLAYER_TAKE_ORACLE = ["each player gains exactly one output per own consumed input (personal bucket/bowl/key/item/payment); "
                          "nothing without consuming one; no cross-player transfer or doubling; the shared source changes per vanilla "
                          "(cow/mooshroom stays usable, vault records each player once)"]
assert PER_PLAYER_CUSTODY <= {s["id"] for s in custody_other}, PER_PLAYER_CUSTODY - {s["id"] for s in custody_other}
for s in custody_other:
    for vec, (kw, hook, orc, caps) in DUPE_OTHER.items():
        if vec == "two_players_take_same_tick" and s["id"] in PER_PLAYER_CUSTODY:
            orc = PER_PLAYER_TAKE_ORACLE
        row = trow(s, **kw)
        add(f"triple:dupe:{vec}", row, s, hook=hook, extra_oracle=orc, extra_caps=caps, title=f"DUPE {vec} x {s['id']}")
# T2 restart x inventories (inventory locations)
INV_LOCS = ["persistence.player_data_roundtrip", "inventory.cursor_on_close", "inventory.armor_equip", "inventory.swap_hands", "containers.ender_chest",
            "containers.chest", "containers.double_chest", "containers.shulker_box", "containers.furnace", "containers.hopper", "containers.brewing_stand",
            "containers.crafter", "containers.chest_boat", "containers.chest_minecart", "entities_misc.horses_mounts", "block_interact.item_frame_use",
            "block_interact.armor_stand_use", "block_interact.shelf", "inventory.drop_item", "plugins.graves_custody", "plugins.basic_economy_shop",
            "settlements.caravan_open", "containers.mount_inventory"]
for sid in INV_LOCS:
    s = SUBJ[sid]
    for life in ("clean_restart_mid_action", "crash_kill9_wal_replay", "hot_backup_during_action"):
        row = trow(s, server_lifecycle=life)
        add(f"triple:restart_x_inventory:{life}", row, s, title=f"RESTART {life} x inventory location {sid}")
# T3 disconnect x containers
for s in menus:
    for net in ("disconnect_during_menu", "reconnect_mid_action", "duplicate_login"):
        row = trow(s, network_condition=net)
        add(f"triple:disconnect_x_container:{net}", row, s, title=f"DISCONNECT {net} x {s['id']}")
# T4 multiplayer x same block
SAME_BLOCK = [
    ("block_break.tool_tier_required", "both players mine the same ore; finish within the same tick"),
    ("block_place.full_block", "both place into the same air cell in the same tick"),
    ("block_place.slab_double", "both place a slab into the same half-slab"),
    ("block_interact.door_trapdoor_gate_toggle", "both toggle the same door in the same tick"),
    ("block_interact.lever_button", "both flip the same lever"),
    ("farming.wheat_carrot_potato_beetroot", "both harvest the same mature crop / bone-meal it"),
    ("block_interact.axe_strip_scrape_unwax", "both strip/scrape the same log/copper"),
    ("fluids.buckets", "both bucket the same water/lava source"),
    ("explosions.tnt_prime_chain", "both prime the same TNT"),
    ("block_interact.composter", "both fill the same composter at level 7"),
    ("block_interact.campfire_cook", "both place food into the last campfire slot"),
    ("containers.chest", "both take the last stack from the same chest slot"),
    ("containers.furnace", "both take the furnace output (XP once)"),
    ("block_interact.jukebox", "both insert/eject the same jukebox"),
    ("block_interact.end_portal_eye", "both insert the 12th eye"),
    ("block_interact.vault_key", "both use keys on the same vault"),
]
for sid, how in SAME_BLOCK:
    s = SUBJ[sid]
    for pc in ("2", "N8"):
        row = trow(s, player_count=pc)
        add("triple:multiplayer_x_same_block", row, s, hook=[f"Same target: {how}"],
            extra_oracle=["exactly one outcome per vanilla tick order; losers resynced; drops/XP/items awarded once"], title=f"SAME-BLOCK p{pc} x {sid}")
# T5 restart x in-flight world state
inflight = [s for s in SUBJ.values() if s["inflight"]]
for s in inflight:
    for life in ("clean_restart_mid_action", "crash_kill9_wal_replay"):
        row = trow(s, server_lifecycle=life)
        add(f"triple:restart_x_inflight:{life}", row, s, title=f"RESTART {life} x in-flight {s['id']}")
# T6 disconnect x transient player states
TRANSIENT = ["portals_dimensions.nether_portal_travel_link", "entities_misc.boats", "entities_misc.horses_mounts", "movement.elytra_glide",
             "weather_time.sleep_skip_night", "death_respawn.death_screen_respawn", "villagers.trade_level_restock", "items.eat_food",
             "combat.bow", "combat.mob_targets_player", "movement.walk_sprint_jump", "plugins.essentials_tpa"]
for sid in TRANSIENT:
    s = SUBJ[sid]
    for net in ("reconnect_mid_action", "duplicate_login"):
        row = trow(s, network_condition=net, player_count="2" if s["mp"] else "1")
        add(f"triple:disconnect_x_transient_state:{net}", row, s, title=f"DISCONNECT {net} x transient {sid}")
# T7 observer clauses: subjects whose reference has a second-player part but no multi-player wave-1 row yet.
# The client depends only on the subject (first TRIPLE_CLIENTS lane whose driver is ready for the 2-player row,
# else the first allowed), so adding/removing one subject never moves another row's id.
for s in SUBJ.values():
    if s["observer"] and not any(sc["subject"] == s["id"] and sc["context"]["player_count"] != "1" for sc in SCEN):
        lanes = [c for c in TRIPLE_CLIENTS if c in s["clients"]] or s["clients"]
        rows = [r for c in lanes if subject_ok(s, r := trow(s, player_count="2", client_kind=c))]
        row = next((r for r in rows if driver_for(r, s, ["multi_client_2"])["status"] == "ready"), rows[0])
        add("triple:observer_clause", row, s, title=f"OBSERVER clause x {s['id']}")
N_TRIPLE = len(SCEN) - N_PAIRWISE - N_BASELINE
_bad = [sc["id"] for sc in SCEN if not subject_ok(SUBJ[sc["subject"]], {**sc["context"], "domain": sc["domain"]})]
assert not _bad, f"constraint-violating scenarios: {_bad[:10]}"

# ------------------------------------------------------------------ dimension value coverage audit
dim_values = {
    "client_kind": list(CLIENTS), "game_mode": GAME_MODES, "dimension": DIMS, "difficulty": DIFFS, "player_count": PLAYERS,
    "network_condition": list(NETWORK), "server_lifecycle": list(LIFECYCLE), "permission": list(PERMS), "domain": DOMAINS,
    "plugin_profile": list(PLUGIN_PROFILES),
}
seen = defaultdict(set)
for sc in SCEN:
    for f, v in sc["context"].items():
        seen[f].add(v)
    seen["domain"].add(sc["domain"])
    seen["subject"].add(sc["subject"])
PI_MAP = {"Join": "session.join_fresh_spawn", "PluginClientReady": "loader_content.bundle_handshake_cache", "LoaderViewRequest": "loader_content.loader_view_request",
          "PermissionLevel": "commands.op", "SetViewDistance": "session.view_distance_negotiation", "SetLocale": "session.locale_presentation",
          "SetChatVisibility": "chat.chat_visibility_setting",
          "Leave": "session.rejoin_restore_state", "Move": "movement.walk_sprint_jump", "MoveRotation": "movement.walk_sprint_jump", "MoveStatus": "movement.walk_sprint_jump",
          "ConfirmTeleport": "movement.teleport_confirm", "Dig": "block_break.tool_tier_required", "BuildPreviewFence": "settlements.build",
          "AuthorizedBuild": "settlements.build", "BuildResync": "block_place.rejected_resync", "UseItemOn": "block_place.full_block", "UseItem": "items.eat_food",
          "SpawnProtectionOperators": "block_place.spawn_protection_radius", "SelectBundleItem": "items.bundle",
          "PickItemFromBlock": "inventory.creative_inventory", "PickItemFromEntity": "inventory.creative_inventory",
          "OpenMountInventory": "containers.mount_inventory",
          "ReleaseUseItem": "combat.bow", "DropSelected": "inventory.drop_item", "SwapHands": "inventory.swap_hands", "CreativeSlot": "inventory.creative_inventory",
          "Sneak": "movement.sneak_edge", "Sprint": "movement.walk_sprint_jump", "StartFallFlying": "movement.elytra_glide", "EditBook": "items.books",
          "SignUpdate": "block_interact.sign_edit_decorate", "Attack": "combat.melee_cooldown_crit_sweep", "InteractEntity": "villagers.trade_level_restock",
          "InteractEntitySecondary": "entities_misc.leash", "VehicleInput": "entities_misc.horses_mounts", "MoveVehicle": "entities_misc.boats",
          "SelectSlot": "inventory.hotbar_select", "ContainerClick": "inventory.click_pickup", "ContainerButton": "crafting.enchanting_offers",
          "ContainerSlotState": "crafting.crafter_automation", "SelectTrade": "villagers.trade_level_restock", "SetBeacon": "special_blocks.beacon_pyramid_effects",
          "RenameItem": "crafting.anvil_repair_combine_rename", "ContainerClose": "inventory.cursor_on_close", "PlaceRecipe": "inventory.recipe_book_place",
          "Respawn": "death_respawn.death_screen_respawn", "RequestStats": "advancements_stats.statistics_request", "LeaveBed": "weather_time.sleep_skip_night",
          "Chat": "chat.public_chat", "Command": "commands.give", "DialogClick": "ui_presentation.server_dialogs", "ConsoleCommand": "commands.stop",
          "SeenAdvancements": "advancements_stats.advancement_tabs_seen"}
MENU_MAP = {"Player": "containers.player_inventory_2x2", "CraftingTable": "containers.crafting_table", "EnchantingTable": "containers.enchanting_table",
            "Anvil": "containers.anvil", "Grindstone": "containers.grindstone", "Smithing": "containers.smithing_table", "Loom": "containers.loom",
            "Stonecutter": "containers.stonecutter", "Cartography": "containers.cartography_table", "Hopper": "containers.hopper", "Dispenser": "containers.dispenser",
            "Crafter": "containers.crafter", "ShulkerBox": "containers.shulker_box", "BrewingStand": "containers.brewing_stand", "Merchant": "containers.merchant_villager",
            "Lectern": "containers.lectern", "Mount": "containers.mount_inventory",
            "Beacon": "containers.beacon", "Chest": "containers.chest", "Furnace": "containers.furnace"}
CLICK_MAP = {m: f"inventory.click_{re.sub(r'(?<!^)([A-Z])', r'_\1', m).lower()}" for m in CG["click_modes"]}
GOAL_MAP = defaultdict(list)
for s in SUBJ.values():
    for g in s.get("ai_goals", []):
        GOAL_MAP[g].append(s["id"])
GOAL_MAP["FangLife"].append("mobs.evoker")  # evoker_fangs entity lifecycle (non-living)
PLUGIN_MAP = {"solaris-permissions": "plugins.perm_groups_user_set", "solaris-essentials": "plugins.essentials_homes", "solaris-economy": "plugins.economy_money_pay",
              "solaris-towns": "plugins.towns_create_info", "solaris-audit": "plugins.audit_queries", "basic-economy": "plugins.basic_economy_shop",
              "solaris-auth": "plugins.auth_register_login", "chisel": "plugins.chisel_conversion", "graves": "plugins.graves_custody",
              "land-claims": "plugins.plugin_startup_fail_closed", "online-roster": "plugins.plugin_startup_fail_closed", "geological-mines": "plugins.plugin_startup_fail_closed",
              "computer": "monitor.monitor_give_place_break"}
PCMD_MAP = {"perm": "plugins.perm_groups_user_set", "sethome": "plugins.essentials_homes", "home": "plugins.essentials_homes", "delhome": "plugins.essentials_homes",
            "setspawn": "plugins.essentials_spawn_warps", "spawn": "plugins.essentials_spawn_warps", "setwarp": "plugins.essentials_spawn_warps",
            "warp": "plugins.essentials_spawn_warps", "delwarp": "plugins.essentials_spawn_warps", "back": "plugins.essentials_back", "tpa": "plugins.essentials_tpa",
            "tpaccept": "plugins.essentials_tpa", "msg": "plugins.essentials_msg_reply", "reply": "plugins.essentials_msg_reply", "money": "plugins.economy_money_pay",
            "pay": "plugins.economy_money_pay", "econadmin": "plugins.economy_admin", "town": "plugins.towns_create_info", "audit": "plugins.audit_queries",
            "economy": "plugins.basic_economy_shop", "register": "plugins.auth_register_login", "login": "plugins.auth_register_login",
            "changepassword": "plugins.auth_register_login", "claim": "plugins.plugin_startup_fail_closed", "who": "plugins.plugin_startup_fail_closed",
            "geology": "plugins.plugin_startup_fail_closed"}
CODE_DIMS = {
    "player_input (core:boundary.rs PlayerInput)": {v: PI_MAP.get(v) for v in CG["player_inputs"] if v != "TestGive"},
    "menu_kind (core:inventory.rs MenuKind)": {v: MENU_MAP[v] for v in CG["menu_kinds"]},
    "click_mode (core:boundary.rs ClickMode)": CLICK_MAP,
    "command_root (edge:command_tree.rs ROOTS)": {r: f"commands.{r}" for r, _ in CG["roots"]},
    "damage_kind (core:survival.rs DamageKind)": {k: f"damage.{k}" for k in CG["kinds"]},
    "mob_effect (mc-data MobEffect)": {e: f"effects.{e}" for e in CG["effects"]},
    "gamerule (core:command_rules.rs RULES, canonical 26.1.2 ids)": {r: f"gamerules.{r}" for r, _ in CG["rules"]},
    "living_entity (mc-data entity contracts)": {n: f"mobs.{n}" for n, _, _ in CG["living"]},
    "nonliving_entity (mc-data entity contracts)": {n: CG["map_nonliving"](n) for n in CG["nonliving"]},
    "mob_category (mc-data MobCategory)": {c: f"spawning.natural_{c}" for c in CG["mob_categories"]},
    "ai_goal (core:blueprint.rs GoalKind)": {g: (GOAL_MAP.get(g) or [None])[0] for g in CG["goal_kinds"]},
    "villager_profession (core:villager/mod.rs)": {p: f"villagers.profession_{p}" for p in CG["professions"]},
    "settlement_subcommand (edge:command_tree.rs SETTLEMENT_HELP/ECONOMY)": {**{h: f"settlements.{h.replace(' ', '_')}" for h, _ in CG["settlement_help"]},
                                                                             **{f"{a} {b}": f"settlements.{a}_{b}" for a, b, _ in CG["settlement_eco"]}},
    "plugin_package (../solaris-default-plugins/*/plugin.toml)": {p: PLUGIN_MAP[p] for p in CG["plugins"]},
    "plugin_command (plugin.toml player_commands)": {c: PCMD_MAP[c] for p in CG["plugins"].values() for c in p["commands"]},
    "weather (core:commands.rs Weather)": {"Clear": "weather_time.weather_cycle", "Rain": "weather_time.weather_cycle", "Thunder": "weather_time.lightning_effects"},
    "selector": {k.split(".", 1)[1]: k for k in SUBJ if k.startswith("selectors.")},
}
audit_missing = []
for dname, mapping in CODE_DIMS.items():
    for v, sid in mapping.items():
        if sid is None or sid not in SUBJ or sid not in seen["subject"]:
            audit_missing.append(f"{dname}: {v} -> {sid}")
for f, vals in dim_values.items():
    for v in vals:
        if v not in seen[f]:
            audit_missing.append(f"{f}: {v}")
for sid in SUBJ:
    if sid not in seen["subject"]:
        audit_missing.append(f"subject: {sid}")
assert not audit_missing, audit_missing

# ------------------------------------------------------------------ full combinatorial set: counts, waves, lazy queue
_dup = [i for i, n in Counter(sc["id"] for sc in SCEN).items() if n > 1]
assert not _dup, f"duplicate wave-1 ids: {_dup[:5]}"
SCEN_BY_ID = {sc["id"]: sc for sc in SCEN}
W1_CELLS = {sc["cell"] for sc in SCEN}
W1_GROUP = {"pw": "pairwise", "bl": "baseline"}
# Value orders used by the queue: most bug-revealing / cheapest-to-drive first.
PRIORITY = {
    "server_lifecycle": ["crash_kill9_wal_replay", "clean_restart_mid_action", "hot_backup_during_action", "save_all_checkpoint", "pause_when_empty_resume", "steady"],
    "network_condition": ["disconnect_during_menu", "duplicate_login", "reconnect_mid_action", "malformed_or_stale_packet", "slow_reader_backpressure", "normal"],
    "player_count": ["2", "N8", "1"],
    "client_kind": ["protocol_bot", "vanilla_java_26_1_2", "native_linux", "fabric_solaris_loader", "fabric_distant_horizons", "fabric_svc",
                    "neoforge_solaris_loader", "forge_solaris_loader", "native_windows"],
    "game_mode": ["survival", "creative", "adventure", "spectator"],
    "difficulty": ["normal", "hard", "easy", "peaceful"],
    "permission": ["non_op_0", "op_4", "op_2", "op_3", "op_1", "console"],
    "dimension": ["overworld", "the_nether", "the_end"],
}
assert all(sorted(PRIORITY[f]) == sorted(VALUES[f]) for f in CELL_FACTORS)
ENUM_ORDER = ["server_lifecycle", "network_condition", "player_count", "client_kind", "game_mode", "difficulty", "permission", "dimension"]  # outer->inner; subject innermost
WAVES = {
    1: "wave 1: the matrix.yaml scenarios (pairwise + baseline + explicit high-risk triples)",
    2: "dupes: custody subject (items change owner) with 2/8 players, a non-normal network condition or a non-steady server lifecycle",
    3: "saves: any non-steady server lifecycle (save-all, clean restart, kill -9 + WAL replay, hot backup, pause/resume)",
    4: "disconnects: any non-normal network condition",
    5: "multiplayer: 2 or 8 players",
    6: "rest: 1 player, normal network, steady server; varies client, game mode, dimension, difficulty, permission",
}


def tier(s, pc, net, life):
    """Queue wave (2..6) of a full-set cell; first matching rule wins."""
    if s["custody"] and (pc != "1" or net != "normal" or life != "steady"):
        return 2
    if life != "steady":
        return 3
    if net != "normal":
        return 4
    if pc != "1":
        return 5
    return 6


def cell_wave(cell):
    if cell in W1_CELLS:
        return 1
    s, row = parse_cell(cell)
    return tier(s, row["player_count"], row["network_condition"], row["server_lifecycle"])


ALLOWED = {sid: {f: frozenset(v for v in VALUES[f] if subject_ok(s, {f: v})) for f in CELL_FACTORS} for sid, s in SUBJ.items()}
INVALID_ORDER = [("C01", "player_count"), ("C02", "dimension"), ("C03", "network_condition"), ("C04", "client_kind"), ("C05", "permission")]


def full_counts():
    """Exact counts: unconstrained product, removals by first violated constraint, valid cells per domain and wave.
    A plugin-sensitive subject counts once per plugin profile (its cells carry @vanilla/@live)."""
    size = {f: len(VALUES[f]) for f in CELL_FACTORS}
    per_subject_all = 1
    for f in CELL_FACTORS:
        per_subject_all *= size[f]
    removed = Counter()
    by_domain = defaultdict(Counter)
    for sid, s in SUBJ.items():
        a = ALLOWED[sid]
        variants = max(1, len(s["profiles"]))
        prefix = 1
        for i, (cid, f) in enumerate(INVALID_ORDER):
            rest = 1
            for g in CELL_FACTORS:
                if g not in [x for _, x in INVALID_ORDER[:i + 1]]:
                    rest *= size[g]
            removed[cid] += variants * prefix * (size[f] - len(a[f])) * rest
            prefix *= len(a[f])
        other = len(a["client_kind"]) * len(a["game_mode"]) * len(a["dimension"]) * len(a["difficulty"]) * len(a["permission"])
        for pc in a["player_count"]:
            for net in a["network_condition"]:
                for life in a["server_lifecycle"]:
                    by_domain[s["domain"]][tier(s, pc, net, life)] += variants * other
    for cell in W1_CELLS:
        s, row = parse_cell(cell)
        by_domain[s["domain"]][tier(s, row["player_count"], row["network_condition"], row["server_lifecycle"])] -= 1
        by_domain[s["domain"]][1] += 1
    valid_total = sum(sum(c.values()) for c in by_domain.values())
    unconstrained = sum(max(1, len(s["profiles"])) for s in SUBJ.values()) * per_subject_all
    assert unconstrained - sum(removed.values()) == valid_total, "count mismatch"
    waves = Counter()
    for c in by_domain.values():
        waves.update(c)
    return {"unconstrained": unconstrained, "contexts_per_subject": per_subject_all, "removed_by_first_violated": dict(removed),
            "plugin_sensitive_subjects": sum(1 for s in SUBJ.values() if s["profiles"]), "plugin_profiles": list(PLUGIN_PROFILES),
            "valid": valid_total, "wave1_unique_cells": len(W1_CELLS), "wave1_scenarios": len(SCEN),
            "waves": {w: waves[w] for w in sorted(WAVES)},
            "by_domain": {d: {"valid": sum(by_domain[d].values()), **{f"wave{w}": by_domain[d][w] for w in sorted(WAVES)}} for d in DOMAINS}}


def wave1_order():
    rank = {"triple:dupe": 0, "triple:restart_x_inventory": 1, "triple:disconnect_x_container": 2, "triple:restart_x_inflight": 3,
            "triple:disconnect_x_transient_state": 4, "triple:multiplayer_x_same_block": 5, "triple:observer_clause": 6, "pairwise": 7, "baseline": 8}
    return sorted(SCEN, key=lambda sc: next(v for k, v in rank.items() if sc["origin"].startswith(k)))  # stable: keeps generation order


def queue(waves, clients=None, drivers=None, domains=None):
    """Lazily yield (id, wave): wave 1 scenarios, then F: cells of waves 2..6 not already in wave 1."""
    def keep(client, domain):
        return ((not clients or client in clients) and (not drivers or CLIENTS[client]["driver"] in drivers)
                and (not domains or domain in domains))
    if 1 in waves:
        for sc in wave1_order():
            if keep(sc["context"]["client_kind"], sc["domain"]):
                yield sc["id"], 1
    subj = [s for s in SUBJ.values() if not domains or s["domain"] in domains]
    groups = {True: [s for s in subj if s["custody"]], False: [s for s in subj if not s["custody"]]}
    for w in sorted(set(waves) - {1}):
        for ctx in itertools.product(*[PRIORITY[f] for f in ENUM_ORDER]):
            row = dict(zip(ENUM_ORDER, ctx))
            if (clients and row["client_kind"] not in clients) or (drivers and CLIENTS[row["client_kind"]]["driver"] not in drivers):
                continue
            for custody, members in groups.items():
                probe = {"custody": custody}
                if tier(probe, row["player_count"], row["network_condition"], row["server_lifecycle"]) != w:
                    continue
                for s in members:
                    a = ALLOWED[s["id"]]
                    if all(row[f] in a[f] for f in ("client_kind", "player_count", "network_condition", "permission", "dimension")):
                        for r in profile_rows(s, row):
                            cell = cell_of(s["id"], r)
                            if cell not in W1_CELLS:
                                yield f"F:{cell}", w


def show(sid):
    if sid in SCEN_BY_ID:
        return {**SCEN_BY_ID[sid], "wave": 1, "wave_desc": WAVES[1]}
    parsed = parse_id(sid)
    if not parsed:
        raise SystemExit(f"unknown or constraint-violating id: {sid}")
    tag, cell, s, row = parsed
    if tag != "F":
        raise SystemExit(f"wave-1 id not in the current matrix (stale?): {sid}")
    w = cell_wave(cell)
    sc = build_scenario("full", row, s)
    return {**sc, "wave": w, "wave_desc": WAVES[w], **({"also_in_wave1": sorted(i for i, x in SCEN_BY_ID.items() if x["cell"] == cell)} if w == 1 else {})}


def status_report():
    latest, errors, legacy, outdated = read_ledger()
    groups = {"pairwise": [], "baseline": [], "triples": []}
    for sc in SCEN:
        groups[W1_GROUP.get(sc["id"].split(":", 1)[0][:2], "triples")].append(sc["id"])
    stale = sorted(i for i in latest if not i.startswith("F:") and i not in SCEN_BY_ID)
    rows = []

    def line(name, total, ids_status):
        c = Counter(ids_status)
        executed = c["pass"] + c["fail"]
        rows.append((name, total, c["pass"], c["fail"], c["blocked"], c["skipped"], executed, total - executed))

    for g, ids in groups.items():
        line(f"wave1 {g}", len(ids), [latest[i]["status"] for i in ids if i in latest])
    line("wave1 total", len(SCEN), [latest[i]["status"] for i in SCEN_BY_ID if i in latest])
    counts = full_counts()
    f_status = defaultdict(list)
    for i, o in latest.items():
        if i.startswith("F:"):
            f_status[cell_wave(i[2:])].append(o["status"])
    for w in range(2, 7):
        line(f"wave{w} {WAVES[w].split(':')[0]}", counts["waves"][w], f_status[w])
    cell_best = {}
    for i, o in latest.items():
        cell = i.split(":", 1)[1]
        prev = cell_best.get(cell)
        # a cell is executed when any of its ids has a verdict; fail outranks pass for reporting
        rank = {"fail": 0, "pass": 1, "blocked": 2, "skipped": 3}[o["status"]]
        if prev is None or rank < prev[0]:
            cell_best[cell] = (rank, o["status"])
    line("full set (valid cells)", counts["valid"], [v[1] for v in cell_best.values()])
    per_client = defaultdict(list)
    for sc in SCEN:
        per_client[sc["context"]["client_kind"]].append(latest[sc["id"]]["status"] if sc["id"] in latest else None)
    print(f"ledger: {LEDGER} ({len(latest)} ids, {len(errors)} invalid lines, {len(stale)} stale wave-1 ids, "
          f"{len(legacy)} legacy profile-less lines, {len(outdated)} outdated plugin-set lines)")
    hdr = ("group", "total", "pass", "fail", "blocked", "skipped", "executed", "remaining")
    width = max(len(r[0]) for r in rows + [hdr])
    for r in [hdr] + rows:
        print(f"{r[0]:<{width}}  " + "  ".join(f"{x:>12}" for x in r[1:]))
    print("\nwave 1 by client_kind (executed/total; blocked; skipped):")
    for client, st in sorted(per_client.items()):
        c = Counter(st)
        print(f"  {client:<26} {c['pass'] + c['fail']:>5}/{len(st):<5} blocked {c['blocked']:<4} skipped {c['skipped']}")
    for e in errors[:20]:
        print("ERROR", e)
    for i in stale[:20]:
        print("STALE (wave-1 id not in current matrix; its cell still counts)", i)
    for i in legacy[:20]:
        print("LEGACY (recorded before its subject got plugin profiles; not counted, rerun as @vanilla/@live)", i)
    for i in outdated[:20]:
        print("OUTDATED (ran on a plugin build that is no longer the profile's fingerprint; not counted, rerun)", i)
    return 1 if errors else 0


def _csv(v):
    return {x for x in v.split(",") if x} if v else None


if ARGS.mode == "full":
    waves = {int(w) for w in _csv(ARGS.wave)} if ARGS.wave else set(WAVES)
    for name, val, allowed in (("--wave", waves, set(WAVES)), ("--client", _csv(ARGS.client), set(CLIENTS)),
                               ("--driver", _csv(ARGS.driver), set(DRIVERS)), ("--domain", _csv(ARGS.domain), set(DOMAINS))):
        if val and not val <= allowed:
            sys.exit(f"{name}: unknown {sorted(val - allowed)}")
    done = set()
    if ARGS.pending:
        latest, errors, _legacy, _outdated = read_ledger()
        if errors:
            sys.exit("ledger has invalid lines; run --status")
        done = {i for i, o in latest.items() if o["status"] in ("pass", "fail")}
    try:
        emitted = 0
        for sid, w in queue(waves, _csv(ARGS.client), _csv(ARGS.driver), _csv(ARGS.domain)):
            if sid in done:
                continue
            sys.stdout.write(f"{w}\t{sid}\n")
            emitted += 1
            if ARGS.limit and emitted >= ARGS.limit:
                break
        sys.stdout.flush()
    except BrokenPipeError:
        os.dup2(os.open(os.devnull, os.O_WRONLY), sys.stdout.fileno())
    sys.exit(0)
if ARGS.mode == "show":
    print(json.dumps(show(ARGS.show), ensure_ascii=False, indent=1))
    sys.exit(0)
if ARGS.mode == "status":
    sys.exit(status_report())


# ------------------------------------------------------------------ outputs
def git_head(p):
    try:
        return subprocess.run(["git", "-C", str(p), "rev-parse", "--short", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    except Exception:
        return "unknown"


heads = {r: git_head(HOME / r) for r in ("solaris", "solaris-default-plugins", "solaris-loader", "solaris-client")}
cov = Counter(sc["existing_coverage"]["status"] for sc in SCEN)
drv = Counter(sc["driver"]["status"] for sc in SCEN)
FULL = full_counts()
LEDGER_CONTRACT = {
    "file": str(LEDGER),
    "format": "JSON Lines, append-only; one object per executed scenario attempt; the LAST line for an id is its current status; never rewrite or delete lines",
    "keys": {"id": "wave-1 id (<pw|bl|tr.*>:CELL) or full-set id (F:CELL); CELL ends @vanilla|@live for plugin-sensitive subjects",
             "status": "pass | fail | blocked | skipped",
             "driver": "protocol_bot | java_mcp | loader_java_mcp | native_mcp; must equal the driver of the id's client_kind",
             "server_sha": "full sha256 of the solaris-v2 binary used (release); no spaces",
             "run_dir": "absolute path of the harness run artifact dir (result.json/logs)",
             "ts": "UTC ISO-8601 ending in Z",
             "note": "fail: finding + evidence path; blocked: missing capability name(s) from `capabilities`; may be empty for pass/skipped",
             "plugin_set": "OPTIONAL key, REQUIRED for @profile ids: sha256 printed by `gen_matrix.py --plugin-fingerprint <deployed [plugins].directory>`; "
                           "--record/--record-file reject it unless it equals the fingerprint of the id's profile (see the scenario's steps)"},
    "exact_keys": list(LEDGER_KEYS),
    "optional_keys": list(LEDGER_OPTIONAL_KEYS),
    "plugin_profiles": {sid: {"changed_by": deps, "vanilla": profile_fingerprint(sid, "vanilla"), "live": profile_fingerprint(sid, "live")}
                        for sid, (deps, _) in PLUGIN_SENSITIVE.items()},
    "writers": "harness receipts -> `python3 gen_matrix.py --record ...` or `--record-file batch.jsonl` (validates, flock(LOCK_EX) on ledger.jsonl, one O_APPEND write, fsync, unlock); direct writers must do the same and never hold the lock across a scenario run",
    "executed": "pass or fail; blocked/skipped stay remaining; a full-set cell counts as executed when any id carrying that CELL (wave-1 or F:) has pass/fail",
    "validation": "`python3 gen_matrix.py --status` exits 1 on any invalid line (bad keys/status/driver/ts, unknown subject or constraint-violating cell)",
}
matrix = {
    "schema": "solaris-scenario-matrix/2",
    "generated": "2026-10-24",
    "generator": "tools/harness/matrix_specs/gen_matrix.py (+ subjects*.py, codegen_subjects.py, existing.py); deterministic seed 20261024",
    "repo_heads": heads,
    "id_scheme": {
        "cell": "subject/" + "/".join(CELL_FACTORS),
        "wave1_id": "<tag>:<cell>; tag = pw (pairwise) | bl (baseline) | tr.<triple origin with ':'->'.'>",
        "full_set_id": "F:<cell>",
        "stability": "content-derived (subject + context values), independent of generation order",
    },
    "full_set": {
        "definition": "every subject x every value of the 8 context factors, minus invalid constraints C01-C05; negative-oracle (C06-C10), server-gap (C11) and procedure (C12) cells stay in the set",
        **{k: v for k, v in FULL.items() if k != "by_domain"},
        "wave_definitions": WAVES,
        "queue_order": {"waves": "1, then 2..6", "wave1": "triples (dupes, restart x inventory, disconnect x container, restart x in-flight, disconnect x transient, same block), then pairwise, then baseline",
                        "waves_2_6": "context loops outer->inner " + " > ".join(ENUM_ORDER) + ", subject innermost; F: cells already in wave 1 are skipped",
                        "value_priority": PRIORITY},
        "by_domain": FULL["by_domain"],
    },
    "ledger_contract": LEDGER_CONTRACT,
    "method": {
        "pairwise_factors": FACTORS,
        "pairwise": "greedy AETG over all valid value pairs of the 9 factors (subject-level constraints projected onto client/player_count/network/permission/dimension); each row then takes the least-used compatible subject of its domain",
        "baseline": "every subject not reached by pairwise gets one ordinary-context scenario (vanilla client when allowed, survival, normal, steady)",
        "triples": "explicit high-risk triples: dupes x custody surfaces, restart x inventory locations, disconnect x containers, multiplayer x same block, restart x in-flight state, disconnect x transient player state",
        "coverage_rule": "covered = an existing real-server test lists the subject (not admission-only) with the same client_kind, player_count, network_condition and lifecycle; partial = subject exercised by a real-server test in another context or admission-only; unit_only = only in-process *_tests.rs; none",
        "driver_rule": "client_kind selects the driver; required capabilities = domain + subject + context; A=proven by an existing scenario, P=tool/packet exists but no scenario exercises it (listed as unproven), M=missing (listed as missing)",
    },
    "counts": {"scenarios": len(SCEN), "pairwise": N_PAIRWISE, "baseline": N_BASELINE, "triples": N_TRIPLE, "subjects": len(SUBJ), "domains": len(DOMAINS),
               "valid_pairs_covered": PAIR_TOTAL, "coverage": dict(cov), "driver_status": dict(drv), "full_set_valid_cells": FULL["valid"]},
    "dimensions": {
        "client_kind": [{"id": k, **v} for k, v in CLIENTS.items()],
        "game_mode": {"values": GAME_MODES, "source": "crates/mc-core/src/commands.rs GameMode"},
        "dimension": {"values": DIMS, "source": "crates/mc-core/src/dimensions.rs Dimension"},
        "difficulty": {"values": DIFFS, "source": "crates/mc-core/src/survival.rs Difficulty"},
        "player_count": {"values": PLAYERS, "note": "N8 = 8 concurrent (catalog-load fleets go larger)"},
        "network_condition": [{"id": k, "desc": v} for k, v in NETWORK.items()],
        "server_lifecycle": [{"id": k, "desc": v} for k, v in LIFECYCLE.items()],
        "permission": [{"id": k, "level": v} for k, v in PERMS.items()],
        "domain": {d: {"sources": [expand_src(x) for x in subjects.D[d]["sources"]], "subjects": [
            {k: v for k, v in s.items() if k not in ("domain",) and v not in (False, [], None) or k in ("op",)}
            for s in SUBJ.values() if s["domain"] == d]} for d in DOMAINS},
        "code_enumerated_values": CODE_DIMS,
    },
    "constraints": CONSTRAINTS,
    "capabilities": {c: {"status": CAP_STATUS[c], "evidence": CAP_NOTE[c]} for c in CAPS},
    "drivers": DRIVER_DESC,
    "existing_real_server_tests": E2E,
    "scenario_template": SCENARIO_TEMPLATE,
    "scenarios": SCEN,
}


class Dumper(yaml.SafeDumper):
    def ignore_aliases(self, data):
        return True


FREEZE = OUT / "MATRIX_FREEZE.json"
WAVE1_IDS = sorted(x["id"] for x in SCEN)
WAVE1_ID_HASH = hashlib.sha256("\n".join(WAVE1_IDS).encode()).hexdigest()
frozen = json.loads(FREEZE.read_text()) if FREEZE.exists() else None
if frozen and frozen["wave1_id_set_sha256"] != WAVE1_ID_HASH and not ARGS.refreeze:
    old = set(frozen["wave1_ids"])
    sys.exit(f"wave-1 id set changed vs {FREEZE.name} (frozen {frozen['frozen_at']}): +{len(set(WAVE1_IDS) - old)} -{len(old - set(WAVE1_IDS))} ids; "
             "matrix.yaml not written. Keep ids stable, or rerun with --refreeze and announce the new freeze to the runners.")
matrix_bytes = yaml.dump(matrix, Dumper=Dumper, sort_keys=False, allow_unicode=True, width=200).encode()
(OUT / "matrix.yaml").write_bytes(matrix_bytes)
keep = frozen and frozen["wave1_id_set_sha256"] == WAVE1_ID_HASH
FREEZE.write_text(json.dumps({
    "frozen_at": frozen["frozen_at"] if keep else datetime.now().astimezone().isoformat(timespec="seconds"),
    "wave1_id_set_sha256": WAVE1_ID_HASH,
    "id_set_hash_rule": "sha256 of the sorted wave-1 scenario ids joined by '\\n' (no trailing newline)",
    "matrix_yaml_sha256": hashlib.sha256(matrix_bytes).hexdigest(),
    "wave1_scenarios": len(WAVE1_IDS),
    "by_client": dict(sorted(Counter(x["context"]["client_kind"] for x in SCEN).items())),
    "rule": "the wave-1 id set is frozen; corrections that only change oracle/notes/status text change matrix_yaml_sha256, "
            "never wave1_id_set_sha256. A changed id set needs --refreeze and an announcement to the runners.",
    "wave1_ids": WAVE1_IDS,
}, indent=1) + "\n")
LEDGER.touch(exist_ok=True)

# ------------------------------------------------------------------ summary
dom_rows = []
for d in DOMAINS:
    sc = [x for x in SCEN if x["domain"] == d]
    c = Counter(x["existing_coverage"]["status"] for x in sc)
    subs = [s for s in SUBJ.values() if s["domain"] == d]
    subj_cov = Counter()
    for s in subs:
        best = "none"
        if any(not p for _, p in E2E_BY_SUBJ.get(s["id"], [])):
            best = "e2e"
        elif E2E_BY_SUBJ.get(s["id"]):
            best = "e2e_partial"
        elif s["unit_tests"]:
            best = "unit_only"
        subj_cov[best] += 1
    ready = sum(1 for x in sc if x["driver"]["status"] == "ready")
    gaps = sum(1 for s in subs if s["status"] != "implemented")
    dom_rows.append((d, len(subs), len(sc), c["covered"], c["partial"], c["unit_only"], c["none"], subj_cov["e2e"], subj_cov["e2e_partial"], subj_cov["unit_only"], subj_cov["none"], ready, gaps))

missing_ctr = Counter()
sole = Counter()
for x in SCEN:
    m = x["driver"].get("missing", [])
    for c in m:
        missing_ctr[c] += 1
    if len(m) == 1:
        sole[m[0]] += 1
# greedy unlock order: pick capability that turns the most scenarios fully driveable
remaining = [set(x["driver"].get("missing", [])) for x in SCEN if x["driver"].get("missing")]
order = []
chosen = set()
while True:
    gains = Counter()
    for m in remaining:
        left = m - chosen
        if left and len(left) <= 3:
            for c in left:
                gains[c] += 1.0 / len(left)
    if not gains:
        break
    c, _ = gains.most_common(1)[0]
    chosen.add(c)
    unlocked = sum(1 for m in remaining if m <= chosen)
    order.append((c, unlocked))
    if len(order) >= 15:
        break

pct = lambda a, b: f"{100.0 * a / b:.1f}%" if b else "-"
tot = len(SCEN)
subj_e2e = sum(r[7] for r in dom_rows)
subj_part = sum(r[8] for r in dom_rows)
subj_unit = sum(r[9] for r in dom_rows)
subj_none = sum(r[10] for r in dom_rows)
gap_subjects = [s for s in SUBJ.values() if s["status"] != "implemented"]
lines = [
    "# Solaris gameplay/server scenario matrix — summary",
    "",
    f"Generated 2026-10-24 by `tools/harness/matrix_specs/gen_matrix.py` (deterministic, seed 20261024) from code at "
    + ", ".join(f"{k}@{v}" for k, v in heads.items()) + ". Machine-readable source: `matrix.yaml`.",
    "",
    "## Totals",
    "",
    f"- Dimensions: 8 context factors + {len(DOMAINS)} gameplay domains with {len(SUBJ)} subjects; {sum(len(v) for v in CODE_DIMS.values())} code-enumerated values "
    f"({', '.join(k.split(' ')[0] for k in CODE_DIMS)}) each mapped to a subject.",
    f"- Scenarios: **{tot}** = {N_PAIRWISE} pairwise (covers all {PAIR_TOTAL} valid value pairs of the 9 factors) + {N_BASELINE} baseline (every remaining subject once) + {N_TRIPLE} explicit high-risk triples.",
    "- Every value of every dimension (context values, domains, subjects, and every code-enumerated value) appears in at least one scenario (asserted by the generator).",
    f"- Existing real-server coverage of scenarios: covered {cov['covered']} ({pct(cov['covered'], tot)}), partial {cov['partial']} ({pct(cov['partial'], tot)}), "
    f"unit-only {cov['unit_only']} ({pct(cov['unit_only'], tot)}), none {cov['none']} ({pct(cov['none'], tot)}).",
    f"- Existing coverage of subjects (any context): real-server e2e {subj_e2e} ({pct(subj_e2e, len(SUBJ))}), e2e admission-only {subj_part} ({pct(subj_part, len(SUBJ))}), "
    f"unit-only {subj_unit} ({pct(subj_unit, len(SUBJ))}), none {subj_none} ({pct(subj_none, len(SUBJ))}).",
    f"- Driveable today: ready {drv['ready']} ({pct(drv['ready'], tot)}), plausible (tools exist, unexercised) {drv['plausible']} ({pct(drv['plausible'], tot)}), "
    f"missing capability {drv['missing_capability']} ({pct(drv['missing_capability'], tot)}).",
    "",
    "## Per domain",
    "",
    "| domain | subjects | scenarios | covered | partial | unit-only | none | subj e2e | subj e2e-partial | subj unit-only | subj none | driver ready | server gaps |",
    "|---|---|---|---|---|---|---|---|---|---|---|---|---|",
]
for r in dom_rows:
    lines.append("| " + " | ".join(str(x) for x in r) + " |")
lines += ["", "Scenario coverage % counts only the exact context match; `subj e2e` counts subjects with at least one real-server test in any context.", "",
          "## Top missing bot capabilities (ordered by scenarios unlocked)", "",
          "Greedy order: each step adds the capability that makes the most additional scenarios fully driveable (scenarios with <= 3 missing capabilities considered).", "",
          "| # | capability | cumulative scenarios unlocked | scenarios blocked by it | sole blocker | drivers lacking it (bot/java/loader/native) | evidence |",
          "|---|---|---|---|---|---|---|"]
for i, (c, unl) in enumerate(order, 1):
    lines.append(f"| {i} | `{c}` | {unl} | {missing_ctr[c]} | {sole[c]} | {CAPS[c][0]} | {CAP_NOTE[c]} |")
rest = [c for c, _ in missing_ctr.most_common() if c not in chosen]
if rest:
    lines += ["", "Other missing capabilities by scenarios blocked: " + ", ".join(f"`{c}` {missing_ctr[c]}" for c in rest)]
unproven_ctr = Counter(c for x in SCEN for c in x["driver"].get("unproven", []))
lines += ["", "### Unproven capabilities (tool/packet exists for the scenario's driver, but no existing scenario exercises it)", "",
          "| capability | scenarios needing proof | drivers (bot/java/loader/native) | evidence |", "|---|---|---|---|"]
for c, n in unproven_ctr.most_common():
    lines.append(f"| `{c}` | {n} | {CAPS[c][0]} | {CAP_NOTE[c]} |")
lines += ["", "## Server gaps surfaced while enumerating (vanilla behaviour with no server code hit)", ""]
for s in gap_subjects:
    lines.append(f"- `{s['id']}` ({s['status']}): {s['expect']}")
lines += ["",
          "## Divergences/notes found in code (not memory)", "",
          "- Duplicate login: `crates/mc-core/src/tick/input/session.rs` join refuses the NEW connection ('UUID is already joined'); vanilla kicks the EXISTING session. Decide policy; triples `disconnect_x_*:duplicate_login` encode both.",
          "- `DamageKind` (crates/mc-core/src/survival.rs) has no freeze, fly_into_wall, cramming, sonic_boom, dragon_breath or stalagmite kinds; listed as `damage.vanilla_*` gap subjects.",
          "- Illusioner falls to the generic `Melee` combat policy (crates/mc-data/src/mob_behavior_26_1_2.rs combat_policy); vanilla illusioner is a ranged spellcaster.",
          "- No server-side Simple Voice Chat bridge in any crate; `fabric_svc` can only test join/compat today.",
          "- Computer terminal exists only in Fabric Loader (loader-core ComputerProtocol, loader-fabric ComputerClient); native client has no `solaris:computer`; screen sharing to other players is excluded by ADR 0018.",
          "- No horse/donkey/llama inventory screen and no lectern menu in `MenuKind` / container mapping (crates/mc-core/src/tick.rs container_kind_for_state).",
          "- No spawn-protection key in server.properties support (crates/mc-edge/src/server_properties.rs).",
          "- Legacy plugin manifests land-claims/online-roster/geological-mines declare api 0.6.0 and are not deployable on 0.8 (scenario `plugins.plugin_startup_fail_closed`).",
          "",
          "## Full combinatorial set (all combinations)", "",
          f"- Unconstrained product: {len(SUBJ)} subjects x {FULL['contexts_per_subject']:,} contexts (9 clients x 4 modes x 3 dimensions x 4 difficulties x 3 player counts x 6 network x 6 lifecycle x 6 permissions) = **{FULL['unconstrained']:,}**.",
          "- Removed by the invalid constraints (each cell counted once, at the first violated rule): "
          + ", ".join(f"{k} {v:,}" for k, v in FULL["removed_by_first_violated"].items()) + ".",
          f"- **Valid combinations: {FULL['valid']:,}** (negative-oracle, server-gap and procedure cells included). Wave 1 = {len(SCEN)} scenarios on {len(W1_CELLS)} distinct cells.",
          "",
          "| wave | definition | cells |", "|---|---|---|"]
for w, desc in WAVES.items():
    lines.append(f"| {w} | {desc} | {FULL['waves'][w]:,} |")
lines += ["", "| domain | valid cells | " + " | ".join(f"wave {w}" for w in WAVES) + " |", "|---|---|" + "---|" * len(WAVES)]
for d, c in FULL["by_domain"].items():
    lines.append(f"| {d} | {c['valid']:,} | " + " | ".join(f"{c[f'wave{w}']:,}" for w in WAVES) + " |")
lines += [f"| **total** | **{FULL['valid']:,}** | " + " | ".join(f"**{FULL['waves'][w]:,}**" for w in WAVES) + " |", "",
          "Ids: CELL = `subject/client_kind/game_mode/dimension/difficulty/player_count/network_condition/server_lifecycle/permission`; "
          "wave-1 id = `<pw|bl|tr.<origin>>:CELL`; full-set id = `F:CELL`. Content-derived, so they stay stable.",
          "Queue: `python3 gen_matrix.py --full [--wave 2] [--client protocol_bot] [--driver native_mcp] [--domain containers] [--pending] [--limit N]` "
          "streams `wave<TAB>id` lazily (no file of all cells); waves 2..6 iterate contexts outer->inner " + " > ".join(ENUM_ORDER)
          + " with subjects innermost, so the first ids of wave 2 are kill-9/disconnect-during-menu dupes on every custody subject.",
          "",
          "## Ledger contract (`ledger.jsonl`)", "",
          f"- One JSON object per line with exactly the keys `{', '.join(LEDGER_KEYS)}`; append-only; the last line for an id wins.",
          "- `status`: pass | fail | blocked | skipped. `driver` must equal the driver of the id's client_kind. `server_sha`: full sha256 of the release solaris-v2 binary. "
          "`run_dir`: absolute harness run dir. `ts`: UTC ISO-8601 with `Z`. `note`: fail -> finding + evidence path; blocked -> missing capability name(s).",
          "- Write through `python3 gen_matrix.py --record --id ID --status S --driver D --server-sha SHA --run-dir DIR [--note T]` or `--record-file batch.jsonl` "
          "(validates, flock on ledger.jsonl, one O_APPEND write, fsync). Ledger lines come from harness receipts; this tool does not run scenarios.",
          "- `python3 gen_matrix.py --status` prints executed (pass+fail) / remaining for pairwise, baseline, triples, wave 1, waves 2..6 and the full set, plus wave 1 per client; exits 1 on any invalid line.",
          "",
          "## How to use", "",
          "- Execution path: tools/harness (single `cmd_run`, shared lifecycle/receipts, catalog_mixes for composite bot scenarios) on the release binary with "
          f"`[plugins].directory = {LIVE_PLUGINS}` (live VPS set of 10 plugins; see `scenario_template` for its global effects: auth login, graves, essentials /msg).",
          "- Pick from `--full --wave 1 --client <yours> --pending`; `--show ID` prints the scenario (steps/oracle/driver/coverage) for any wave-1 or F: id.",
          "- Re-generate after code changes: `python3 tools/harness/matrix_specs/gen_matrix.py` (the external mutable ledger is never truncated).",
          f"- Freeze: `MATRIX_FREEZE.json` pins the wave-1 id set (`wave1_id_set_sha256` {WAVE1_ID_HASH[:16]}…, frozen {json.loads(FREEZE.read_text())['frozen_at']}); "
          "a regenerate that would change it exits 1 without writing; `--refreeze` accepts the change and must be announced to the runners. Record the id-set hash per batch.",
          f"- Plugin profiles: {len(PLUGIN_SENSITIVE)} plugin-sensitive subjects ({', '.join(f'{k} [{', '.join(v[0])}]' for k, v in PLUGIN_SENSITIVE.items())}) "
          "carry @vanilla (live set minus the listed plugins) or @live (full live set) in every id; their ledger lines need "
          "`plugin_set` = `gen_matrix.py --plugin-fingerprint <deployed dir>` (pass `--plugin-set FP` to --record); expected fingerprints are in `ledger_contract.plugin_profiles`. "
          "Profile-less lines recorded earlier for these subjects are reported as LEGACY by --status and not counted; "
          "lines whose plugin_set is no longer their profile's fingerprint (an older plugin build) are reported as OUTDATED and not counted.",
          "- Coverage cites real-server tests from `existing.py` (harness profiles/scenario ids) and unit test files by module; `e2e_other_context` shows the nearest existing test."]
(OUT / "summary.md").write_text("\n".join(lines) + "\n")
print(json.dumps({"scenarios": tot, "pairwise": N_PAIRWISE, "baseline": N_BASELINE, "triples": N_TRIPLE, "subjects": len(SUBJ), "domains": len(DOMAINS),
                  "pairs": PAIR_TOTAL, "coverage": dict(cov), "driver": dict(drv), "unlock_order": order[:10],
                  "full_set": {k: v for k, v in FULL.items() if k != "by_domain"}}, indent=1))
