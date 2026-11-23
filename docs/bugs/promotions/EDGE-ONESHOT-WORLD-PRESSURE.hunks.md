# EDGE-ONESHOT-WORLD-PRESSURE — fix-hunk map

The row's fix is `60f1d475b` (`fix(edge): coalesce explosion impulses and detect skipped knockback`) plus
the API it needs; this patch is the forward-ported fix at HEAD, SHA-256 of its non-test hunks
`patch:369819164c76bf8efbe48a506fa3ce818ea23e047c9aa915aa2069f217b6657b` (file
`docs/bugs/promotions/EDGE-ONESHOT-WORLD-PRESSURE.patch`). It reverse-applies on the candidate.

A blanket reversal of the commit does not build (the commit's own kept tests call `close_skipped_knockback`
and read `WorldSnapshot::player_knockback_ticks`), so this is a curated HEAD-equivalent patch that keeps the
observation API and reverses the two decisions the row is about:

1. **Coalescing.** `crates/mc-edge/src/plan/viewer_plan.rs` (the commit's `plan.rs` `plan_viewer_inner`
   block, moved here by a later refactor): a/-side plans every in-range explosion as its own reliable packet
   carrying that recipient's impulse (the pre-fix shape: a 600-blast burst fills the 512-deep reliable FIFO
   and disconnects); b/-side is HEAD's single summed reliable carrier packet plus best-effort visuals.
   a/-side adaptation: pre-fix it passed `viewer.entity_id` to the packet builder; the kept `explode_packet`
   signature takes `Option<[f64; 3]>` (the kept refs call it through the production path and decode the
   packet), so the a/-side looks the recipient's impulse up and passes it. Same packet, same impulse.
2. **Lag detection.** `crates/mc-edge/src/server/pump.rs` `close_skipped_knockback`: the a/-side keeps the
   function and its join-baseline bookkeeping (the kept refs call it directly and the baseline belongs to
   `EDGE-JOIN-KNOCKBACK-BASELINE`) but seeds the pre-fix policy — a skipped knockback tick was not detected,
   so nobody is closed; b/-side is HEAD's cumulative-epoch comparison and explicit close.
3. The other two files carry their original non-test hunks: `crates/mc-edge/src/fanout.rs`'s traffic-class
   docs and `Priority::BestEffort` doc (comment-only reversal, no behaviour) and
   `crates/mc-edge/src/plan/tick_index.rs`'s `knockback` recipient index (removed in the a/-side; nothing in
   the pre-fix planning read it).

## Original hunks left out, with reasons

| Original hunk | Reason |
|---|---|
| `crates/mc-core/src/boundary.rs`: `WorldSnapshot::player_knockback_ticks` field | (b) the kept refs construct `WorldSnapshot { player_knockback_ticks: … , .. }` and read it; reversing the field stops the control from compiling. |
| `crates/mc-core/src/core/tick_loop.rs` and today's `crates/mc-core/src/core.rs` `observe_explosion_knockback` (epoch counting, `Arc` publication, `KnockbackPublication` write) | (a) another row's behaviour: `crates/mc-core/src/dimensions_tests.rs:43`, `:2980`, `:3012` and `:3019` call `observe_explosion_knockback` and assert `player_knockback_ticks` and its `Arc` sharing (`DIMENSION-SWITCH-KNOCKBACK-DISCONNECT`, `EDGE-JOIN-KNOCKBACK-BASELINE`); reversing it turns their assertions red while the row's own two refs build their epochs by hand. |
| `crates/mc-edge/src/server/pump.rs` doc comments around `spawn_pump`, the `Lagged` arm and `skip_publications` | (c) later commits rewrote those comment blocks into today's `skip_publications`/`spawn_pump` docs; no HEAD equivalent of the original comment text exists. |
| `docs/architecture/runtime.md`, `docs/decisions/0013-core-model-v2.md`, `docs/COMPATIBILITY.md` | documentation, not hunks. |

## Proving refs

- `cargo:mc-edge:server::admin_tests::explosion_queue_tests::six_hundred_explosions_coalesce_knockback_without_disconnect`
  — the control pushes 600 reliable explosion packets, overflows `RELIABLE_QUEUE_BOUND` (512), and the
  connection is closed: red at `assert!(server.connections.get(id).is_some())` (the reconnect/disconnect
  assertion, `explosion_queue_tests.rs:203`). The row's symptom is exactly a burst dropping knockback or
  closing the viewer.
- `cargo:mc-edge:server::admin_tests::explosion_queue_tests::broadcast_lag_closes_only_recipients_of_skipped_knockback`
  — with the a/-side policy the `lost` recipient's closer never fires: red at
  `explosion_queue_tests.rs:311` (`closers[0].try_recv().unwrap()` inside the `assert_eq!`; the row's
  lag-detection assertion).

Both are production-path (`publish_and_route`) or directly-called API assertions, not setup: no compile
break, no timeout, no missing packet decode.
