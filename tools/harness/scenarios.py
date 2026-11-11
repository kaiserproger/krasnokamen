#!/usr/bin/env python3
"""Discoverable catalog of *existing* executable scenarios.

The catalog is data, not a second runner: every entry names an existing harness
profile plus the exact selector (or the profile's own scenario id), so
``explain`` can answer "what would run and why" without side effects and
``run scenario <id>`` dispatches through the one shared run path in
``__main__.cmd_run`` (same artifact directory, bounded scope, receipt writer).

Assertions stay in the executable tests. An entry records only where to find
them, what they prove, and what they do not prove.

Rules for entries:

* no ``eval``, shell fragments, arbitrary plugin commands or a private DSL;
* no success condition duplicated from the test into this file;
* a native entry lists the exact test paths it requires, so a selector that
  silently matches nothing (or fewer tests than expected) fails closed.
"""

from __future__ import annotations

import os
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path

from .runtime import REPO_ROOT

# Evidence kinds. A graphical or client entry never falls back to TCP evidence.
EVIDENCE_KINDS = (
    "pure",
    "native",
    "persistence",
    "component",
    "tcp",
    "graphical",
    "performance",
)

# Prerequisite tokens resolved by `missing_prerequisites`.
PREREQUISITES = (
    "sibling:solaris-default-plugins",
    "sibling:solaris-loader",
    "client-token",
    "xvfb",
)


@dataclass(frozen=True)
class Scenario:
    """One existing check, its exact selector, and the limits of its evidence."""

    id: str
    summary: str
    owner: str
    evidence: str
    profile: str
    selector: tuple[str, ...] = ()
    tests: tuple[str, ...] = ()
    requires: tuple[str, ...] = ()
    scenario_id: str | None = None
    manifest: str | None = None
    seed: str | None = None
    fixture: str | None = None
    sources: tuple[str, ...] = ()
    proves: str = ""
    limits: str = ""

    def __post_init__(self) -> None:
        if self.evidence not in EVIDENCE_KINDS:
            raise ValueError(f"{self.id}: unknown evidence kind {self.evidence!r}")
        for token in self.requires:
            if token not in PREREQUISITES:
                raise ValueError(f"{self.id}: unknown prerequisite {token!r}")
        if self.evidence == "native" and self.tests and not self.selector:
            raise ValueError(f"{self.id}: a native entry needs an exact selector")
        if self.manifest is not None and self.scenario_id is None:
            raise ValueError(f"{self.id}: a manifest needs the scenario_id it belongs to")


def _entry(**kwargs: object) -> Scenario:
    return Scenario(**kwargs)  # type: ignore[arg-type]


SCENARIOS: dict[str, Scenario] = {
    "native-v2-ledger-painting": _entry(
        id="native-v2-ledger-painting", summary="Place a painting and remove its support",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-ledger-painting",
        requires=("xvfb",), sources=("tools/harness/backends/native_v2.py",),
        limits="One painting on a fixture wall; privileged support removal.",
    ),

    "native-v2-ledger-axe": _entry(
        id="native-v2-ledger-axe", summary="Use an axe to strip, scrape and unwax blocks",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-ledger-axe",
        requires=("xvfb",), sources=("tools/harness/backends/native_v2.py",),
        limits="One log orientation and two copper states; fixture-supplied axe.",
    ),

    "native-v2-ledger-sponge": _entry(
        id="native-v2-ledger-sponge", summary="Place a sponge beside a water source and observe absorption",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-ledger-sponge",
        requires=("xvfb",), sources=("tools/harness/backends/native_v2.py",),
        limits="One adjacent source; not the full absorption radius.",
    ),

    "native-v2-ledger-setblock-snbt": _entry(
        id="native-v2-ledger-setblock-snbt", summary="Open a chest populated by setblock SNBT and withdraw its contents",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-ledger-setblock-snbt",
        requires=("xvfb",), sources=("tools/harness/backends/native_v2.py",),
        limits="One chest with seven diamonds in slot zero; privileged setup.",
    ),
    "native-v2-ledger-crafting-pickup": _entry(
        id="native-v2-ledger-crafting-pickup", summary="Crafting table remains open on the first inventory pickup",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-ledger-crafting-pickup",
        requires=("xvfb",), sources=("tools/harness/backends/native_v2.py",),
        limits="One first pickup from 16 fixture-supplied planks; not the disconnect path.",
    ),

    "native-world-first-start-interrupted": _entry(
        id="native-world-first-start-interrupted",
        summary="Recover a singleplayer world interrupted during its first start",
        owner="core:harness", evidence="graphical", profile="singleplayer-regressions",
        scenario_id="native-world-first-start-interrupted", requires=("xvfb",),
        sources=("tools/harness/backends/singleplayer_regressions.py",),
        proves="A fresh world enters Play after its server is killed with its first-start marker and database present.",
        limits="One Linux native singleplayer first-start interruption.",
    ),
    "native-sp-relaunch-lease-race": _entry(
        id="native-sp-relaunch-lease-race",
        summary="Relaunch singleplayer while the prior server holds the world lease",
        owner="core:harness", evidence="graphical", profile="singleplayer-regressions",
        scenario_id="native-sp-relaunch-lease-race", requires=("xvfb",),
        sources=("tools/harness/backends/singleplayer_regressions.py",),
        proves="A relaunched client enters after a logged lease wait and the previous holder exits.",
        limits="One Linux native singleplayer held-lease release; not the timeout branch.",
    ),
    "native-sp-slow-machine": _entry(
        id="native-sp-slow-machine",
        summary="Start a fresh singleplayer world with a CPU-limited server and software renderer",
        owner="core:harness", evidence="graphical", profile="singleplayer-regressions",
        scenario_id="native-sp-slow-machine", requires=("xvfb",),
        sources=("tools/harness/backends/singleplayer_regressions.py",
                 "tools/harness/backends/singleplayer_slow.py"),
        proves="A software-rendered native client enters a fresh world after its release server takes more than 180 seconds at 1% CPU, clears loading, and accepts ordinary look input.",
        limits="Linux lavapipe; artificial server throttling is a startup regression, not the four-CPU laptop benchmark.",
    ),
    "native-sp-weak-hardware": _entry(
        id="native-sp-weak-hardware",
        summary="Fresh singleplayer Play-to-control under software Vulkan",
        owner="core:harness", evidence="graphical", profile="singleplayer-regressions",
        scenario_id="native-sp-weak-hardware", requires=("xvfb",),
        sources=("tools/harness/backends/singleplayer_regressions.py",
                 "tools/harness/backends/singleplayer_slow.py"),
        proves="A software-rendered native client enters a fresh world, clears loading, and accepts ordinary look input; records Play-to-control and RSS.",
        limits="Run in the documented four-logical-CPU combined client/server memory envelope; not a physical weak-GPU measurement.",
    ),
    "native-weak-first-view": _entry(
        id="native-weak-first-view",
        summary="Fresh one-CPU, 2G dedicated server to an actual playable view",
        owner="core:harness", evidence="graphical", profile="weak-first-view",
        requires=("xvfb",),
        sources=("tools/harness/backends/weak_first_view.py",),
        proves="A fresh dedicated server limited to one logical CPU and 2G produces a GPU-confirmed native view with loading cleared.",
        limits="The observer uses the remaining affinity CPUs (shared on a one-CPU host); this measures startup, not sustained capacity.",
    ),
    "native-v2-map-frame-waypoint": _entry(
        id="native-v2-map-frame-waypoint", summary="Distant framed home waypoint remains on the traveler's held clone",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-map-frame-waypoint",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_maps.py", "tools/harness/backends/native_promotion_wire.py"),
        limits="Scale-zero map and fixture geometry; recorded visual review required.",
    ),
    "native-v2-map-banner-removal": _entry(
        id="native-v2-map-banner-removal", summary="Revisiting a destroyed banner removes its marker while terrain refreshes",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-map-banner-removal",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_maps.py", "tools/harness/backends/native_promotion_wire.py"),
        limits="One scale-zero banner waypoint; recorded visual review required.",
    ),
    "native-v2-map-locked-banners": _entry(
        id="native-v2-map-locked-banners", summary="Locked map toggles banner waypoints without changing its terrain pixels",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-map-locked-banners",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_maps.py", "tools/harness/backends/native_promotion_wire.py"),
        limits="Ordinary cartography lock and banner use; recorded visual review required.",
    ),
    "native-v2-map-pumpkin-hiding": _entry(
        id="native-v2-map-pumpkin-hiding", summary="Ordinary carved-pumpkin equipment hides the other clone carrier's map marker",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-map-pumpkin-hiding",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_maps.py", "tools/harness/backends/native_promotion_wire.py"),
        limits="Two actual native clients; recorded visual review required.",
    ),
    "native-v2-map-terrain-shading": _entry(
        id="native-v2-map-terrain-shading", summary="Received and drawn map pixels distinguish water depth and height contours",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-map-terrain-shading",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_maps.py", "tools/harness/backends/native_promotion_wire.py"),
        limits="Scale-zero fixture columns; recorded visual review required.",
    ),
    "native-v2-map-nether-terrain": _entry(
        id="native-v2-map-nether-terrain", summary="Nether map ceiling noise ignores a physical roof and its replacement",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-map-nether-terrain",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_maps.py", "tools/harness/backends/native_promotion_wire.py"),
        limits="One ceiling-dimension map; recorded visual review required.",
    ),
    "native-v2-map-inventory-player-markers": _entry(
        id="native-v2-map-inventory-player-markers", summary="A non-held inventory clone carrier remains visible on the observer's map",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-map-inventory-player-markers",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_maps.py", "tools/harness/backends/native_promotion_wire.py"),
        limits="Two native players and two distinct map ids of one item type; recorded visual review required.",
    ),
    "native-v2-banner-placement": _entry(
        id="native-v2-banner-placement", summary="Patterned and named banner components survive ordinary placement",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-banner-placement",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_items.py",
                 "tools/harness/backends/native_promotion_wire.py", "tools/harness/backends/client_probes.py"),
        fixture="patterned and plain named banners; ordinary use on the prepared floor",
        proves="each banner is debited and its exact patterns/name arrive in the matching placed block-entity packet",
        limits="banner screenshots require recorded review; head/profile and pot-decoration gaps have separate open ledger rows",
    ),
    "native-v2-snow-golem-shear": _entry(
        id="native-v2-snow-golem-shear", summary="Ordinary shears remove the snow-golem pumpkin and yield its pickup",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-snow-golem-shear",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_mobs.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private snow golem and survival shears",
        proves="same golem changes received pumpkin flag after ordinary shears use; player collects a carved pumpkin",
        limits="face/removed-pumpkin screenshots require recorded review; no persistence or global drop count claim",
    ),
    "native-v2-cursor-close": _entry(
        id="native-v2-cursor-close", summary="Three-apple cursor closes and reopens inventory and crafting without a phantom",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-cursor-close",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_items.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private pad, crafting table and three apples; ordinary pickup, close, reopen",
        proves="before any repair click, reopened inventory and crafting cursor is empty and exactly three apples remain",
        limits="original three-apple repro only; no full-inventory overflow or disconnect branch",
    ),
    "native-v2-happy-ghast-harness": _entry(
        id="native-v2-happy-ghast-harness", summary="Harness custody and four distinct native happy-ghast riders",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-happy-ghast-harness",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_happy_ghast.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private ghast, one survival harness, graphical rider and four independent headless peers",
        proves="bare-ghast refusal, harness debit, four distinct passenger ids, fifth refusal and independent dismount",
        limits="the checkpoint codec round trip is covered by the focused core regression, not a restart; screenshots require recorded review",
    ),
    "native-v2-mob-metadata-spot": _entry(
        id="native-v2-mob-metadata-spot", summary="Frog, strider, camel and sniffer entity data decode in one native client",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-mob-metadata-spot",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_mob_metadata.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private obsidian pad, one dirt cell, one each of frog, strider, camel, sniffer and one small slime",
        proves="the client stays in play through the sniffer state, strider cold, frog tongue target and the unchanged camel accessors, and its log has no decode failure",
        limits="the camel dash flag needs a mounted jump and is covered by the focused core test and wire test, not this spot-check; screenshots require recorded review",
    ),
    "native-v2-custody-mount-export": _entry(
        id="native-v2-custody-mount-export", summary="Imported chested donkey withdrawal and deposit survive native export and reimport",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-custody-mount-export",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_custody.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private tamed donkey; ordinary chest attachment and seven-diamond cargo; shipping CLI export/import before and after real menu custody edits",
        proves="imported UUID receives seven diamonds, player withdraws all and deposits five emeralds, fresh reimport has exact new cargo with no resurrected diamonds",
        limits="donkey representative only, no mule/llama species coverage or vanilla Java save",
    ),
    "native-v2-p2-optional-pack": _entry(
        id="native-v2-p2-optional-pack", summary="Optional resource pack is declined by UUID and native graphical join completes",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-p2-optional-pack",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_custody.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private offline server with optional loopback resource-pack URL; unmodified wire observer",
        proves="actual Configuration offer, matching serverbound Declined, subsequent Play login, ordered chat and graphical frame",
        limits="no installation, required-pack acceptance or Play-phase pack-offer claim",
    ),
    "native-v2-console-shutdown": _entry(
        id="native-v2-console-shutdown", summary="Console stop logs orderly shutdown and preserves a graphical session across restart",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-console-shutdown",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_world.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private native server with console stdin and a graphical native player",
        proves="console stop exits cleanly, commits a checkpoint without ERROR, and the same player reconnects",
        limits="no injected storage failure; separate unit regression covers journal failure",
    ),
    "native-v2-lease-navigation": _entry(
        id="native-v2-lease-navigation",
        summary="Native writer lifetime, cancellation handoff, edge and open-door navigation",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-lease-navigation",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_lease_navigation.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private operator platform, airborne stand, two MCP sessions, support edge and open-door corridor",
        proves="bounded 120-second action retains writer; cancelled old writer leaves replacement and its held input; grounded edge and open-door routes arrive",
        limits="single Linux native fixture; cancellation scheduling is nondeterministic",
    ),
    "native-v2-end-restart": _entry(
        id="native-v2-end-restart", summary="Live peaceful End dragon survives clean checkpoint and graphical reconnect",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-end-restart",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_world.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private peaceful End world, creative observer and landing pad, clean owned server restart",
        proves="same player returns to the same End world and receives the exact retained live dragon UUID",
        limits="live peaceful fight only, not dead fight or repair of old corrupt checkpoints",
    ),
    "native-v2-entity-gallery": _entry(
        id="native-v2-entity-gallery", summary="Mixed frozen entity/projectile gallery survives unfreeze and row compaction",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-entity-gallery",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_promotion_world.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private pad; cow, floor-impact snowball, flying arrow, final armor stand while frozen",
        proves="snowball removal followed by live ordered echo and exact survivor UUIDs without core panic",
        limits="minimal compaction trigger, not every entity type or model visual parity",
    ),
    "native-v2-destroyed-chest": _entry(
        id="native-v2-destroyed-chest", summary="Destroyed open chest closes before ordinary commit, with exact extraction/deposit custody",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-destroyed-chest",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_custody.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private world; declared chest/diamond and tick freeze, command destruction with the menu open",
        proves="authoritative menu closes while simulation remains frozen, cursor is empty and exactly one diamond remains in ground/inventory for both extraction and insertion branches",
        limits="stale click prevented by menu closure; no crash/reconnect or physical mining claim",
    ),
    "native-v2-destroyed-chest-race": _entry(
        id="native-v2-destroyed-chest-race", summary="Stale container click sent right behind the destroying command: no duplicated or discarded item",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-destroyed-chest-race",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_custody.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private world; a second headless client populates the Nether (shared command execution); declared chest/diamond and tick freeze; six rounds per branch issue the extraction or deposit click before the client has seen the menu close",
        proves="in every round the one diamond remains exactly once in inventory or on the ground (extraction never duplicates it, deposit never discards it) and the menu is closed; at least one round per branch really sent the click",
        limits="which of command and click the server handles first depends on the tick boundary, so rounds repeat; an unfixed server loses custody only when the click lands after the command",
    ),
    "native-v2-duplicate-login": _entry(
        id="native-v2-duplicate-login", summary="A second native client of the same account replaces the first session, both ways, without duplicating the inventory",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-duplicate-login",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_duplicate_login.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private world; one declared diamond; second headless native client with the same username",
        proves="the second login enters Play with the same player and exactly one diamond, the first session leaves Play, and the first client's re-login replaces the second in turn",
        limits="offline-mode identity (same name, same UUID); no online-auth or plugin-login claimant",
    ),
    "native-v2-auth-budget": _entry(
        id="native-v2-auth-budget", summary="solaris-auth /register and /login complete on a native client behind the native login gate",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-auth-budget",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_auth_budget.py", "tools/harness/backends/client_probes.py"),
        fixture="private world with the sibling checkout's shipping solaris-auth package and [auth] require_plugin_login = true",
        proves="the locked session accepts ordinary chat only after /register, and again after a reconnect and /login, i.e. the plugin survives its Argon2 host-import callbacks",
        limits="depends on host hashing time exceeding the retired 100 ms allowance; the negative control is a server whose epoch allowance is the legacy four ticks",
    ),
    "native-v2-corrupt-block-entities": _entry(
        id="native-v2-corrupt-block-entities",
        summary="Imported world with undecodable jukebox and shrieker records: those chunks are refused, the server keeps ticking and checkpointing; an odd beacon record keeps running",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-corrupt-block-entities",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_corrupt_block_entities.py",
                 "tools/harness/backends/anvil_corrupt.py", "tools/harness/backends/native_promotion_custody.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private world; jukebox, sculk shrieker and beacon, each alone in its own chunk two chunks from the player, saved, exported to Anvil, one field of each record corrupted, imported again; the same player rejoins beside them",
        proves="the server refuses exactly the two undecodable chunks (one error each naming the record), never panics, keeps advancing game time past the beacon's 80-tick pass, serves ordered chat and keeps the player's own chunk; the checkpoint after the refusal and the shutdown checkpoint both commit, one error lists every refused chunk with its counts, and a restarted server refuses the same chunks again; the decodable beacon record is not refused",
        limits="a beacon record is decodable by construction, so its fixture is the runtime-path control, not a refusal; crafter, shelf, shulker box and conduit records are validated by the importer through their block and cannot reach the server (mc-world test); byte identity of a refused record is a Cargo check; the native client keeps its loading screen until the chunks within about 12 blocks of the camera are known, so fixtures keep two chunks away (see native-v2-corrupt-chunk-near-camera)",
    ),
    "native-v2-corrupt-chunk-near-camera": _entry(
        id="native-v2-corrupt-chunk-near-camera",
        summary="Known-red client repro: a refused chunk under the spawn keeps the native client on its loading screen",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-corrupt-chunk-near-camera",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_corrupt_block_entities.py",
                 "tools/harness/backends/anvil_corrupt.py", "tools/harness/backends/native_promotion_custody.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private world; one jukebox in the player's own chunk, six blocks from the player, record corrupted through an Anvil export and imported again; the same player rejoins",
        proves="when green: the client reaches in_play with a refused chunk under or within about 12 blocks of the camera, as a vanilla client does with a hole in the world",
        limits="red today (solaris-client play_world_presentation_ready needs the biome-neighbour chunks); a repro, not a regression gate",
    ),
    "native-command-feedback": _entry(
        id="native-command-feedback",
        summary="Player command feedback obeys the gamerule",
        owner="core:harness", evidence="graphical", profile="scene-shot",
        scenario_id="native-command-feedback", requires=("xvfb",),
        sources=("tools/harness/backends/scene_shot.py", "tools/harness/command_feedback_scene.json"),
        fixture="private native client, operator commands and ordered chat fences",
        proves="disabled success messages are absent, failure and chat remain visible, enabled success is visible",
        limits="native player command path; console behavior is covered separately by a Rust regression",
    ),
    "native-frozen-game-age": _entry(
        id="native-frozen-game-age",
        summary="Frozen simulated age stays fixed across teleport and local input",
        owner="core:harness", evidence="graphical", profile="scene-shot",
        scenario_id="native-frozen-game-age", requires=("xvfb",),
        sources=("tools/harness/backends/scene_shot.py", "tools/harness/frozen_game_age_scene.json"),
        fixture="private native client, operator freeze and teleport, local input",
        proves="received game age stays constant across held frames and teleport",
        limits="one native client; does not cover restart",
    ),
    "native-low-roof-house": _entry(
        id="native-low-roof-house",
        summary="Certify a house with an ordinary bed under a low roof",
        owner="core:harness", evidence="graphical", profile="scene-shot",
        scenario_id="native-low-roof-house", requires=("xvfb",),
        sources=("tools/harness/backends/scene_shot.py", "tools/harness/low_roof_house_scene.json"),
        fixture="private world with a two-block-high enclosed house and paired bed",
        proves="the native player receives house certification despite absent breeding headroom",
        limits="certification only; villager sleep and breeding are not checked",
    ),
    "native-falling-block-prediction": _entry(
        id="native-falling-block-prediction",
        summary="A received falling sand block renders and moves between server updates",
        owner="core:harness", evidence="graphical", profile="scene-shot",
        scenario_id="native-falling-block-prediction", requires=("xvfb",),
        sources=("tools/harness/backends/scene_shot.py", "tools/harness/falling_block_scene.json"),
        fixture="private windowed client; sand dropped in front of a dark backdrop",
        proves="the same rendered falling block moves downward between received movement updates",
        limits="one sand block in air over solid terrain; no landing, restart or TNT coverage",
    ),
    "native-v2-chest-drag-disconnect": _entry(
        id="native-v2-chest-drag-disconnect",
        summary="Physical chest drag then disconnect with an item on the cursor",
        owner="core:harness", evidence="graphical", profile="entities",
        scenario_id="native-v2-chest-drag-disconnect", requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py",
                 "tools/harness/backends/native_chest_drag_disconnect.py",
                 "tools/harness/backends/client_probes.py",
                 "tools/harness/backends/menu_observer.py"),
        fixture="private world, empty chest and one operator-granted oak log",
        proves="server cursor survives drag and disconnect returns exactly one log on rejoin",
        limits="single log, one chest and native client; no full-inventory overflow branch",
    ),
    "native-v2-input-pick": _entry(
        id="native-v2-input-pick", summary="Physical middle-click block/entity and gameplay F custody",
        owner="core:harness", evidence="graphical", profile="entities",
        scenario_id="native-v2-input-pick", requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_player_input.py"),
        fixture="diamond block, no-AI cow and declared inventory in survival and creative",
        proves="physical pick selects a matching stack, creative creates one, survival creates none, F swaps hands",
        limits="ordinary block and cow spawn egg; no control-pick block entity data",
    ),
    "native-v2-input-sand": _entry(
        id="native-v2-input-sand", summary="Setblock sand becomes a descending rendered entity and lands",
        owner="core:harness", evidence="graphical", profile="entities",
        scenario_id="native-v2-input-sand", requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_player_input.py"),
        fixture="sand 30 blocks above a private obsidian pad; stepped at one tick per second, then running world",
        proves="frozen server holds the falling block between steps; falling entity predicts vanilla gravity between position updates, source clears, sand lands; retained airborne/descent/landing frames",
        limits="frames require visual review; not Windows/VNC transport",
    ),
    "native-v2-input-menus": _entry(
        id="native-v2-input-menus", summary="Physical number/offhand/drop keys and short clicks in ordinary and creative inventory",
        owner="core:harness", evidence="graphical", profile="entities",
        scenario_id="native-v2-input-menus", requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_player_input.py"),
        fixture="declared diamond/emerald stacks in survival and creative menus on a private pad",
        proves="physical 1-9, F, Q, Ctrl+Q, short press/release, cursor pixels and authoritative move after reopen",
        limits="native X11; not Windows/VNC transport",
    ),
    "native-v2-input-camera": _entry(
        id="native-v2-input-camera", summary="Frozen teleport camera, live client waits and F1 HUD pixels",
        owner="core:harness", evidence="graphical", profile="entities",
        scenario_id="native-v2-input-camera", requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_player_input.py"),
        fixture="private pad, frozen server and local teleport",
        proves="camera follows teleport while frozen, client tick wait completes, F1 hides and restores hotbar pixels",
        limits="native X11; not server tick-control parity",
    ),
    "native-v2-shift-double-click": _entry(
        id="native-v2-shift-double-click",
        summary="Physical Shift-double-click player and chest inventory matrix",
        owner="core:harness", evidence="graphical", profile="entities",
        scenario_id="native-v2-shift-double-click", requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py",
                 "tools/harness/backends/native_shift_double_click.py",
                 "tools/harness/backends/shift_matrix.py",
                 "tools/harness/backends/shift_correction_wire.py",
                 "tools/harness/backends/client_probes.py",
                 "tools/harness/backends/menu_observer.py"),
        fixture="private world with operator-granted stone stacks and a chest",
        proves="physical shift clicks with component, timing, empty source, chest, correction and close checks against ordered server snapshots",
        limits="bounded inventory cases; frames require visual review; Java runtime is separate",
    ),
    "native-v2-shift-components": _entry(
        id="native-v2-shift-components",
        summary="Physical Shift-double-click with nonmatching item components",
        owner="core:harness", evidence="graphical", profile="entities",
        scenario_id="native-v2-shift-components", requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/shift_matrix.py",
                 "tools/harness/backends/menu_observer.py"),
        fixture="private world with named and unnamed stone stacks",
        proves="matching stacks move and differently named stacks remain; ordered server conservation",
        limits="one native GUI case; frames require visual review",
    ),
    "native-v2-shift-chest-correction": _entry(
        id="native-v2-shift-chest-correction",
        summary="Physical chest Shift-double-click with a received server correction",
        owner="core:harness", evidence="graphical", profile="entities",
        scenario_id="native-v2-shift-chest-correction", requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/shift_matrix.py",
                 "tools/harness/backends/shift_correction_wire.py",
                 "tools/harness/backends/menu_observer.py"),
        fixture="private chest and operator-granted stone stacks",
        proves="server slot update is received between physical clicks; matching chest stacks move and stone is conserved",
        limits="one native chest case; fixture removes a declared amount; frames require visual review",
    ),
    "native-v2-craft-drag": _entry(
        id="native-v2-craft-drag", summary="Native graphical inventory press-drag-release and rejected output placement",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-craft-drag",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_craft_drag.py",
                 "tools/harness/backends/client_probes.py"),
        fixture="private fresh world, declared operator clear/fill and one oak log plus one diamond",
        proves="initial drag keeps the cursor; a separate click places the grid item and yields recipe output; rejected output preserves custody",
        limits="requires pre-fix negative control and independent screenshot review; no generic in-flight reconciliation or vanilla parity claim",
    ),
    "native-v2-craft-drag-paced": _entry(
        id="native-v2-craft-drag-paced", summary="Frame-paced native player-inventory craft drag",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-craft-drag-paced",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_craft_drag_paced.py",
                 "tools/harness/backends/native_craft_drag.py", "tools/harness/backends/client_probes.py",
                 "tools/harness/backends/menu_observer.py", "tools/harness/backends/drag_variants.py"),
        fixture="operator-granted oak logs in the private player inventory",
        proves="ordered authoritative snapshots conserve custody during left, right and outside-release drags",
        limits="frame captures require visual review; vanilla comparison is separate",
    ),
    "native-v2-settlement-storage": _entry(
        id="native-v2-settlement-storage", summary="Settlement storage, Assign crafts and building icon menus driven by physical input",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-settlement-storage",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_settlement_storage.py",
                 "tools/harness/backends/native_craft_drag.py", "tools/harness/backends/client_probes.py"),
        fixture="private fresh world, declared operator pad/settlement/chest/composter/give/summon/bind/tp/time fixtures; second composter observed before refresh; non-operator windowed StorageGuest holds X focus only for its own legs",
        proves="owner XTest Shift/left/right/grid/Escape storage moves with 70+5 conservation against physical chests, full-inventory and view-only refusals, menu assignment, refused and accepted upgrade, hire into the new place, vanilla job acquisition at day 2000",
        limits="guest clicks use the MCP executor; map-cell click runs only with VillageMap dialog link nodes, else listed unsupported; screenshots need independent review; no vanilla Java parity claim",
    ),
    "loader-v2-settlements-smoke": _entry(
        id="loader-v2-settlements-smoke", summary="Frozen v2 server C-key settlement overview through Fabric Loader",
        owner="core:harness", evidence="graphical", profile="loader-live", scenario_id="loader-v2-settlements-smoke",
        requires=("sibling:solaris-loader", "client-token", "xvfb"),
        sources=("tools/harness/loader.py", "tools/harness/loader_settlements.py"),
        fixture="explicit --server-bin and --plugins-dir; isolated Loader permission/cache; no settlements package deployed",
        proves="Loader connection, core-owned C binding opens the native settlement dialog, and native settlement create/list/info",
        limits="minimal native settlement smoke, not plugin production/war coverage; visual review required",
    ),
    "loader-fabric-screen-admission": _entry(
        id="loader-fabric-screen-admission",
        summary="Release server starts the solaris:screen service for a real Fabric Loader session",
        owner="core:harness", evidence="graphical", profile="loader-live",
        scenario_id="loader-fabric-screen-admission",
        requires=("sibling:solaris-loader", "client-token", "xvfb"),
        sources=("tools/harness/loader.py", "tools/harness/loader_screen_admission.py"),
        fixture="explicit --server-bin and --plugins-dir; real Solaris Loader Fabric client joins; assertion reads the server log",
        proves="the server log records 'screen service started for admitted loader session' with platform=Fabric for the joined session",
        limits="admission of the screen service only; no H264 frame flow or viewer-side rendering is asserted",
    ),
    "native-composter-chance": _entry(
        id="native-composter-chance",
        summary="Graphical hopper-fed wheat seed compost probability",
        owner="core:harness", evidence="graphical", profile="composter-chance",
        requires=("xvfb",),
        sources=("tools/harness/backends/composter_chance.py",),
        fixture="seed 81 isolated world; operator builds locked hopper and level-1 composters, grants one seed per sample",
        proves="16 player menu deposits, independent hopper debits and client-observed resulting levels; rejects sixteen successes",
        limits="statistical Java 26.3 data and bytecode oracle, 0.3**16 false-rejection probability; screenshot review required",
    ),
    "dh-baseline": _entry(
        id="dh-baseline", summary="Real Java DH baseline: MC 26.1.2 + DH 3.3.3 protocol 16",
        owner="core:harness", evidence="graphical", profile="dh-gate",
        requires=("sibling:solaris-loader", "xvfb"),
        sources=("tools/harness/backends/dh_gate.py",),
        fixture="MC 26.1.2 + DH 3.3.3 on --platform fabric|neoforge; isolated world and independent control/enabled caches",
        proves="actual Java DH handshake, never-visited distant data versus disabled control, edit/reconnect/dimension/reset isolation and vanilla packet bounds",
        limits="26.1.2 baseline only; 26.2/26.3 await profile integration; screenshots require visual review; no performance claim",
    ),
    "native-v2-p5-humanoid-armor": _entry(
        id="native-v2-p5-humanoid-armor", summary="One native graphical observer views a headless actor's worn armor and unchanged stand",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-p5-humanoid-armor",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p5.py"),
        fixture="one graphical native observer and one headless actor; given magenta and crafted cyan leather; summoned mob equipment",
        proves="same-player iron/diamond/dyed armor; dyed player and stand custody; iron zombie and diamond skeleton captures",
        limits="visual review required; requires dyed_color, dye crafting and summon equipment support; no plain-leather fallback",
    ),
    "native-v2-light-seam": _entry(
        id="native-v2-light-seam", summary="Native windowed chunk-admission lighting: shadowed roof/tunnel seam whose second chunk is admitted after the first, in both orders",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-light-seam",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_light.py",
                 "tools/harness/backends/light_seam_model.py", "tools/harness/backends/light_wire.py"),
        fixture="floating obsidian platform with stone roof, tunnel and two torches across a chunk border plus far standing pads; creative operator; two save-all + same-world server restarts",
        proves="after a settled first-chunk admission (server residency = simulation_distance square, second chunk outside it) a one-chunk teleport admits the neighbour; geometry_ready with zero pending sections; received client light equals the server's wire light and an analytic oracle at every open cell of a 24x7x16 border region (A then B, B then A, plus a near close-up); no fully dark admitted chunk; retained screenshots",
        limits="server light is its own chunk/light packet stream, not a live light query; separate admission is proved by residency construction plus wire order, not a server admission log; operator-built terrain, not earned; visual review of screenshots still required",
    ),
    "native-v2-radius-full-view": _entry(
        id="native-v2-radius-full-view", summary="G5 radius split: stationary native viewer at view 8 / sim 4 holds the full 17x17 square",
        owner="core:edge", evidence="graphical", profile="entities", scenario_id="native-v2-radius-full-view",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_radius.py",
                 "tools/harness/backends/light_wire.py", "tools/harness/backends/client_probes.py"),
        fixture="server view_distance=8 simulation_distance=4 (LoginPlay 8/4); live client render_distance=8 required; shared _prepare stationary pad, no broad fills; per-chunk client cell at the pad level",
        proves="live minecraft_observe.client_settings.render_distance is 8; all 289 view-square chunks client-readable via minecraft_read_block at unique per-chunk cells (unheld chunks raise, fail-closed); no ForgetLevelChunk for a square member on the wire",
        limits="stationary readability only, not gameplay activity at the square edge or visual fidelity of every distant chunk mesh",
    ),
    "java-v2-p5-parity": _entry(
        id="java-v2-p5-parity", summary="Vanilla Java P4 surfaces, persistence and post-teleport walking on v2",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="java-v2-p5-parity",
        requires=("sibling:solaris-loader", "xvfb"),
        fixture="one real Java client; declared operator fixtures and clean same-world server restarts",
        proves="ordinary mining/crafting, vanilla screens, stand nameplate, damage, Q hopper custody and teleport movement",
        limits="visual screenshot review required; no native replacement or load coverage",
    ),
    "java-v2-p5-towns": _entry(
        id="java-v2-p5-towns", summary="Sequential non-op Java town leader and guest, protected builds and restart",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="java-v2-p5-towns",
        requires=("sibling:solaris-loader", "xvfb"),
        sources=("tools/harness/backends/java_p5.py", "tools/harness/backends/client_probes.py"),
        fixture="server plugin deployment; spawn protection disabled; temporary fixture op grants revoked before all tested player actions",
        proves="non-op leader creates/claims/builds and may open the claim chest, light fire and hurt a cow; a non-member guest's builds are corrected without item loss and cannot open the chest, light fire or hurt the cow; after the leader trusts the guest by UUID the guest can do all of it; town/claim survive restart",
        limits="one graphical process at a time; build, container, ignition and animal-damage protection, not PvP, explosions or every plugin command (unit-tested natively)",
    ),
    "java-v2-p5-cursor": _entry(
        id="java-v2-p5-cursor", summary="Bounded vanilla Java cursor split, swap, merge and carried-close custody",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="java-v2-p5-cursor",
        requires=("sibling:solaris-loader", "xvfb"),
        sources=("tools/harness/backends/java_p5.py", "tools/harness/backends/client_probes.py"),
        fixture="one real Java client with eight apples and three diamonds in an ordinary crafting menu",
        proves="primary pickup, secondary placement, different-item swap, merge, quick move and carried-close conservation",
        limits="bounded cursor/inventory click coverage; screenshots require independent visual review",
    ),
    "java-v2-p5-shift-double-click": _entry(
        id="java-v2-p5-shift-double-click",
        summary="Vanilla Java physical Shift-double-click matrix",
        owner="core:harness", evidence="graphical", profile="entities",
        scenario_id="java-v2-p5-shift-double-click",
        requires=("sibling:solaris-loader", "xvfb"),
        sources=("tools/harness/backends/shift_matrix.py",
                 "tools/harness/backends/native_shift_double_click.py",
                 "tools/harness/backends/shift_correction_wire.py",
                 "tools/harness/backends/java_p5.py",
                 "tools/harness/backends/menu_observer.py"),
        fixture="private world with operator-granted stone stacks and chest",
        proves="physical clicks and ordered server inventory conservation",
        limits="bounded GUI-scale-2 scenarios; captures require independent visual review",
    ),
    "java-v2-p5-craft-drag": _entry(
        id="java-v2-p5-craft-drag", summary="Vanilla Java physical player-inventory craft drag",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="java-v2-p5-craft-drag",
        requires=("sibling:solaris-loader", "xvfb"),
        sources=("tools/harness/backends/java_craft_drag.py", "tools/harness/backends/java_p5.py",
                 "tools/harness/backends/client_probes.py", "tools/harness/backends/native_craft_drag.py",
                 "tools/harness/backends/menu_observer.py", "tools/harness/backends/drag_variants.py"),
        fixture="operator-granted oak logs in the private player inventory",
        proves="an unfenced Java drag retains the cursor; instrumented left, right and outside-release drags record server custody",
        limits="one GUI-scale-2 vanilla Java client; captures require visual review; no native-client coverage",
    ),
    "java-v2-p5-craft-drag-paced": _entry(
        id="java-v2-p5-craft-drag-paced", summary="Frame-paced vanilla Java player-inventory craft drag",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="java-v2-p5-craft-drag-paced",
        requires=("sibling:solaris-loader", "xvfb"),
        sources=("tools/harness/backends/java_craft_drag.py", "tools/harness/backends/java_p5.py",
                 "tools/harness/backends/client_probes.py", "tools/harness/backends/native_craft_drag.py",
                 "tools/harness/backends/menu_observer.py", "tools/harness/backends/drag_variants.py"),
        fixture="operator-granted oak logs in the private player inventory",
        proves="records Java cursor retention and authoritative custody for frame-paced left, right and outside-release drags",
        limits="one GUI-scale-2 Java client; visual frame review required; no native coverage",
    ),
    "inventory-client": _entry(
        id="inventory-client",
        summary="Inventory path in a real client: crafting, cursor round-trip and held-item selection",
        owner="core:harness",
        evidence="tcp",
        profile="inventory",
        scenario_id="m94-03a-inventory-oak-log-to-planks",
        fixture="fresh isolated offline operator fixture; item setup declared in receipt",
        sources=(
            "tools/harness/backends/client_probes.py",
            "tools/harness/compatibility.py",
            "tools/harness/backends/probe_steps.py",
        ),
        proves="ordinary slot clicks consume one oak log and create four planks, conserve two apples across cursor pickup/return, and select the crafted stack",
        limits="focused inventory probe only; native state/protocol evidence is not rendering/GUI preservation; bare Java inventory retains its broader compatibility gates",
    ),
    "java-protocol-pot-creative-echo": _entry(
        id="java-protocol-pot-creative-echo",
        summary="Older creative client returns a lossy decorated pot without replacing canonical components",
        owner="edge:protocol", evidence="graphical", profile="inventory",
        scenario_id="java-protocol-pot-creative-echo",
        requires=("sibling:solaris-loader", "xvfb"),
        sources=("tools/harness/backends/java_protocol_pot.py", "tools/harness/backends/client_probes.py"),
        fixture="creative 26.1.2 Java client; one named pot with three absent sides and a named three-brick template",
        proves="physical creative-screen pickup and return to another hotbar slot preserve exact server-authoritative components twice",
        limits="one lossy pot projection through the current chain; not arbitrary creative edits or component parity",
    ),
    "core-replay-seed-81": _entry(
        id="core-replay-seed-81",
        summary="Core replay seed-81 actions in a real client",
        owner="loader:real-client-regression",
        evidence="graphical",
        profile="replay",
        scenario_id="core-actions-seed-81",
        manifest="docs/real-client-regression/manifests/core-replay-seed-81.json",
        seed="81",
        requires=("sibling:solaris-loader", "client-token", "xvfb"),
        fixture="fresh per-run world; the manifest names the seed-81 action sequence",
        sources=("docs/real-client-regression/manifests/core-replay-seed-81.json",),
        proves="the recorded seed-81 action sequence still runs against the current server",
        limits="deferred_by_owner until CP-096",
    ),
    "native-v2-redstone": _entry(
        id="native-v2-redstone", summary="Native lever-wire-repeater-lamp on/off and reversible second cycle",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-redstone",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/client_probes.py"),
        fixture="native v2 only; explicit isolated operator pad and unpowered circuit",
        proves="ordinary lever use yields exact received wire power, repeater power and lamp lit states in both directions",
        limits="not piston/hopper/comparator coverage; graphical only with --windowed",
    ),
    "native-v2-stations": _entry(
        id="native-v2-stations", summary="Native station menus, netherite smithing and chest/copper-chest conservation",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-stations",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/client_probes.py"),
        fixture="native v2 only; explicit operator pad, stations and exactly one of each smithing input",
        proves="received station menu shapes; exact netherite result/debits; chest and copper-chest reopen/return without duplicate",
        limits="anvil/enchanting/crafter menu admission only; no operation parity or earned input claim",
    ),
    "native-v2-brewing": _entry(
        id="native-v2-brewing", summary="Native water/wart/fuel insertion produces one received awkward potion",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-brewing",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/client_probes.py"),
        fixture="native v2 only; explicit operator stand, water bottle, nether wart and blaze powder",
        proves="ordinary menu clicks transform received water component to awkward and conserve the returned bottle/input debits",
        limits="no progress/fuel HUD predicate; graphical only with --windowed",
    ),
    "native-v2-jukebox": _entry(
        id="native-v2-jukebox", summary="Native jukebox record insertion, inventory debit and ejection",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-jukebox",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/client_probes.py"),
        fixture="native v2 only; explicit operator empty jukebox and one music disc",
        proves="ordinary use changes received has_record false/true/false, debits one disc and ejects the exact UUID-fenced disc stack",
        limits="no audible playback claim",
    ),
    "native-v2-tnt-fire-light": _entry(
        id="native-v2-tnt-fire-light", summary="Native torch light, fire ignition and TNT priming/blast",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-tnt-fire-light",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/client_probes.py"),
        fixture="native v2 only; explicit obsidian escape lane, torch/flint supply and TNT/dirt fixtures",
        proves="ordinary torch placement increases received block light; ignition produces fire and UUID-fenced TNT removes dirt",
        limits="no fire-spread/extinguishing or sky-light parity; graphical only with --windowed",
    ),
    "native-v2-dimensions": _entry(
        id="native-v2-dimensions", summary="Native command travel to Nether/End and cross-dimension death/respawn",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-dimensions",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/client_probes.py"),
        fixture="native v2 only; explicit operator creative, dimension-qualified teleport and kill",
        proves="received dimension transitions, own chat in each world and UUID-preserving live overworld respawn",
        limits="command travel only; not portal entry, earned exploration, dragon combat or rendered dimension parity",
    ),
    "native-v2-ranged": _entry(
        id="native-v2-ranged", summary="Native bow/crossbow/trident shots debit ammunition and damage one fenced target",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-ranged",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/client_probes.py"),
        fixture="native v2 only; explicit isolated range, NoAI iron golem and one weapon/ammunition per shot",
        proves="ordinary use/release yields a new projectile, exact ammunition debit and received damage to the same UUID",
        limits="no arrow pickup, Loyalty, weapon enchantment or full flight presentation claim",
    ),
    "native-v2-automation-storage": _entry(
        id="native-v2-automation-storage", summary="Native crafter pulse, shelf swap and sculk vibration cycle",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-automation-storage",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/client_probes.py"),
        fixture="native v2 only; explicit crafter/log, shelf/one diamond/one emerald, lever/sensor fixtures",
        proves="one ordinary pulse yields four planks without held-power duplicate; shelf swap/withdraw conserves both items; sensor active then inactive follows ordinary use",
        limits="no powered shelf chains, disabled crafter slots, calibrated filtering, shrieker/warden or visual model parity",
    ),
    "native-v2-storage-adversarial": _entry(
        id="native-v2-storage-adversarial", summary="Native bounded chest cursor-close and occupied-slot swap adversarial pass",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-storage-adversarial",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/client_probes.py"),
        fixture="native v2 only; isolated chest and exactly one diamond/emerald",
        proves="close with carried item returns once, occupied-slot swap preserves both owners, reopen and withdrawal conserve both items",
        limits="no crash/reconnect or full-inventory overflow; graphical only with --windowed",
    ),
    "native-v2-p1-armor-stand": _entry(
        id="native-v2-p1-armor-stand", summary="Native armor stand placement, equip/take, name and two-punch drops",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p1-armor-stand",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2; isolated pad, one stand/chestplate/named tag",
        proves="ordinary placement, exact armor custody, name-tag debit, stand/equipment drops and pickup",
        limits="rendered equipment/name require retained windowed screenshot review",
    ),
    "native-v2-p1-signs": _entry(
        id="native-v2-p1-signs", summary="Native sign text/color/glow/wax, edit refusal and empty replacement editor",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p1-signs",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2; isolated sign and one dye/glow ink/honeycomb",
        proves="ordinary sign edit/decorations, wax refusal, break/pickup and empty editor at the same replacement position",
        limits="text/color/glow require retained windowed screenshot review",
    ),
    "native-v2-p1-piglin": _entry(
        id="native-v2-p1-piglin", summary="Native piglin gold barter and no unpaid second payout",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p1-piglin",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2; isolated overworld piglin and one gold ingot",
        proves="ordinary gold interaction/debit and newly received non-gold loot UUID",
        limits="admire pose requires retained windowed screenshot review; no gold-armor neutrality claim",
    ),
    "native-v2-p1-trader": _entry(
        id="native-v2-p1-trader", summary="Native wandering trader merchant admission and close/reopen",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p1-trader",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2; isolated NoAI wandering trader",
        proves="ordinary interaction receives merchant menu twice with no inventory credit",
        limits="nonempty offers require retained windowed screenshot review; no purchase/restock claim",
    ),
    "native-v2-p1-leaves": _entry(
        id="native-v2-p1-leaves", summary="Native log mining, leaf distance/decay/drops and persistent control",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p1-leaves",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2; natural canopy, two trunk logs, persistent leaf, randomTickSpeed 4096 after mining",
        proves="player log removal, received distance seven then air, leaf drops and surviving persistent control",
        limits="bounded fixture canopy; not terrain-generated tree/restart/chunk-seam coverage",
    ),
    "native-v2-p1-helmet-equip": _entry(
        id="native-v2-p1-helmet-equip", summary="Native air-use helmet equip, occupied-slot swap and exact custody",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p1-helmet-equip",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2; one gold and one iron helmet, empty head slot and air raycast",
        proves="ordinary right-click equips and swaps the head slot; empty repeat use creates no items",
        limits="head-slot swap only; no full inventory, other armor slots or enchants",
    ),
    "native-v2-p1-player-observer": _entry(
        id="native-v2-p1-player-observer", summary="Two windowed native clients receive and render a player holding a sword",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p1-player-observer",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2; one actor and one observer on the same server/Xvfb, ordinary held sword",
        proves="observer receives actor UUID as minecraft:player with held equipment; slot change clears held equipment",
        limits="one observer and one held item; no remote skins, armor layers, reconnect or movement claim",
    ),
    "native-v2-p4-nameplates": _entry(
        id="native-v2-p4-nameplates", summary="Native explicit/hidden armor-stand labels, targeted pig and server restart",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p4-nameplates",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p4.py"),
        fixture="native v2; explicit CustomNameVisible stand, default-hidden named stand and NoAI named pig",
        proves="actual crosshair targets and same stand UUIDs after owned server restart",
        limits="visible/absent label pixels require retained windowed screenshot review",
    ),
    "native-v2-p4-advancements": _entry(
        id="native-v2-p4-advancements", summary="Native story advancements, earned stone pickaxe and restart",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p4-advancements",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p4.py"),
        fixture="fresh native player; declared wood/table/cobble/stick supply, ordinary table crafting",
        proves="actual earned pickaxe, advancement screen and recipes before/after real server restart",
        limits="story labels/completion/chat and no toast flood require screenshot/transcript review",
    ),
    "native-v2-p4-damage": _entry(
        id="native-v2-p4-damage", summary="Native zombie hit and retained before/hit/after frames",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-p4-damage",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p4.py"),
        fixture="native v2; nighttime zombie beside healthy survival player",
        proves="received health loss and live client/input after the hit",
        limits="hurt tilt/direction requires hit-frame visual review; health loss alone is not presentation",
    ),
    "native-v2-p4-identities": _entry(
        id="native-v2-p4-identities", summary="Named damage identities and an angered warden sonic attack",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-p4-identities",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p4.py"),
        fixture="seven explicit damage types on a healthy survival player, then lever-provoked warden behind a wall",
        proves="received command damage identities and health loss; real warden sonic cause, death screen and respawn",
        limits="command cases prove identities, not freeze/cramming/wall collision mechanics; no complete damage or rendering parity",
    ),
    "native-v2-p4-death-protection": _entry(
        id="native-v2-p4-death-protection", summary="Custom held death protection consumes authored effects",
        owner="core:harness", evidence="graphical", profile="entities", scenario_id="native-v2-p4-death-protection",
        requires=("xvfb",),
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p4.py"),
        fixture="held stick with clear-all and speed actions, followed by lethal generic damage",
        proves="survival at one health, one item consumed, authored speed without default totem effects",
        limits="clear/apply actions only; remove, probability, sound and codec variants have separate Cargo regressions",
    ),
    "native-v2-p4-hopper": _entry(
        id="native-v2-p4-hopper", summary="Native Q drop enters nearby hopper, exact withdrawal and no duplicate",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p4-hopper",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p4.py"),
        fixture="native v2; adjacent hopper over solid pad and exactly one diamond",
        proves="ordinary Q debit, exact hopper menu receipt rather than player re-pickup, withdrawal/reopen custody",
        limits="single hopper/drop; no chains, powered state or unloaded boundary",
    ),
    "native-v2-p4-statistics": _entry(
        id="native-v2-p4-statistics", summary="Native earned statistics, tabs, restart and repeated authoritative requests",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p4-statistics",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p4.py"),
        fixture="native v2; ordinary walking/jump and stone/dirt mining/pickup, native Statistics request UI",
        proves="received plausible movement and exact mining/pickup counters survive restart and repeated requests",
        limits="one player and bounded actions; no synthetic request or inferred AwardStats receipt",
    ),
    "native-v2-p2-slime": _entry(
        id="native-v2-p2-slime", summary="Native medium slime kill, small children and no recursive split",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p2-slime",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2; one Size:1 NoAI slime and ordinary diamond sword",
        proves="UUID-fenced medium death yields 2-4 distinct small live children; a small child does not split",
        limits="one isolated split family; no magma cube, restart or natural spawning claim",
    ),
    "native-v2-p2-enderman": _entry(
        id="native-v2-p2-enderman", summary="Native arrow reverses and Enderman teleports without damage",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p2-enderman",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2 release; dry clear teleport pad, NoAI Enderman, bow and one arrow",
        proves="ordinary shot debits arrow; new arrow reverses, same Enderman UUID teleports and retains health",
        limits="no water/stare anger/block carrying or teleport persistence coverage",
    ),
    "native-v2-p2-fishing": _entry(
        id="native-v2-p2-fishing", summary="Native open-water cast, real bite, reel, caught inventory and early-reel refusal",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p2-fishing",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2 release; source-water pool and one ordinary rod; unmodified hook metadata trace",
        proves="real native cast/reel after hook biting yields received inventory item; early reel does not award another",
        limits="canonical native bite projection missing; forwarded wire metadata is supplemental wait evidence",
    ),
    "native-v2-p2-creaking": _entry(
        id="native-v2-p2-creaking", summary="Native Creaking freezes under gaze, walks away and refreezes",
        owner="core:harness", evidence="tcp", profile="entities", scenario_id="native-v2-p2-creaking",
        sources=("tools/harness/backends/native_v2.py", "tools/harness/backends/native_p1.py"),
        fixture="native v2; ordinary unbound Creaking on a clear pad, survival observer",
        proves="same UUID remains still during first gaze, moves on look-away and freezes on second gaze",
        limits="no heart linkage, pumpkin, occlusion or multiple observers coverage",
    ),
}


def verify_native_evidence(log_text: str, tests: tuple[str, ...]) -> list[str]:
    """Return the expected test paths that are not reported as passed.

    ``cargo test`` prints one ``test <path> ... ok`` line per executed test; a
    selector that matches nothing prints ``running 0 tests`` and no such line,
    so a zero-selected run can never be mistaken for a pass.
    """
    missing = [name for name in tests if f"test {name} ... ok" not in log_text]
    if "test result: FAILED" in log_text:
        missing.append("<a test in this selector failed>")
    return missing


_CARGO_SUMMARY = re.compile(
    r"test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored"
    r"(?:; (\d+) measured)?(?:; (\d+) filtered out)?"
)


def cargo_counts(log_text: str) -> dict[str, int] | None:
    """Summarize the ``test result:`` lines of one cargo run, or None when absent.

    ``selected`` is every test the selector matched (executed + ignored) and
    ``executed`` is passed + failed; ``filtered_out`` is what cargo itself
    reports as excluded. ``None`` means the engine reported no counts, which is
    not evidence of a pass either; the caller fails closed on it.
    """
    totals = {
        "selected": 0,
        "executed": 0,
        "passed": 0,
        "failed": 0,
        "ignored": 0,
        "filtered_out": 0,
    }
    seen = False
    for match in _CARGO_SUMMARY.finditer(log_text):
        seen = True
        passed, failed, ignored = (int(match.group(index)) for index in (2, 3, 4))
        totals["passed"] += passed
        totals["failed"] += failed
        totals["ignored"] += ignored
        totals["executed"] += passed + failed
        totals["selected"] += passed + failed + ignored
        totals["filtered_out"] += int(match.group(6) or 0)
    return totals if seen else None


def count_failures(counts: dict[str, int] | None) -> list[str]:
    """Reasons the engine counts alone forbid a pass."""
    if counts is None:
        return ["the engine reported no test counts"]
    reasons: list[str] = []
    if counts["failed"]:
        reasons.append(f"{counts['failed']} test(s) failed")
    if counts["executed"] == 0:
        reasons.append(
            f"no test was executed; {counts['ignored']} selected ignored"
            if counts["ignored"]
            else "no test was selected and executed"
        )
    return reasons


def log_text(ctx: object) -> str:
    """Concatenate every log the run already recorded."""
    parts: list[str] = []
    for relative in getattr(ctx, "logs", []):
        path = Path(relative)
        if not path.is_absolute():
            path = REPO_ROOT / path
        try:
            parts.append(path.read_text(errors="replace"))
        except OSError:
            continue
    return "\n".join(parts)


def repo_revisions() -> dict[str, str]:
    """Best-effort revisions of the repos a named run may involve.

    Never fatal: a missing checkout or git reports ``unavailable`` instead of
    turning a run into a failure.
    """
    repos = {
        "solaris": REPO_ROOT,
        "solaris-default-plugins": Path(
            os.environ.get("SOLARIS_DEFAULT_PLUGINS_ROOT", REPO_ROOT.parent / "solaris-default-plugins")
        ),
        "solaris-loader": Path(os.environ.get("SOLARIS_LOADER_ROOT", REPO_ROOT.parent / "solaris-loader")),
    }
    revisions: dict[str, str] = {}
    for name, path in repos.items():
        try:
            completed = subprocess.run(
                ["git", "-C", str(path), "rev-parse", "HEAD"],
                capture_output=True,
                text=True,
                timeout=5,
                check=False,
            )
            revision = completed.stdout.strip()
            revisions[name] = revision if completed.returncode == 0 and revision else "unavailable"
        except (OSError, subprocess.SubprocessError):
            revisions[name] = "unavailable"
    return revisions


def describe(scenario: Scenario) -> str:
    """Resolved, side-effect-free description for ``explain``."""
    lines = [
        f"scenario: {scenario.id}",
        f"summary: {scenario.summary}",
        f"owner: {scenario.owner}",
        f"evidence: {scenario.evidence}",
        f"profile: {scenario.profile}",
    ]
    if scenario.selector:
        lines.append("selector: cargo test " + " ".join(scenario.selector))
    if scenario.scenario_id:
        lines.append(f"scenario id: {scenario.scenario_id} (bound for this run)")
    if scenario.manifest:
        lines.append(f"manifest: {scenario.manifest} (bound for this run)")
    if scenario.seed:
        lines.append(f"seed: {scenario.seed}")
    if scenario.fixture:
        lines.append(f"fixture: {scenario.fixture}")
    if scenario.tests:
        lines.append("required tests:")
        lines.extend(f"  {name}" for name in scenario.tests)
    if scenario.requires:
        lines.append(
            "declared prerequisites (enforced by the owning profile/backend, not here): "
            + ", ".join(scenario.requires)
        )
    if scenario.sources:
        lines.append("sources:")
        lines.extend(f"  {path}" for path in scenario.sources)
    lines.append(f"proves: {scenario.proves}")
    lines.append(f"does not prove: {scenario.limits}")
    lines.append("receipt: .analysis/validation/<stamp>-<profile>-<id>/result.json (status from the profile)")
    return "\n".join(lines)
