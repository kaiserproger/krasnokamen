#!/usr/bin/env python3
"""``python3 -m tools.harness plan``: the validation a change requires.

Input is a changed-path list (default: ``git diff --name-only`` against ``--base``
plus untracked files, in the cwd tree). Output is the deduplicated list of gates
the repository's own rules ask for, each with the rule's doc source.

Every rule is one row of ``RULES`` below, with a ``source`` (doc path and section)
and a ``why``. Nothing here is policy of its own: change a row only when the
cited document changes. The mapping is path-based and minimal: a path that no
row names, and a path in two or more domains, is printed as ``manual classification
required``; a change that crosses a domain the paths do not show (a merge step
that releases container items, say) needs a manual classification too. The absence of a class is never proof that it does not apply.

The module is standard-library only so it can run against any checkout of the
tree: the profile and scenario catalogs are read from the *target* tree's
``profiles.py`` and ``scenarios.py`` with ``ast``, never imported.
"""

from __future__ import annotations

import argparse
import ast
import json
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

# ------------------------------------------------------------------ sources

AGENTS_VALIDATION = "AGENTS.md#build-and-validation"
AGENTS_RULE5 = "AGENTS.md#working-rules (rule 5)"
TOOLING_TEST = "docs/AGENT_TOOLING.md#test-gate-nextest"
PERSISTENCE_DOC = "docs/architecture/persistence.md#startup-and-recovery"
PROTOCOL_DOC = "docs/development/protocol-changes.md#implement-and-test"
GATES_DOC = "docs/development/real-client-gates.md"
LEDGER_DOC = "docs/bugs/README.md#the-fixed-rule-single-home"
QA_HYGIENE = "docs/QA_AGENT_PROTOCOL.md#gate-hygiene"
SETTLEMENT_GATES = "docs/settlements/acceptance-gates.md"
SCENARIO_CATALOG = "tools/harness/scenarios.py"
PROFILE_CATALOG = "tools/harness/profiles.py"

# -------------------------------------------------------------- path tables

RUST_GLOBS = (
    "crates/**/*.rs",
    "crates/**/Cargo.toml",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
)

# Serving state, WAL, checkpoints, recovery, capture, chunk loading
# (docs/architecture/persistence.md; module names under crates/mc-core/src).
PERSISTENCE_GLOBS = (
    "crates/mc-core/src/**/*journal*",
    "crates/mc-core/src/**/*checkpoint*",
    "crates/mc-core/src/**/*recovery*",
    "crates/mc-core/src/**/*persistence*",
    "crates/mc-core/src/**/*residency*",
    "crates/mc-core/src/**/*hydration*",
    "crates/mc-core/src/chunk_loader*",
    "crates/mc-core/src/chunk_tickets*",
    "crates/mc-core/src/hot_backup.rs",
    "crates/mc-core/src/core/capture.rs",
    "crates/mc-core/src/world/capture.rs",
    "crates/mc-core/src/core_stop*",
    "crates/mc-core/src/core_session*",
    "crates/mc-edge/src/backup*",
    "crates/mc-edge/src/server_lifecycle*",
    "crates/mc-world/src/native.rs",
    "crates/mc-world/src/native/**",
    "crates/mc-world/src/storage.rs",
    "crates/mc-world/src/storage/**",
    "crates/mc-world/src/atomic_file.rs",
    "crates/mc-world/src/anvil/**",
)

# Packets, codecs, translation and session handling
# (docs/development/protocol-changes.md: mc-protocol, and mc-edge translation).
PROTOCOL_GLOBS = (
    "crates/mc-protocol/**",
    "crates/mc-nbt/**",
    "crates/mc-world/src/wire*",
    "crates/mc-edge/src/decode*",
    "crates/mc-edge/src/connection*",
    "crates/mc-edge/src/handshake*",
    "crates/mc-edge/src/login*",
    "crates/mc-edge/src/configuration*",
    "crates/mc-edge/src/play*",
    "crates/mc-edge/src/translation/**",
    "crates/mc-edge/src/fanout*",
    "crates/mc-edge/src/plan.rs",
    "crates/mc-edge/src/plan/**",
    "crates/mc-edge/src/command_tree*",
    "crates/mc-edge/src/server_protocol*",
    "crates/mc-edge/src/*_wire_tests.rs",
)

# Item and container custody: inventories, containers, item entities, merges that
# release their stacks (tick/merge.rs removes container block entities and spawns
# their items). Minimal list: custody also lives outside these names.
CUSTODY_GLOBS = (
    "crates/mc-core/src/inventory.rs",
    "crates/mc-core/src/inventory/**",
    "crates/mc-core/src/chests*",
    "crates/mc-core/src/chiseled_shelf*",
    "crates/mc-core/src/ender_chest*",
    "crates/mc-core/src/shulker*",
    "crates/mc-core/src/lectern*",
    "crates/mc-core/src/crafter*",
    "crates/mc-core/src/item_*",
    "crates/mc-core/src/plugin_graves*",
    "crates/mc-core/src/command_item_replace.rs",
    "crates/mc-core/src/mob_lanes_trades*",
    "crates/mc-core/src/block_drops*",
    "crates/mc-core/src/tick/merge.rs",
    "crates/mc-core/src/tick/containers.rs",
    "crates/mc-core/src/tick/container_access.rs",
    "crates/mc-core/src/tick/stations.rs",
    "crates/mc-core/src/tick/merchant.rs",
    "crates/mc-core/src/tick/input/inventory.rs",
    "crates/mc-core/src/mounts/menus.rs",
    "crates/mc-core/src/**/*custody*",
    "crates/mc-edge/src/container*",
)

# Authoritative gameplay and client-visible presentation.
GAMEPLAY_GLOBS = (
    "crates/mc-core/src/**",
    "crates/mc-physics/src/**",
    "crates/mc-worldgen/src/**",
    "crates/mc-edge/src/screen*",
    "crates/mc-edge/src/resource_pack*",
    "crates/mc-edge/src/boss_bars*",
    "crates/mc-edge/src/light*",
    "crates/mc-edge/src/chunks*",
    "crates/mc-edge/src/anti_xray*",
    "crates/mc-edge/src/dialog*",
    "crates/mc-edge/src/settlement_labels*",
    "crates/mc-edge/src/voice*",
    "crates/mc-edge/src/lod*",
    "crates/mc-edge/src/dh.rs",
    "crates/mc-edge/src/dh/**",
    "crates/mc-edge/src/advancements*",
)
GAMEPLAY_EXCLUDE = (
    "crates/mc-core/src/bin/**",
    "crates/mc-core/src/config.rs",
    "**/*bench*",
)

GATE_GLOBS = (
    "tools/harness/backends/**",
    "tools/harness/scenarios.py",
    "tools/harness/profiles.py",
    "tools/harness/loader.py",
    "tools/harness/loader_settlements.py",
)
HARNESS_PY_GLOBS = ("tools/harness/**/*.py",)


@dataclass(frozen=True)
class Area:
    """A gameplay area and the real-client gates its docs and catalogs name."""

    id: str
    globs: tuple[str, ...]
    gates: tuple[str, ...]  # ``profile:<name>`` or ``scenario:<id>``
    source: str


AREAS = (
    Area("settlements",
         ("crates/mc-core/src/settlements/**", "crates/mc-core/src/settlements.rs",
          "crates/mc-core/src/villager/**", "crates/mc-core/src/villager.rs",
          "crates/mc-core/src/plugin_structures/settlement*",
          "crates/mc-core/src/blueprint*", "crates/mc-edge/src/settlement_labels*"),
         ("profile:village-autonomy-qa", "profile:civilization-seed-qa"),
         SETTLEMENT_GATES),
    Area("settlement-army", ("crates/mc-core/src/**/army*",), ("profile:army-qa",),
         "tools/harness/profiles.py (army-qa: barracks, orders, combat)"),
    Area("settlement-autocraft", ("crates/mc-core/src/**/autocraft*",), ("profile:autocraft-qa",),
         "tools/harness/profiles.py (autocraft-qa: dependency chains)"),
    Area("settlement-buildings",
         ("crates/mc-core/src/settlements/building*", "crates/mc-core/src/blueprint*",
          "crates/mc-core/src/settlements/blueprints*"),
         ("profile:buildings-qa",),
         "tools/harness/profiles.py (buildings-qa: every blueprint built by a real Builder)"),
    Area("settlement-ui",
         ("crates/mc-core/src/settlements/ui*", "crates/mc-edge/src/settlement_labels*"),
         ("scenario:native-v2-settlement-storage", "profile:settlement-map-qa"),
         "tools/harness/scenarios.py (native-v2-settlement-storage), profiles.py (settlement-map-qa)"),
    Area("commands",
         ("crates/mc-core/src/command*", "crates/mc-core/src/commands*",
          "crates/mc-edge/src/command_tree*"),
         ("profile:admin-commands",),
         "tools/harness/profiles.py (admin-commands: vanilla admin commands as an operator)"),
    Area("dialogs", ("crates/mc-core/src/dialog*", "crates/mc-edge/src/dialog*"), ("profile:dialogs",),
         "tools/harness/profiles.py (dialogs)"),
    Area("anti-xray", ("crates/mc-core/src/anti_xray*", "crates/mc-edge/src/anti_xray*"),
         ("profile:anti-xray",), "tools/harness/profiles.py (anti-xray)"),
    Area("block-log", ("crates/mc-core/src/block_log*",), ("profile:block-log",),
         "tools/harness/profiles.py (block-log)"),
    Area("composter", ("crates/mc-core/src/composter*",), ("profile:composter-chance",),
         "tools/harness/profiles.py (composter-chance)"),
    Area("chat-visibility", ("crates/mc-core/src/chat_visibility*", "crates/mc-core/src/dimensions_tick.rs"),
         ("profile:chat-visibility",), "tools/harness/profiles.py (chat-visibility)"),
    Area("datapacks", ("crates/mc-core/src/datapacks*", "crates/mc-core/src/datapack_*",
                       "crates/mc-edge/src/recipes*"), ("profile:datapack-reload",),
         "tools/harness/profiles.py (datapack-reload)"),
    Area("containers-custody", CUSTODY_GLOBS,
         ("profile:inventory", "scenario:native-v2-destroyed-chest", "scenario:native-v2-cursor-close"),
         "docs/bugs/README.md#native-promotion-probes"),
    Area("mounts", ("crates/mc-core/src/mounts/**", "crates/mc-core/src/mounts.rs"),
         ("scenario:native-v2-custody-mount-export",), "docs/bugs/README.md#native-promotion-probes"),
    Area("light", ("crates/mc-core/src/light*", "crates/mc-edge/src/light*", "crates/mc-world/src/light*"),
         ("scenario:native-v2-light-seam",), "tools/harness/scenarios.py (native-v2-light-seam)"),
)

# Profiles marked variable: timing-, seed- or concurrency-dependent behaviour
# needs a pass rate over at least three runs (AGENTS.md, QA protocol "Variance").
VARIABLE_PROFILES = {
    "civilization-seed-qa": f"{SETTLEMENT_GATES}#civilization-observation: generated seed, accelerated ticks",
    "village-autonomy-qa": f"{SETTLEMENT_GATES}#autonomous-village: generated village, real restart timing",
    "village-owner-order-qa": f"{SETTLEMENT_GATES}#owner-order: restart and funding timing",
    "village-qa": f"{SETTLEMENT_GATES}: simulated villagers over time",
    "claim-only-qa": f"{SETTLEMENT_GATES}: settled autonomous world and restart",
    "buildings-qa": f"{SETTLEMENT_GATES}: builders over simulated time",
    "army-qa": f"{SETTLEMENT_GATES}: combat and restart",
    "autocraft-qa": f"{SETTLEMENT_GATES}: restart mid-chain",
    "composter-chance": "docs/bugs/README.md#native-promotion-probes: individual random debits",
    "native-lifecycle": f"{GATES_DOC}: reconnect and 30-minute soak",
}

# Profiles whose real-client evidence is the native windowed client; the list
# must stay inside the native allow-list in tools/harness/__main__.py (tested).
NATIVE_WINDOWED = frozenset({
    "admin-commands", "anti-xray", "army-qa", "autocraft-qa", "block-log", "buildings-qa",
    "chisel", "civilization-seed-qa", "chat-visibility", "claim-only-qa", "composter-chance", "datapack-reload", "dialogs",
    "entities", "inventory", "native-lifecycle", "scene-shot", "settlement-map-qa",
    "singleplayer-regressions", "survival", "title-connect", "village-autonomy-qa",
    "village-owner-order-qa", "village-qa",
})

# Backend engine -> the profiles that run it (profiles.py ``run_*`` imports).
BACKEND_PROFILES: dict[str, tuple[str, ...]] = {
    "admin_commands": ("admin-commands",),
    "anti_xray": ("anti-xray",),
    "army_qa": ("army-qa",),
    "army_qa_watchdog": ("army-qa",),
    "autocraft_qa": ("autocraft-qa",),
    "block_log": ("block-log",),
    "buildings_qa": ("buildings-qa",),
    "chisel": ("chisel",),
    "civilization_seed_qa": ("civilization-seed-qa",),
    "claim_only_qa": ("claim-only-qa",),
    "client_probes": ("core-client", "inventory", "entities", "native-lifecycle"),
    "chat_visibility": ("chat-visibility",),
    "composter_chance": ("composter-chance",),
    "datapack_reload": ("datapack-reload",),
    "dh_gate": ("dh-gate",),
    "dialogs": ("dialogs",),
    "gameplay": ("survival",),
    "graves": ("graves",),
    "item_merge": ("item-merge",),
    "native_logical": ("logical-native",),
    "native_scale": ("native-scale",),
    "native_v2": ("entities",),
    "pause_empty": ("pause-empty",),
    "scene_shot": ("scene-shot",),
    "settlement_map_qa": ("settlement-map-qa",),
    "singleplayer_regressions": ("singleplayer-regressions",),
    "title_connect": ("title-connect",),
    "village_autonomy_qa": ("village-autonomy-qa", "civilization-seed-qa", "village-owner-order-qa"),
    "village_qa": ("village-qa",),
}

PERSISTENCE_FILTERS = ("restart", "recovery", "checkpoint", "journal", "reopen")
PROTOCOL_EDGE_FILTERS = ("wire", "decode", "translation")
CUSTODY_FILTERS = ("conservation", "custody")
SCENARIO_COLLAPSE = 3  # more matching scenarios than this: run their profile instead
HARNESS_SUITE = "python3 -m unittest discover -t . -s tools/harness -p 'test_*.py'"
SIBLING_NAMES = {"client": "solaris-client", "loader": "solaris-loader", "plugins": "solaris-default-plugins"}
SIBLING_ENV = {"client": "SOLARIS_CLIENT_ROOT", "loader": "SOLARIS_LOADER_ROOT",
               "plugins": "SOLARIS_DEFAULT_PLUGINS_ROOT"}
SIBLING_MARKERS = {
    "client": ("launcher", "SPECIFICATION.md"),
    "loader": ("loader-core", "settings.gradle.kts"),
    "plugins": ("standard-pack", "solaris-towns"),
}

REVIEWER_KINDS = {
    "persistence": "independent read-only reviewer before Main commits a persistence change",
    "protocol": "independent read-only reviewer before Main commits a protocol change",
    "custody": "independent read-only reviewer before Main commits a custody change",
    "ledger-batch": "independent read-only reviewer for a ledger promotion batch of 5 or more rows",
}
LEDGER_BATCH_MIN = 5
INNOVATION_DOC = "docs/development/adding-a-feature.md#new-dependencies-and-components"
DEP_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


@dataclass(frozen=True)
class Rule:
    """One validation rule: paths in, gates out, with the document that says so."""

    id: str
    repo: str  # core | client | loader | plugins
    source: str
    why: str
    globs: tuple[str, ...]
    gates: tuple[str, ...]  # literal command, or ``@expander``
    exclude: tuple[str, ...] = ()
    when: str = "edit"  # edit | slice | commit | integrate
    reviewer: str = ""
    cwd: str = "."
    tests_trigger: bool = True  # do test-only paths trigger the rule?


RULES: tuple[Rule, ...] = (
    Rule("rust-l0", "core", AGENTS_VALIDATION,
         "L0: tests for the affected crate scope while editing",
         RUST_GLOBS, ("@cargo-l0",)),
    Rule("rust-l1", "core", AGENTS_VALIDATION, "L1: finishing a slice",
         RUST_GLOBS,
         ("python3 -m tools.harness run fmt", "python3 -m tools.harness run code-health"),
         when="slice"),
    Rule("rust-l2", "core", AGENTS_VALIDATION, "L2: a code commit needs the correctness gate",
         RUST_GLOBS, ("python3 -m tools.harness run correctness",), when="commit"),
    Rule("persistence", "core", PERSISTENCE_DOC,
         "persistence paths need restart/recovery tests and the independent reviewer",
         PERSISTENCE_GLOBS, ("@restart-tests",), reviewer="persistence"),
    Rule("protocol", "core", PROTOCOL_DOC,
         "protocol paths need protocol tests, both versions when translation changes, and a real-client gate",
         PROTOCOL_GLOBS, ("@protocol-tests", "@no-gate:protocol"), reviewer="protocol"),
    Rule("custody", "core", f"{AGENTS_RULE5}; {TOOLING_TEST}",
         "custody changes need the conservation tests and the independent reviewer",
         CUSTODY_GLOBS, ("@custody-tests",), reviewer="custody"),
    Rule("gameplay-client-surface", "core", f"{AGENTS_VALIDATION}; docs/bugs/README.md",
         "client-surface and gameplay changes need a real-client gate: a unit test cannot close a client failure",
         GAMEPLAY_GLOBS, ("@area-gates",), exclude=GAMEPLAY_EXCLUDE, tests_trigger=False),
    Rule("changed-gate", "core", f"{AGENTS_VALIDATION}; {QA_HYGIENE}",
         "a new or changed gate needs a negative control, every new branch on a real client, "
         "and at least 3 runs when variable",
         GATE_GLOBS, ("@changed-gates",), tests_trigger=False),
    Rule("harness-python", "core", f"{AGENTS_VALIDATION} (L0, L1); docs/AGENT_TOOLING.md#harness-entrypoint",
         "harness Python: tests of the touched modules, then the whole harness suite",
         HARNESS_PY_GLOBS, ("@py-l0", HARNESS_SUITE)),
    Rule("innovation", "core", INNOVATION_DOC,
         "a new direct dependency, toolchain change or new required prerequisite spends an innovation token: "
         "choose boring technology and write a decision record",
         ("**/Cargo.toml", "rust-toolchain*", ".cargo/config.toml", "tools/harness/profiles.py"),
         ("@innovation",)),
    Rule("ledger", "core", LEDGER_DOC,
         "ledger edits run the regression gate; a status change goes through `ledger promote`",
         ("docs/bugs/ledger.toml", "docs/bugs/promotions/**"), ("@ledger",)),
    Rule("markdown", "core", AGENTS_VALIDATION,
         "Markdown-only changes need doclinks, not Cargo or harness gates",
         ("**/*.md",), ("python3 tools/doclinks.py",)),
    Rule("client-rust", "client", "../solaris-client/docs/AGENT_TESTING.md (tests, strict Clippy, native scenario)",
         "client Rust: strict Clippy and the client tests (project-wide runs by the integration owner)",
         ("**/*.rs", "Cargo.toml", "Cargo.lock", "build.rs"),
         ("cargo clippy --workspace --all-targets --locked -- -D warnings",
          "cargo test --workspace --locked"),
         when="integrate"),
    Rule("client-surface", "client", f"{GATES_DOC}; docs/bugs/README.md",
         "client changes need a native real-client gate from the core harness",
         ("src/**", "launcher/**"), ("@no-gate:client",), exclude=("**/tests/**", "**/*_test.rs"),
         tests_trigger=False, cwd="../solaris"),
    Rule("loader-gradle", "loader", "../solaris-loader/AGENTS.md#build",
         "Loader changes: the platform module tests",
         ("**/*.java", "**/*.kts", "gradle.properties"),
         ("./gradlew :loader-core:test :loader-fabric:test :loader-neoforge:test :loader-forge:test",)),
    Rule("loader-core-harness", "loader", "docs/AGENT_TOOLING.md#profiles-and-scenarios; docs/QA_AGENT_PROTOCOL.md",
         "Loader changes: the core harness Gradle profile and a real Loader client per platform",
         ("**/*.java", "**/*.kts", "gradle.properties"),
         ("python3 -m tools.harness run java",
          "python3 -m tools.harness run loader-live --platform fabric",
          "python3 -m tools.harness run loader-live --platform neoforge",
          "python3 -m tools.harness run loader-live --platform forge"),
         cwd="../solaris"),
    Rule("plugins-build", "plugins", "../solaris-default-plugins/README.md#repository-layout",
         "plugin sources and packages must byte-verify against the core SDK",
         ("sources/**", "*/plugin.toml", "*/plugin.wasm", "*/config.toml", "tools/build_standard_pack.py"),
         ("python3 tools/build_standard_pack.py --core-root ../solaris --check",)),
    Rule("plugins-harness", "plugins", "../solaris-default-plugins/README.md#repository-layout",
         "plugin changes: the core standard-pack gate, with a real client for behaviour",
         ("sources/**", "*/plugin.toml", "*/plugin.wasm", "*/config.toml", "tools/build_standard_pack.py"),
         ("python3 -m tools.harness run standard-pack",
          "python3 -m tools.harness run standard-pack --real-client"),
         cwd="../solaris"),
)

STATIC_NOTES = (
    "Path-based and minimal: the path tables are not exhaustive. A path no rule names, or one in two or "
    "more domains, is listed under manual classification, and so is any change that crosses a domain its "
    "paths do not show (for example tick/merge.rs releasing container items is custody). "
    "The absence of a class is not proof it does not apply.",
    "Owner host: wrap heavy commands in mem-run.sh and graphical gates in scene-slot.sh; real-client "
    "gates take a release server and a pinned client binary (AGENTS.md, real-client-gates.md).",
    "Results cite this plan: every gate listed is run, or named in gaps with its blocker "
    "(docs/development/agent-fleet.md#briefs).",
)


# ---------------------------------------------------------------- path logic


def _glob_regex(glob: str) -> re.Pattern[str]:
    out = []
    i = 0
    while i < len(glob):
        if glob.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
        elif glob.startswith("**", i):
            out.append(".*")
            i += 2
        elif glob[i] == "*":
            out.append("[^/]*")
            i += 1
        elif glob[i] == "?":
            out.append("[^/]")
            i += 1
        else:
            out.append(re.escape(glob[i]))
            i += 1
    return re.compile("".join(out) + r"\Z")


_GLOB_CACHE: dict[str, re.Pattern[str]] = {}


def matches(path: str, globs: tuple[str, ...]) -> bool:
    for glob in globs:
        pattern = _GLOB_CACHE.get(glob)
        if pattern is None:
            pattern = _GLOB_CACHE[glob] = _glob_regex(glob)
        if pattern.match(path):
            return True
    return False


def is_test_path(path: str) -> bool:
    name = path.rsplit("/", 1)[-1]
    return (
        name.endswith("_tests.rs") or name == "tests.rs" or name.endswith("_test.rs")
        or "/tests/" in f"/{path}" or (name.startswith("test_") and name.endswith(".py"))
    )


@dataclass(frozen=True)
class Changed:
    repo: str  # core | client | loader | plugins | outside
    rel: str


def repo_kind(root: Path) -> str:
    """Which of the four repositories ``root`` is, by marker files."""
    if (root / "tools" / "harness" / "__main__.py").is_file():
        return "core"
    for kind, markers in SIBLING_MARKERS.items():
        if all((root / marker).exists() for marker in markers):
            return kind
    return "core"


def sibling_roots(root: Path) -> dict[str, Path]:
    roots = {}
    for kind, name in SIBLING_NAMES.items():
        env = os.environ.get(SIBLING_ENV[kind])
        candidates = [Path(env)] if env else []
        candidates += [root.parent / name, root.parent.parent / name]
        roots[kind] = next((c for c in candidates if c.is_dir()), root.parent / name)
    return roots


def locate(path: str, root: Path, own: str, siblings: dict[str, Path]) -> Changed:
    """Resolve one path argument to (repository, repo-relative path)."""
    text = path.replace("\\", "/")
    for kind, name in SIBLING_NAMES.items():
        prefix = f"../{name}/"
        if text.startswith(prefix):
            return Changed(kind, text[len(prefix):])
    absolute = Path(os.path.normpath(text if os.path.isabs(text) else str(root / text)))
    for kind, base in ((own, root), *siblings.items()):
        try:
            return Changed(kind, absolute.relative_to(base).as_posix())
        except ValueError:
            continue
    return Changed("outside", text)


# ------------------------------------------------------------------ git/AST


def _git(root: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=False)


def git_root(cwd: Path) -> Path:
    done = _git(cwd, "rev-parse", "--show-toplevel")
    if done.returncode != 0:
        raise SystemExit(f"plan: {cwd} is not inside a git tree: {done.stderr.strip()}")
    return Path(done.stdout.strip())


def changed_paths(root: Path, base: str) -> list[str]:
    if _git(root, "rev-parse", "--verify", "--quiet", f"{base}^{{commit}}").returncode != 0:
        raise SystemExit(f"plan: unknown base revision {base!r}")
    diff = _git(root, "diff", "--name-only", "--no-renames", base)
    untracked = _git(root, "ls-files", "--others", "--exclude-standard")
    if diff.returncode != 0 or untracked.returncode != 0:
        raise SystemExit(f"plan: git failed: {(diff.stderr or untracked.stderr).strip()}")
    return sorted({*diff.stdout.splitlines(), *untracked.stdout.splitlines()} - {""})


def _text_at(root: Path, base: str, rel: str, current: bool) -> str | None:
    if current:
        path = root / rel
        return path.read_text(encoding="utf-8") if path.is_file() else None
    done = _git(root, "show", f"{base}:{rel}")
    return done.stdout if done.returncode == 0 else None


def scenario_catalog(text: str | None) -> dict[str, dict[str, Any]]:
    """``scenarios.py`` entries (id -> literal keyword arguments), read with ``ast``."""
    if text is None:
        return {}
    catalog: dict[str, dict[str, Any]] = {}
    for node in ast.walk(ast.parse(text)):
        targets = node.targets if isinstance(node, ast.Assign) else [node.target] if isinstance(node, ast.AnnAssign) else []
        if not any(isinstance(t, ast.Name) and t.id == "SCENARIOS" for t in targets):
            continue
        if not isinstance(node.value, ast.Dict):
            continue
        for value in node.value.values:
            if not isinstance(value, ast.Call):
                continue
            entry = {}
            for keyword in value.keywords:
                try:
                    entry[keyword.arg] = ast.literal_eval(keyword.value)
                except ValueError:
                    entry[keyword.arg] = ast.unparse(keyword.value)
            if isinstance(entry.get("id"), str):
                catalog[entry["id"]] = entry
    return catalog


def profile_fingerprints(text: str | None) -> dict[str, str]:
    """Profile name -> description, scope and run-function source, read with ``ast``."""
    if text is None:
        return {}
    tree = ast.parse(text)
    functions = {n.name: ast.unparse(n) for n in tree.body if isinstance(n, ast.FunctionDef)}
    prerequisites: dict[str, str] = {}
    profiles: dict[str, str] = {}
    for node in tree.body:
        targets = node.targets if isinstance(node, ast.Assign) else [node.target] if isinstance(node, ast.AnnAssign) else []
        names = {t.id for t in targets if isinstance(t, ast.Name)}
        if "PROFILE_PREREQUISITES" in names and isinstance(node.value, ast.Dict):
            for key, value in zip(node.value.keys, node.value.values):
                if isinstance(key, ast.Constant):
                    prerequisites[key.value] = ast.unparse(value)
        if "PROFILES" in names and isinstance(node.value, ast.Dict):
            for key, value in zip(node.value.keys, node.value.values):
                if isinstance(key, ast.Constant) and isinstance(value, ast.Dict):
                    spec = {k.value: v for k, v in zip(value.keys, value.values) if isinstance(k, ast.Constant)}
                    run = spec.get("run")
                    body = functions.get(run.id, "") if isinstance(run, ast.Name) else ast.unparse(run) if run else ""
                    profiles[key.value] = "\n".join((
                        ast.unparse(spec.get("scope", ast.Constant(""))),
                        ast.unparse(spec.get("description", ast.Constant(""))), body))
    for name in profiles:
        profiles[name] += "\n" + prerequisites.get(name, "")
    return profiles


# --------------------------------------------------------------------- plan


@dataclass
class Entry:
    gate: str
    cwd: str = "."
    why: list[str] = field(default_factory=list)
    runs: int = 1
    negative_control: bool = False
    reviewer: list[str] = field(default_factory=list)
    when: str = "edit"
    notes: list[str] = field(default_factory=list)
    paths: list[str] = field(default_factory=list)

    def as_dict(self) -> dict[str, Any]:
        return {
            "gate": self.gate, "cwd": self.cwd, "why": self.why, "runs": self.runs,
            "negative_control": self.negative_control, "reviewer": self.reviewer,
            "when": self.when, "notes": self.notes, "paths": self.paths,
        }


_WHEN_ORDER = {"edit": 0, "slice": 1, "integrate": 2, "commit": 3}


class Plan:
    """Entries keyed by (cwd, gate): rules union, duplicates merge, strictest runs win."""

    def __init__(self) -> None:
        self.entries: dict[tuple[str, str], Entry] = {}
        self.default_reviewer = ""  # applied to every entry the current rule adds

    def add(self, gate: str, *, cwd: str = ".", why: str, runs: int = 1, negative_control: bool = False,
            reviewer: str = "", when: str = "edit", notes: tuple[str, ...] = (), paths: tuple[str, ...] = ()) -> Entry:
        entry = self.entries.setdefault((cwd, gate), Entry(gate=gate, cwd=cwd, when=when))
        if why not in entry.why:
            entry.why.append(why)
        entry.runs = max(entry.runs, runs)
        entry.negative_control = entry.negative_control or negative_control
        reviewer = reviewer or self.default_reviewer
        if reviewer and reviewer not in entry.reviewer:
            entry.reviewer.append(reviewer)
        if _WHEN_ORDER[when] < _WHEN_ORDER[entry.when]:
            entry.when = when
        for note in notes:
            if note not in entry.notes:
                entry.notes.append(note)
        for path in paths:
            if path not in entry.paths:
                entry.paths.append(path)
        return entry

    def ordered(self) -> list[Entry]:
        return sorted(self.entries.values(), key=lambda e: _WHEN_ORDER[e.when])  # stable within a stage


@dataclass
class Context:
    root: Path
    base: str
    own: str
    changed: list[Changed]
    plan: Plan = field(default_factory=Plan)
    warnings: list[str] = field(default_factory=list)
    reviewers: dict[str, list[str]] = field(default_factory=dict)

    def repo_paths(self, repo: str) -> list[str]:
        return [c.rel for c in self.changed if c.repo == repo]

    def text(self, rel: str, current: bool) -> str | None:
        """File text in the target tree (current) or at the base revision."""
        root = self.root if self.own == "core" else self.root
        return _text_at(root, self.base, rel, current)

    def variable(self, profile: str) -> str | None:
        return VARIABLE_PROFILES.get(profile)


def profile_command(profile: str) -> str:
    flags = " --client native --windowed" if profile in NATIVE_WINDOWED else ""
    return f"python3 -m tools.harness run {profile}{flags}"


def scenario_command(scenario_id: str, catalog: dict[str, dict[str, Any]]) -> str:
    profile = catalog.get(scenario_id, {}).get("profile", "")
    if scenario_id.startswith("java-") or scenario_id.startswith("dh-"):
        flags = " --client java --windowed" if scenario_id.startswith("java-") else ""
    elif profile == "loader-live" or scenario_id.startswith("loader-"):
        flags = " --platform fabric"
    else:
        flags = " --client native --windowed"
    return f"python3 -m tools.harness run scenario {scenario_id}{flags}"


def _variable_note(profile: str, variable: str) -> str:
    return f"{profile} is variable ({variable}): at least 3 runs ({QA_HYGIENE}, AGENTS.md)"


def _profile_gate(ctx: Context, profile: str, why: str, *, negative_control: bool = False,
                  notes: tuple[str, ...] = (), reviewer: str = "", paths: tuple[str, ...] = ()) -> None:
    variable = ctx.variable(profile)
    if variable:
        notes = (*notes, _variable_note(profile, variable))
    ctx.plan.add(profile_command(profile), why=why, runs=3 if variable else 1,
                 negative_control=negative_control, notes=notes, reviewer=reviewer, paths=paths)


def _scenario_gate(ctx: Context, scenario_id: str, catalog: dict[str, dict[str, Any]], why: str, *,
                   negative_control: bool = False, notes: tuple[str, ...] = (), reviewer: str = "",
                   paths: tuple[str, ...] = ()) -> None:
    profile = catalog.get(scenario_id, {}).get("profile", "")
    variable = ctx.variable(profile)
    if variable:
        notes = (*notes, _variable_note(profile, variable))
    ctx.plan.add(scenario_command(scenario_id, catalog), why=why, runs=3 if variable else 1,
                 negative_control=negative_control, notes=notes, reviewer=reviewer, paths=paths)


def _unmapped(ctx: Context, why: str, paths: list[str], choices: list[str], *, negative_control: bool = False,
              cwd: str = ".", reviewer: str = "") -> None:
    ctx.plan.add(f"no mapped gate: choose from {', '.join(choices)}", cwd=cwd, why=why, runs=1,
                 negative_control=negative_control, reviewer=reviewer, paths=tuple(paths))


def real_client_profiles() -> list[str]:
    return sorted(NATIVE_WINDOWED - {"scene-shot", "survival"}) + ["core-client", "loader-live"]


# ---- expanders: (ctx, rule, hits) -> adds entries


def _crate_scope(rel: str) -> tuple[str, str, str | None]:
    """(crate, kind, selector): kind is ``lib`` filter, ``test`` target or whole ``crate``."""
    parts = rel.split("/")
    if len(parts) < 3 or parts[0] != "crates":
        return ("", "workspace", None)
    crate, tail = parts[1], parts[2:]
    if tail[0] == "src":
        module = tail[1:]
        if module[0] == "bin" or module in (["main.rs"], ["lib.rs"]) or not module[-1].endswith(".rs"):
            return (crate, "crate", None)
        module[-1] = module[-1][:-3]
        if module[-1] == "mod":
            module.pop()
        return (crate, "lib", "::".join(module)) if module else (crate, "crate", None)
    if tail[0] == "tests" and len(tail) == 2 and tail[1].endswith(".rs"):
        return (crate, "test", tail[1][:-3])
    return (crate, "crate", None)


def _minimal(filters: list[str]) -> list[str]:
    """Drop a filter that contains another as a substring: nextest filters are substrings."""
    unique = sorted(set(filters), key=lambda f: (len(f), f))
    kept: list[str] = []
    for candidate in unique:
        if not any(k in candidate for k in kept):
            kept.append(candidate)
    return sorted(kept)


def _expand_cargo_l0(ctx: Context, rule: Rule, hits: list[str]) -> None:
    by_crate: dict[str, dict[str, Any]] = {}
    workspace = [h for h in hits if _crate_scope(h)[1] == "workspace"]
    for hit in hits:
        crate, kind, selector = _crate_scope(hit)
        if kind == "workspace":
            continue
        slot = by_crate.setdefault(crate, {"filters": [], "tests": [], "crate": False, "src": [], "test_files": [], "paths": []})
        slot["paths"].append(hit)
        slot["src" if not is_test_path(hit) else "test_files"].append(hit)
        if kind == "lib":
            slot["filters"].append(selector)
        elif kind == "test":
            slot["tests"].append(selector)
        else:
            slot["crate"] = True
    base = "python3 -m tools.harness run test"
    for hit in workspace:
        ctx.plan.add(base, why=f"{rule.source}: {rule.why} (workspace manifest)", paths=(hit,))
    for crate, slot in sorted(by_crate.items()):
        control = any(p.endswith(".rs") for p in slot["src"])
        notes = ()
        if control and not slot["test_files"]:
            notes = (f"no test file changed in {crate}: a behaviour change needs a regression test",)
        why = (f"{rule.source}: {rule.why}"
               + ("; behaviour change: its regression test goes red when only the fix hunks are reverted"
                  if control else ""))
        paths = tuple(slot["paths"])
        if slot["crate"]:
            ctx.plan.add(f"{base} -p {crate}", why=why, negative_control=control, notes=notes, paths=paths)
            continue
        if slot["filters"]:
            gate = f"{base} -p {crate} --lib {' '.join(_minimal(slot['filters']))}"
            ctx.plan.add(gate, why=why, negative_control=control, notes=notes, paths=paths)
        for target in sorted(set(slot["tests"])):
            ctx.plan.add(f"{base} -p {crate} --test {target}", why=why, negative_control=control,
                         notes=notes, paths=paths)


def _crates(hits: list[str], only: tuple[str, ...] = ()) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    for hit in hits:
        crate = _crate_scope(hit)[0]
        if crate and (not only or crate in only):
            out.setdefault(crate, []).append(hit)
    return out


def _expand_restart_tests(ctx: Context, rule: Rule, hits: list[str]) -> None:
    for crate, paths in sorted(_crates(hits).items()):
        ctx.plan.add(
            f"python3 -m tools.harness run test -p {crate} --lib {' '.join(PERSISTENCE_FILTERS)}",
            why=f"{rule.source}: {rule.why}", paths=tuple(paths),
            notes=("a change to persistence order or recovery also updates docs/architecture/persistence.md "
                   "and the owning ADR (AGENTS.md#project-invariants)",))


def _expand_protocol_tests(ctx: Context, rule: Rule, hits: list[str]) -> None:
    why = f"{rule.source}: {rule.why}"
    ctx.plan.add("python3 -m tools.harness run test -p mc-protocol", why=why, paths=tuple(hits))
    edge = _crates(hits, ("mc-edge",)).get("mc-edge")
    if edge:
        ctx.plan.add(
            f"python3 -m tools.harness run test -p mc-edge --lib {' '.join(PROTOCOL_EDGE_FILTERS)}",
            why=f"{why}; exercise both versions when translation changes", paths=tuple(edge))


def _expand_custody_tests(ctx: Context, rule: Rule, hits: list[str]) -> None:
    core = _crates(hits, ("mc-core",)).get("mc-core")
    if core:
        ctx.plan.add(
            f"python3 -m tools.harness run test -p mc-core --lib {' '.join(CUSTODY_FILTERS)}",
            why=f"{rule.source}: {rule.why}; inventory::conservation_tests item totals never change",
            paths=tuple(core))


def _expand_no_gate(ctx: Context, rule: Rule, hits: list[str], kind: str) -> None:
    why = f"{rule.source}: {rule.why}"
    paths = [h for h in hits if not is_test_path(h)]
    if not paths:
        return
    reviewer = ""
    if kind == "protocol":
        choices = ["core-client", "title-connect", "entities", "native-lifecycle", "loader-live"]
    else:
        choices = real_client_profiles()
    cwd = rule.cwd
    ctx.plan.add(f"no mapped gate: choose from {', '.join(choices)}", cwd=cwd, why=why, paths=tuple(paths),
                 reviewer=reviewer)


def _expand_area_gates(ctx: Context, rule: Rule, hits: list[str]) -> None:
    catalog = scenario_catalog(ctx.text(SCENARIO_CATALOG, True))
    unmapped = []
    for hit in hits:
        areas = [a for a in AREAS if matches(hit, a.globs)]
        if not areas:
            if not matches(hit, PERSISTENCE_GLOBS):
                unmapped.append(hit)
            continue
        for area in areas:
            for gate in area.gates:
                kind, _, name = gate.partition(":")
                why = f"{rule.source}: area {area.id} -> {gate} ({area.source})"
                if kind == "profile":
                    _profile_gate(ctx, name, why, paths=(hit,))
                else:
                    _scenario_gate(ctx, name, catalog, why, paths=(hit,))
    if unmapped:
        _unmapped(ctx, f"{rule.source}: {rule.why}; no area maps these paths", unmapped, real_client_profiles())


def _backend_stem(rel: str) -> str:
    return rel.rsplit("/", 1)[-1].rsplit(".", 1)[0]


def _expand_changed_gates(ctx: Context, rule: Rule, hits: list[str]) -> None:
    why = f"{rule.source}: {rule.why}"
    notes = ("exercise every new or changed branch on a real client; assertions must observe something "
             "the action produced (no pre-existing pass)",)
    current = scenario_catalog(ctx.text(SCENARIO_CATALOG, True))
    base_catalog = scenario_catalog(ctx.text(SCENARIO_CATALOG, False))
    profile_ids: dict[str, list[str]] = {}
    scenario_ids: dict[str, list[str]] = {}
    unmapped: list[str] = []

    def add_profile(name: str, path: str) -> None:
        profile_ids.setdefault(name, []).append(path)

    def add_scenario(name: str, path: str) -> None:
        scenario_ids.setdefault(name, []).append(path)

    for hit in hits:
        if hit == SCENARIO_CATALOG:
            for sid, entry in current.items():
                if base_catalog.get(sid) != entry:
                    add_scenario(sid, hit)
            continue
        if hit == PROFILE_CATALOG:
            now = profile_fingerprints(ctx.text(PROFILE_CATALOG, True))
            before = profile_fingerprints(ctx.text(PROFILE_CATALOG, False))
            for name, fingerprint in now.items():
                if before.get(name) != fingerprint:
                    add_profile(name, hit)
            continue
        profiles = BACKEND_PROFILES.get(_backend_stem(hit), ())
        for name in profiles:
            add_profile(name, hit)
        sources = [sid for sid, entry in current.items() if hit in entry.get("sources", ())]
        if len(sources) > SCENARIO_COLLAPSE:
            for sid in sources:
                profile = current[sid].get("profile")
                if profile:
                    add_profile(profile, hit)
        else:
            for sid in sources:
                add_scenario(sid, hit)
        if not profiles and not sources:
            unmapped.append(hit)
    for name, paths in sorted(profile_ids.items()):
        _profile_gate(ctx, name, why, negative_control=True, notes=notes, paths=tuple(paths))
    for sid, paths in sorted(scenario_ids.items()):
        _scenario_gate(ctx, sid, current, why, negative_control=True, notes=notes, paths=tuple(paths))
    if unmapped:
        _unmapped(ctx, why, unmapped, real_client_profiles(), negative_control=True)


def _expand_py_l0(ctx: Context, rule: Rule, hits: list[str]) -> None:
    modules = []
    for hit in hits:
        name = hit.rsplit("/", 1)[-1]
        directory = hit.rsplit("/", 1)[0]
        test = hit if name.startswith("test_") else f"{directory}/test_{name}"
        if (ctx.root / test).is_file():
            modules.append(test[:-3].replace("/", "."))
    if modules:
        ctx.plan.add(f"python3 -m unittest {' '.join(sorted(set(modules)))}",
                     why=f"{rule.source}: {rule.why} (L0: the affected scope)", paths=tuple(hits))


def _ledger_rows(text: str | None) -> dict[str, dict[str, Any]]:
    if text is None:
        return {}
    return {row["id"]: row for row in tomllib.loads(text).get("bug", []) if "id" in row}


def _expand_ledger(ctx: Context, rule: Rule, hits: list[str]) -> None:
    why = f"{rule.source}: {rule.why}"
    ctx.plan.add("python3 -m tools.harness run regressions", why=why, paths=tuple(hits), when="integrate")
    if "docs/bugs/ledger.toml" not in hits:
        return
    try:
        now = _ledger_rows(ctx.text("docs/bugs/ledger.toml", True))
        before = _ledger_rows(ctx.text("docs/bugs/ledger.toml", False))
    except (tomllib.TOMLDecodeError, KeyError) as error:
        ctx.warnings.append(f"ledger.toml could not be compared with the base: {error}")
        return
    changed = sorted(i for i, row in now.items() if i in before and row.get("status") != before[i].get("status"))
    added_fixed = sorted(i for i, row in now.items() if i not in before and row.get("status") == "fixed")
    promoted = sorted(set(changed) | set(added_fixed))
    for row_id in promoted:
        row = now[row_id]
        if row.get("status") == "fixed" and not row.get("promotion"):
            ctx.warnings.append(f"{row_id}: status is fixed without a promotion receipt; only `ledger promote` sets it")
        ctx.plan.add(f"python3 -m tools.harness ledger promote {row_id}", why=f"{LEDGER_DOC}: status changed",
                     paths=("docs/bugs/ledger.toml",), when="integrate",
                     notes=("promote runs its own automatic negative control and needs the real-client "
                            "receipt for client rows",))
    if any(now[i].get("area") == "client" or now[i].get("repro") in ("manual", "client") for i in promoted):
        ctx.plan.add("python3 -m tools.harness run regressions-client",
                     why="docs/bugs/README.md: run when a client-surface fix is promoted",
                     paths=("docs/bugs/ledger.toml",), when="integrate")
    if len(promoted) >= LEDGER_BATCH_MIN:
        ctx.reviewers.setdefault("ledger-batch", []).append(
            f"{AGENTS_RULE5}: {len(promoted)} rows changed status (batch of {LEDGER_BATCH_MIN} or more)")
        for entry in ctx.plan.entries.values():
            if entry.gate.startswith("python3 -m tools.harness ledger promote"):
                if "ledger-batch" not in entry.reviewer:
                    entry.reviewer.append("ledger-batch")


def _dependency_names(text: str | None) -> tuple[dict[str, dict[str, Any]], set[str]]:
    """(direct dependency name -> spec across the dependency tables, [workspace.dependencies] names)."""
    if text is None:
        return {}, set()
    data = tomllib.loads(text)
    tables = [data.get(t, {}) for t in DEP_TABLES]
    for target in data.get("target", {}).values():
        tables += [target.get(t, {}) for t in DEP_TABLES]
    direct = {name: spec for table in tables for name, spec in table.items()}
    return direct, set(data.get("workspace", {}).get("dependencies", {}))


def _spec_is_path(spec: Any) -> bool:
    return isinstance(spec, dict) and "path" in spec


def _string_tokens(node: ast.AST, env: dict[str, ast.AST]) -> set[str]:
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return {node.value}
    if isinstance(node, (ast.Tuple, ast.List)):
        return set().union(*(_string_tokens(e, env) for e in node.elts)) if node.elts else set()
    if isinstance(node, ast.Name) and node.id in env:
        return _string_tokens(env[node.id], env)
    return set()


def _prerequisite_tokens(text: str | None) -> dict[str, set[str]]:
    """profile -> required-prerequisite tokens from ``PROFILE_PREREQUISITES`` (names resolved)."""
    if text is None:
        return {}
    tree = ast.parse(text)
    env = {t.id: n.value for n in tree.body if isinstance(n, ast.Assign) for t in n.targets
           if isinstance(t, ast.Name) and t.id != "PROFILE_PREREQUISITES"}
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "PROFILE_PREREQUISITES"
                                                for t in node.targets) and isinstance(node.value, ast.Dict):
            return {k.value: _string_tokens(v, env) for k, v in zip(node.value.keys, node.value.values)
                    if isinstance(k, ast.Constant)}
    return {}


def _innovations(ctx: Context, hits: list[str]) -> list[tuple[str, str, str]]:
    """(label, path, what) per innovation token: new crate, toolchain change, new prerequisite."""
    found: list[tuple[str, str, str]] = []
    _, workspace_before = _dependency_names(ctx.text("Cargo.toml", False))
    for hit in hits:
        name = hit.rsplit("/", 1)[-1]
        if name == "Cargo.toml":
            try:
                before, ws_before = _dependency_names(ctx.text(hit, False))
                now, ws_now = _dependency_names(ctx.text(hit, True))
            except tomllib.TOMLDecodeError as error:
                ctx.warnings.append(f"{hit}: could not be parsed for new dependencies: {error}")
                continue
            for dep in sorted(set(now) - set(before)):
                spec = now[dep]
                if _spec_is_path(spec):
                    continue  # a workspace crate, not an outside technology
                if isinstance(spec, dict) and spec.get("workspace") and dep in workspace_before:
                    continue  # already a workspace dependency at base
                found.append((f"new dependency {dep}", hit, "added to a dependency table"))
            for dep in sorted(ws_now - ws_before):
                found.append((f"new dependency {dep}", hit, "added to [workspace.dependencies]"))
        elif name.startswith("rust-toolchain") or hit == ".cargo/config.toml":
            if ctx.text(hit, True) != ctx.text(hit, False):
                found.append((f"toolchain or cargo config change {hit}", hit, "toolchain"))
        elif hit == PROFILE_CATALOG:
            before = _prerequisite_tokens(ctx.text(hit, False))
            now = _prerequisite_tokens(ctx.text(hit, True))
            known = set().union(*before.values()) if before else set()
            for profile, tokens in sorted(now.items()):
                for token in sorted(tokens - known):
                    found.append((f"new required prerequisite {token} (profile {profile})", hit, "prerequisite"))
    seen: set[str] = set()
    return [f for f in found if not (f[0] in seen or seen.add(f[0]))]


def _expand_innovation(ctx: Context, rule: Rule, hits: list[str]) -> None:
    for label, path, _what in _innovations(ctx, hits):
        ctx.plan.add(f"decision record required ({INNOVATION_DOC.split('#')[0]} innovation checklist): {label}",
                     why=f"{rule.source}: {rule.why}", paths=(path,),
                     notes=("answer: what we do with what we already have, why that is prohibitively hard, "
                            "and what the new thing replaces (an open-transitions row)",))


EXPANDERS: dict[str, Callable[..., None]] = {
    "@innovation": _expand_innovation,
    "@cargo-l0": _expand_cargo_l0,
    "@restart-tests": _expand_restart_tests,
    "@protocol-tests": _expand_protocol_tests,
    "@custody-tests": _expand_custody_tests,
    "@area-gates": _expand_area_gates,
    "@changed-gates": _expand_changed_gates,
    "@py-l0": _expand_py_l0,
    "@ledger": _expand_ledger,
}


def domains(repo: str, rel: str) -> list[str]:
    """Risk domains a core path belongs to; two or more need a human to classify it."""
    if repo != "core":
        return []
    found = [name for name, globs in (("persistence", PERSISTENCE_GLOBS), ("protocol", PROTOCOL_GLOBS),
                                      ("custody", CUSTODY_GLOBS)) if matches(rel, globs)]
    if matches(rel, GATE_GLOBS):
        found.append("gate")
    if not found and matches(rel, GAMEPLAY_GLOBS) and not matches(rel, GAMEPLAY_EXCLUDE):
        found.append("gameplay")
    return found


def manual_classification(changed: list[Changed], matched: set[tuple[str, str]]) -> list[dict[str, Any]]:
    """Paths no rule names, and paths that fall in two or more domains: never silent."""
    out = []
    for change in changed:
        found = domains(change.repo, change.rel)
        if (change.repo, change.rel) not in matched:
            out.append({"path": f"{change.repo}:{change.rel}" if change.repo != "core" else change.rel,
                        "reason": "no rule matches this path", "domains": found})
        elif len(found) >= 2:
            out.append({"path": change.rel, "reason": f"spans domains {', '.join(found)}: the union of "
                        "their rules is printed, but the classification is a human decision",
                        "domains": found})
    return out


def build_plan(root: Path, base: str, paths: list[str]) -> dict[str, Any]:
    own = repo_kind(root)
    siblings = sibling_roots(root)
    changed = [locate(p, root, own, siblings) for p in paths]
    ctx = Context(root=root, base=base, own=own, changed=changed)
    matched: set[tuple[str, str]] = set()
    classes: dict[str, list[str]] = {}
    for rule in RULES:
        hits = []
        for change in changed:
            if change.repo != rule.repo or not matches(change.rel, rule.globs):
                continue
            if rule.exclude and matches(change.rel, rule.exclude):
                continue
            matched.add((change.repo, change.rel))
            if is_test_path(change.rel) and not rule.tests_trigger:
                continue
            hits.append(change.rel)
        if not hits:
            continue
        classes[rule.id] = hits
        reviewer_hits = [h for h in hits if not is_test_path(h)]
        ctx.plan.default_reviewer = rule.reviewer if reviewer_hits else ""
        for spec in rule.gates:
            if spec.startswith("@no-gate:"):
                _expand_no_gate(ctx, rule, hits, spec.partition(":")[2])
            elif spec in EXPANDERS:
                EXPANDERS[spec](ctx, rule, hits)
            else:
                ctx.plan.add(spec, cwd=rule.cwd, why=f"{rule.source}: {rule.why}", when=rule.when,
                             paths=tuple(hits))
        ctx.plan.default_reviewer = ""
        if rule.reviewer and reviewer_hits:
            ctx.reviewers.setdefault(rule.reviewer, []).append(
                f"{rule.source}: {rule.why} ({len(reviewer_hits)} non-test path(s))")
    manual = manual_classification(changed, matched)
    entries = ctx.plan.ordered()
    return {
        "root": str(root), "base": base, "repo": own, "changed_files": len(changed),
        "classes": {k: v for k, v in classes.items()},
        "entries": [e.as_dict() for e in entries],
        "reviewers": [
            {"kind": kind, "required": True, "what": REVIEWER_KINDS[kind], "why": why}
            for kind, why in sorted(ctx.reviewers.items())
        ],
        "manual_classification": manual, "warnings": ctx.warnings, "notes": list(STATIC_NOTES),
    }


# ------------------------------------------------------------------- output


def render(plan: dict[str, Any]) -> str:
    lines = [f"plan: base {plan['base']}, {plan['changed_files']} changed path(s) in {plan['root']} "
             f"(repo: {plan['repo']})"]
    lines.append("classes: " + (", ".join(f"{k}({len(v)})" for k, v in plan["classes"].items()) or "none"))
    lines.append("")
    lines.append(f"{'#':>2}  {'when':<9} {'runs':<4} {'neg-ctl':<7} {'reviewer':<22} gate")
    for index, entry in enumerate(plan["entries"], 1):
        reviewer = ("yes: " + ",".join(entry["reviewer"])) if entry["reviewer"] else "no"
        gate = entry["gate"] if entry["cwd"] == "." else f"[in {entry['cwd']}] {entry['gate']}"
        lines.append(f"{index:>2}  {entry['when']:<9} {entry['runs']:<4} "
                     f"{'yes' if entry['negative_control'] else 'no':<7} {reviewer:<22} {gate}")
        for why in entry["why"]:
            lines.append(f"      why: {why}")
        for note in entry["notes"]:
            lines.append(f"      note: {note}")
        if entry["gate"].startswith("no mapped gate"):
            lines.append(f"      for: {', '.join(entry['paths'])}")
    if not plan["entries"]:
        lines.append("   (no gate: nothing changed)")
    for reviewer in plan["reviewers"]:
        lines.append("")
        lines.append(f"reviewer required [{reviewer['kind']}]: {reviewer['what']}")
        for why in reviewer["why"]:
            lines.append(f"      why: {why}")
    if plan["manual_classification"]:
        lines.append("")
        lines.append("manual classification required: "
                     + ", ".join(m["path"] for m in plan["manual_classification"]))
        for item in plan["manual_classification"]:
            lines.append(f"  {item['path']}: {item['reason']}")
    for warning in plan["warnings"]:
        lines.append(f"warning: {warning}")
    lines.append("")
    lines.extend(f"note: {note}" for note in plan["notes"])
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="python3 -m tools.harness plan",
        description="Print the validation a change requires, from the repository's own rules.")
    parser.add_argument("--base", default="HEAD", help="revision the changed files are compared with (default HEAD)")
    parser.add_argument("--paths", nargs="+", metavar="PATH", default=None,
                        help="changed paths instead of the git diff; ../solaris-client/... names a sibling repo")
    parser.add_argument("--json", action="store_true", help="print JSON instead of the table")
    args = parser.parse_args(argv)
    root = git_root(Path.cwd())
    paths = args.paths if args.paths is not None else changed_paths(root, args.base)
    plan = build_plan(root, args.base, paths)
    print(json.dumps(plan, indent=2) if args.json else render(plan))
    return 0


if __name__ == "__main__":
    sys.exit(main())
