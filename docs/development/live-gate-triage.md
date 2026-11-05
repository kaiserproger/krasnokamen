# Triage of failing live gates

A live gate here is any run against a real server process: harness profiles,
the plugin repository's integration gates (`../solaris-default-plugins`,
`tools/run_integration.py`, `standard-pack`), and bot or client scenarios.
Receipt reading is in [AGENT_TOOLING.md](../AGENT_TOOLING.md) "Receipts and
failure diagnosis"; this page is the method and the causes seen so far.

## Method

1. **Read the receipt, then the source.** Start from `result.json` (nonzero
   exit, its command and log). Verify every hypothesis in source or logs before
   spending another 5-60 minute run, and never forward an agent's or advisor's
   claim as fact without a `file:line` you read.
2. **Stacked causes hide each other.** Fix them in the order they appear and
   rerun once per fix. When many fixtures fail the same way, look for the common
   factor (the source state), not the per-fixture target.
3. **Temporary tracing.** When the harness cannot reach crate-private state, add
   temporary `eprintln!`/`tracing` lines with a unique marker inside the crate,
   run one serial gate so stderr lands in the server log, read it, turn the
   finding into a deterministic failing-first test, then remove every trace
   (grep for zero markers) and rerun.
4. **Narrow fixes, mutation-checked.** A reconciliation fix names the exact
   case it handles (action, intent, state, epoch). Prove its test bites by
   temporarily restoring the broad condition and watching it fail. A broad
   "status poll completes pending work" fix once let a pending start intent leak
   into a mutation path; read-only actions must never mutate on a generic
   condition.
5. **Classify before fixing product code:** harness/fixture artifact,
   environment (load, GPU, disk), stale binary, or product defect. See the false
   alarms in [real-client gates](real-client-gates.md).
6. **Don't over-instrument the API.** Speculative WIT/API fields added for a
   path that was later abandoned had to be reverted across core, SDK, tools and
   seven manifests (2026-10).

## Stale or wrong binaries

- `cargo test` does not rebuild the `solaris-v2` binary. Check
  `server-build.log` or the receipt's commands for `--bin solaris-v2 -p mc-edge`
  and the binary's identity before trusting a run.
- Pooled or shared target dirs can serve another tree's artifacts; see the
  2026-11-04 entry in [lessons](lessons.md). Symptoms: "unresolved import" for a
  symbol that exists, or new tests silently absent. Never `touch
  crates/*/src/main.rs` to force rebuilds: it created empty bin targets (E0601)
  and an empty `crates/xtask/src/lib.rs` once.
- A green gate counts only for the binary, plugin package and content it ran.
  Plugin or package identity changes need the affected gates rerun even when
  the core tree and L2 are unchanged.

## Plugins and guests

- **All plugin commands go silent or become "Unknown command" after one step:**
  grep the server log for `plugin instance retired`. The host retires an
  instance on an invalid guest answer. Causes seen: an operation id outside
  `[a-z0-9_-]` or over 64 bytes (`validate_script_id_value` in
  `crates/mc-script/src/lib.rs`; storage keys use a different validator), and a
  storage value over `MAX_PLUGIN_STORAGE_VALUE_BYTES` (4096) from an unbounded
  list stored as one value. Fix in the plugin with a bounded or digest form and
  a worst-case unit test.
- **API/WIT version.** A guest's API version must equal
  `COMPONENT_PLUGIN_API_VERSION`. A WIT change invalidates every prebuilt
  `plugin.wasm` (`failed to convert function to given type` at load); rebuild
  every first-party package and bump each `plugin.toml`. Don't bump without a
  surviving consumer. A capability name unknown to core's manifest parser is a
  core bug, not a plugin bug.
- **Candidates are immutable.** Build each candidate into a new output dir with
  `../solaris-default-plugins/tools/build_standard_pack.py`, verify its `content-receipt.json` input hashes
  against the sources you changed, publish one candidate per core state, and
  never write into a shared candidate. Passes on mixed candidates are
  provisional; Main orders one consolidated build and reruns all affected gates.
  The pack fingerprint fails if inputs change mid-build: hand over write
  ownership explicitly.
- **Admission is not completion.** Order and route receipts acknowledge
  admission; arrival or cancellation comes from a later query of the native
  state. An `Applied` move proves the slot and route check only; the body can
  still never arrive.
- **Refusal-only receipts prove nothing about the feature.** Acceptance that
  asks for a working feature needs world evidence (block states before/after,
  item transfers), not reply strings or HP loss.
- Plugin operation receipts are never pruned by count or age: eviction would
  let a retried operation id execute twice.

## Server/client protocol stalls (v2 history, still instructive)

- A vanilla Java client stalled when the server sent nothing to an idle player:
  the server must send `SetTime` periodically (vanilla does every 20 ticks).
- After a same-dimension respawn the Java client stays on the loading screen
  unless the respawn burst includes GameEvent 13 (start waiting for chunks) and
  the server forgets and resends the whole view.
- A best-effort packet dropped after the server optimistically marked the
  viewer's cache left a mob invisible forever; drops must undo the mark.
- Encoding the whole join view inline on the connection task (about 11 s in a
  debug build) starved the client; chunk streaming belongs to the pump.
- On failure read the server log (the edge logs peer, stage and reason for each
  close) and the client log for client-side rejects; `prepared` is not a pass and
  every timeout is a failure.

## Retired: settlements live-gate matrix (2026-10)

The 13-gate settlements matrix (commerce, production, caravan, war) in the
plugin repository's integration tests ran against the v1 core. Those tests
were archived when v1 was retired (ADR 0014, 2026-10-15) and their file and
symbol pointers no longer exist. Durable lessons from that work are folded into
the sections above.
