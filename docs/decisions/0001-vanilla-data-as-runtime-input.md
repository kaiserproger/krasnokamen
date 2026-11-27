# ADR 0001 — Vanilla data as local runtime input

**Status:** Accepted; implemented. Checked against the code on 2026-11-27
(`crates/mc-edge/src/content_cache.rs`, `operator_cli.rs`, `server.rs`).

## Decision

Krasnokamen serves Minecraft Java 26.3 and translates for the older releases of
its support window ([ADR 0015](0015-multiversion-protocol-translation.md)).
Registry, tag, recipe, loot, component, block-property, protocol and world-format
facts come from the matching vanilla distribution the operator owns, never from
remembered layouts or guessed values.

- **Observe, do not copy.** Extraction, `wire-probe`, `javap`
  ([ADR 0002](0002-vanilla-protocol-metadata-as-reference.md)) and side-by-side
  runs establish observable behaviour. They never authorize copying Mojang
  implementations or mechanically translating them into Rust.
- **No Mojang bytes in Git**, crates or release bundles. `data/vanilla*` and
  `.analysis/` stay local and ignored.
- **The operator derives the cache explicitly**:
  `krasnokamen content import [--version 26.3] [--from <jar>] [--gameplay-from <jar>] [--download] [--cache <dir>]`.
  `--from` is the canonical server bundle and `--gameplay-from` the 26.1.2
  bundle whose data pack the canonical cache uses; a bundle not supplied is
  downloaded, with size and SHA-1 pinned by Mojang's metadata. One import
  installs the canonical cache, the other endpoints of the support window and
  the adjacent translation steps. Import stages and validates before it
  publishes; a failure keeps the previous cache. `content verify` validates the
  same cache offline, without starting the world or the listener.
- **Serving discovers and validates; only a first start with no cache at all
  imports.** When no candidate location holds a cache, the server runs the same
  window-wide importer (with download) before it opens a world or a listener;
  it never overwrites an existing cache, broken or not. Search order:
  `[data].vanilla_data_dir`, then `KRASNOKAMEN_CONTENT_CACHE`, then
  `$XDG_DATA_HOME/krasnokamen/content/<release>` (else `~/.local/share/…`), then
  `data/vanilla` in the working directory and up to four of its ancestors. A
  configured or environment override that is invalid fails startup instead of
  falling through. A missing cache names every searched location and the import
  command.
- **One owned copy.** Loot catalogs, the light oracle and other startup tables
  load once from the selected root into `CoreRegistry`; consumers borrow them.
  There is no process-global cache, alternative discovery path or compiled-in
  vanilla copy. The tracked `crates/mc-data/data` fixtures are test facts, not
  an operator startup mode.

## Ownership

| Crate | Owns |
|---|---|
| `mc-data` | decoded immutable data and its loaders |
| `mc-edge` | cache discovery, import and startup composition |
| `mc-core` | consumes the validated registries |
| `mc-vanilla` | vanilla process control and registry capture used by import and the harness |

## Evidence

- Search order and the authoritative override: `content_cache.rs:105-176`,
  `content_cache.rs:243-280` (error text names the searched roots).
- First-start import: `content_cache::prepare_startup`, called from
  `bin/krasnokamen.rs:36`; offline commands only resolve
  (`server::resolve_content_config`).
- Import flags: `operator_cli.rs:58-80`.

A data-loading change keeps complete-cache validation, actionable errors and
failure before the listener starts. It is not gameplay proof; client-visible
changes need their own protocol and client scope. Operator detail lives in
[operating](../OPERATING.md#vanilla-content-cache).
