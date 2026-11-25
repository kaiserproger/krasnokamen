# DRAGON-EGG-SKIPS-A-COLD-TARGET-THAT-VANILLA-WOULD-LOAD — fix-hunk map

**Promoted** as `patch:92ca0cba…` (promotion `20261125T004214-ledger-promote-v_83wwku`, tree `f3382ce42`,
red on the ledger's own reversal at `interact_parity_tests.rs:678` — the missing second priority-0
demand). The fix itself is the scaffold from `70d39713e` plus the wave2-13 M1/m2 changes it now carries;
this file is the record, not a second patch.

Fix commit: the one that adds this file's row refs (see the ledger row's `fix_commit`). Design:
`receipts/D123/D123Battle/DRAGON-EGG-TICK-NOTE.md` (Main's OD-1 option 1).

| file | what it does |
|---|---|
| `crates/mc-core/src/vanilla_interact.rs` | `dragon_egg` records a cold sampled target instead of skipping it; `EggTeleport` (target, source state, 40-tick budget), `TickState::pending_egg_chunks` (the demand) and `TickState::complete_pending_egg_teleports` (the move, dropped when the egg moved/changed, the cell is not air, or the budget expired) |
| `crates/mc-core/src/tick/residency.rs` | `desired_chunks` adds each pending egg chunk at priority 0 (one loop after the player/ticket/active demand; the demand is read as a snapshot before the run) |
| `crates/mc-core/src/tick/dimension.rs` | one call after `world.ensure` in the chunk phase, so the move is written through `merge_block_writes` for this tick's block phase |
| `crates/mc-core/src/tick.rs`, `crates/mc-core/src/tick/state.rs` | the `pending_egg_teleports: Vec<(BlockPos, EggTeleport)>` field and its initialiser (derived runtime state, never persisted: a checkpoint mid-flight drops the record) |
| `crates/mc-core/src/interact_parity_tests.rs` | the two refs: the tick-driven demand-path proof and the stale-record drops |

Not changed: the border/protected/plugin refusals, the 1000-sample order, and the resident fast path
(`DRAGON-EGG-TELEPORTS-INTO-COLD-CHUNK-OVERLAY`'s guarantee that no write lands in a cold overlay —
`interact_parity_dragon_egg_never_teleports_into_a_cold_overlay` stays green).

## Control

Reverse the `tick/dimension.rs` call (the completion hunk): `interact_parity_dragon_egg_requests_a_cold_target_through_the_demand_path`
reds at the emptied origin, the other four tests stay green. Seen 2026-11-25.
