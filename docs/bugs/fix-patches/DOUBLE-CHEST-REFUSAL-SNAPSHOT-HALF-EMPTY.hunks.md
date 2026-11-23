# DOUBLE-CHEST-REFUSAL-SNAPSHOT-HALF-EMPTY — patch-mode fix identity (new fix)

Fix: `docs/bugs/fix-patches/DOUBLE-CHEST-REFUSAL-SNAPSHOT-HALF-EMPTY.patch`
(`fix_commit = "patch:4b2427db26a3655cf61a61d70d0da199d367934cc6c193419d95588928724c90"`).
Re-audit:
`python3 -m tools.harness ledger promote DOUBLE-CHEST-REFUSAL-SNAPSHOT-HALF-EMPTY --fix-patch docs/bugs/fix-patches/DOUBLE-CHEST-REFUSAL-SNAPSHOT-HALF-EMPTY.patch --dry-run`

## Why patch mode

The fix is uncommitted work in this lane (`696d843631` swept the body in with a
docs commit, then `1e5f305f2e` fixed the regression's assertion). The
signature change (`&mut TickState`) is spread over four call sites and one
kept test; reversing the commit would restore the `&TickState` signature and
stop the control from building against the kept test. The identity therefore
carries the body change only.

## What the patch carries

`crates/mc-core/src/tick/containers.rs`, `inventory_transition_refused`: the
cold-partner check (`chests::partner_position` at the open menu's block, then
`ChunkWorld::chunk` for the partner half) and, for a cold partner, the refusal
by `close_player_menu` returning empty effects instead of resending the menu
image. This is the row's whole path: the pre-fix body rebuilds `container_cells`
from whatever half is readable and sends 27 real slots plus 27 empty ones.

## a/-side adaptation

The kept test calls `inventory_transition_refused(&mut state, player)`, so the
`&mut TickState` parameter is kept on the a/-side (reason (b): restoring the
pre-image's `&TickState` stops the control from building). The body is the
pre-fix resync verbatim in behaviour (`players.get` then
`inventory_transition_refused(&slots)`), with the same names. The call sites
are unchanged on both sides, so nothing else outside the original pre-image is
constructed.

## Proving ref (crates/mc-core/src/chests_tests.rs)

- `integrity_reserved_refusal_with_a_cold_partner_closes_the_menu` — a
  54-slot double chest whose left half sits at `x = 15` of its chunk; only the
  right half's chunk is evicted (`finish_inhabited_tick`, empty active set,
  `capture_dirty`, `mark_flushed`, `evict_outside`). The refusal must leave
  `player.open == None`, the container set empty, the readable half's diamond
  in place, and a `MenuClosed` for the opened window in the player's outbox.
  In the control the menu stays open (the pre-fix resync) and the test fails at
  its own `"the cold-partner refusal closes the menu"` assertion.
