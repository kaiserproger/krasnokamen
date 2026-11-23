# DOUBLE-CHEST-REFUSAL-SNAPSHOT-HALF-EMPTY — patch-mode fix identity (third form)

Fix: `docs/bugs/fix-patches/DOUBLE-CHEST-REFUSAL-SNAPSHOT-HALF-EMPTY.patch`
(`fix_commit = "patch:83e6126e934477f5f90d63c76e0d2884cae009765d944df319fa4b436846f843"`).
Re-audit:
`python3 -m tools.harness ledger promote DOUBLE-CHEST-REFUSAL-SNAPSHOT-HALF-EMPTY --fix-patch docs/bugs/fix-patches/DOUBLE-CHEST-REFUSAL-SNAPSHOT-HALF-EMPTY.patch --dry-run`

## Why patch mode

The fix is one function in `crates/mc-core/src/tick/containers.rs`, committed
as `96fca8717b` "fix(core): close a menu whose custody spans a cold chunk". The
patch is the whole function: pre-image verbatim on the a/-side, the candidate
form on the b/-side.

## What the patch carries

`inventory_transition_refused` in `crates/mc-core/src/tick/containers.rs`:

- the vehicle and ender returns come first (unchanged on both sides);
- the block-menu branch reconstructs the menu's cells and slots, then applies
  the same custody check the click (`tick/input/inventory.rs`) and recipe
  (`place_recipe`) paths make: `menu.kind.container_size() != container.len()`
  or any named block non-resident (`ChunkWorld::block_at` is `None`);
- a cold half — the open half, the partner, or both — closes the menu through
  `close_player_menu` and returns a window-0 resync
  (`inventory_transition_refused(&[])`, which resolves against
  `inventory_menu` once `open` is cleared) so the client's predicted stack,
  cursor or book edit rolls back;
- otherwise the real storage is resent.

`chests::partner_position` is deleted: the cold test does not need the
partner's coordinate (that helper was the second convention the review
rejected).

## a/-side

None. The a/-side is the pre-fix function text exactly as it stood in
`ac8c352a3` (`state: &TickState`, `player.inventory_transition_refused(...)`
for vehicles and the tail): `&mut` callers coerce to `&`, so neither the
call sites nor the kept tests need an adaptation. No text outside the original
pre-image is constructed.

## Proving refs (crates/mc-core/src/chests_tests.rs)

All three share `reserved_refusal_with_a_cold_half`, which puts the halves on
opposite sides of a chunk boundary, opens the selected half, sets a cursor
stack, evicts the named halves (`finish_inhabited_tick`, empty active set,
`capture_dirty`, `mark_flushed`, `evict_outside`) and calls the refusal.

- `integrity_reserved_refusal_with_a_cold_partner_closes_the_menu` — open
  left half, partner cold. Asserts: no `MenuContents` for the open window
  (never a 27+27 image), a `MenuClosed` in the player outbox, a window-0
  resync, `open` cleared, the readable half's diamond intact, and the cursor
  emerald in the player's inventory.
- `integrity_reserved_refusal_with_a_cold_open_half_closes_the_menu` — the
  menu is anchored on the half whose chunk is cold; the partner is resident.
  This is the case the earlier identity missed (the partner-position check
  returned `None`).
- `integrity_reserved_refusal_with_both_halves_cold_closes_the_menu` — both
  chunks evicted.

In the control all three fail at their own assertions: the pre-fix body
resends the half-empty menu and leaves `open` set. The red is
`chests_tests.rs:706:5` (the first `MenuContents`-for-the-closed-window
assertion) for all three refs; the promote receipt is
`.analysis/validation/20261123T223848-ledger-promote-2rc43auc`
(`ledger-1-run.log:335`, `:354`, `:373`), restore green in the same receipt.

## Fix in several commits

| Commit | Closes |
|---|---|
| `696d843631` | the first (rejected) cold-partner form of the body, inside a docs-subject commit |
| `c77677ec95` | its clippy fix (pass the slots slice directly) |
| `96fca8717b` | the accepted form: the click/recipe custody check, the cold-open-half and both-cold cases, the window-0 resync, `partner_position` deleted |

This identity is the accepted form and supersedes the
`patch:6af67cbd`/`patch:4b2427db` receipts, which are deleted. History is not
rewritten.
