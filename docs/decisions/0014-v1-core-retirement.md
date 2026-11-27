# ADR 0014 — Retire the v1 core

**Date:** 2026-10-15  
**Status:** Accepted; complete. Source retirement finished 2026-10-15; the owner
dropped the old-binary A/B comparator on 2026-10-16. Checked 2026-11-27: none
of `mc-server`, `mc-net`, `mc-simulation`, `mc-entity` exists under `crates/`.

## Decision

The native server, `mc-edge`'s `krasnokamen` binary over `mc-core`
([ADR 0013](0013-core-model-v2.md)), is the only server. The v1 crates were
deleted in one cutover, without compatibility shims and without changing native
behaviour. What v1 still provided moved to its own crate first:

- `mc-loader`: the Loader manifest, artifact, acknowledgement and sound wire
  contract ([ADR 0010](0010-krasnokamen-loader-contract.md));
- `mc-vanilla`: vanilla-jar process control, registry capture and the minimal
  protocol client used by content import and the harness.

Every harness server lifecycle launches `krasnokamen`; there are no variant
flags or environment selectors. The operator surface is content import and
verify, backup and restore, stdin vanilla commands with vanilla `ops.json`, the
offline `world` commands, and native `world pregenerate --radius` in place of
the old Anvil rectangle pregeneration. There is no legacy `--check`, TUI,
`operator` subcommand or `check-world`. There is no frozen old binary and no
A/B tooling; `tools/coreperf` measures the native server only.

## Consequences

- Superseded Krasnokamen APIs and authorities are deleted once their callers
  move; there is no released compatibility surface (AGENTS.md).
- The v1 decision records 0003–0007, 0009 and 0011 were tombstones and are
  removed; their text is in Git history. Retained principles live in
  ADR 0013 (plugin admission) and the
  [runtime map](../architecture/runtime.md#connection-liveness) (keepalive).
- Historical receipts keep their original tree and meaning; they are never
  relabelled as native evidence.
