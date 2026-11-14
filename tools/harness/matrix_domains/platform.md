# Platform domain adapters

These modules use the shared `Adapter` interface in `matrix_api.py`, not another runner or executable YAML language:

- `advancements_stats`, `block_log`, `computer`, `loader_content`, `monitor`
- `plugins`, `resource_packs`, `scoreboard_teams`, `sculk_trials`, `ui_presentation`, `voice`

`_platform.py` holds shared received-state helpers. Missing fields, optional verbs and fixtures raise named `MissingCapability`; unknown state never becomes air, an empty inventory or a successful oracle. Protocol input submission is not success: negative block-use checks require the returned sequence and a received `block_ack_sequence`, followed by their block/inventory oracle. Arena setup force-loads its full region through the canonical received-chunk callback. Tick waits use the published dashboard `simulation_tick` counter and event-driven waiting.

The plain-carrier refusal fixture publishes stone then air and waits for both states on every observer before input. Delta-only clients therefore have an actual received baseline; a missing block-state entry remains pending, never implicit air. The oracle asserts the resulting authoritative state, not an unobserved graphical prediction correction.

Plugin setup preserves the complete effective package set selected before server startup, copies it into the run artifact, changes only requested package configuration and calls `server.reconfigure`. Replacing packages in the existing world would violate its checkpointed native startup policy. A requested package absent from the initial set is an explicit fixture capability failure, not a runtime package swap. Ordinary fixtures preserve core auth settings and login automation. Auth-negative fixtures explicitly require server login and disable only client login automation before reconnecting. No changes are made to source live-plugin packages.

A real oracle body is not evidence that a row ran. Complete subject-specific assertions can still require unavailable graphical/custom-channel observations. Explicit simultaneous-custody and all-actor rows remain blocked until the domain supplies the shared authoritative concurrency receipts and own/shared result checks. Ordinary two-client scenarios need an actual received observer result; a second socket alone is not coverage. Graphical, audio, screen-capture, SVC and native voice observations are never synthesized from protocol login.

The deployed chisel manifest, not its stale README, is authoritative: its bundle is optional. Vanilla joins and sees the stick carrier; consenting Loader clients must demonstrate custom rendering. The required-bundle refusal scenario uses a modified isolated fixture. Source references and exact per-subject matrix IDs live in the requested `scenarios/domains/<domain>.yaml` source indexes; capability requests, completeness counts and real receipts are separate artifacts.

Focused contract tests: `python3 -m unittest tools.harness.test_matrix_platform`. They exercise admission, observer routing, copied configuration, sequence fencing and failure retention; they are not live gameplay or graphical receipts. Live execution uses the canonical `protocol-matrix`, Java or native profile and the machine's current memory-admission policy. Supplemental `--subject` runs are not matrix ledger rows.
