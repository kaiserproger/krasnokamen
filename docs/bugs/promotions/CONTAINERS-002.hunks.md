# CONTAINERS-002: patch-mode fix identity

Original fix: `2ccf8cc1a` (`fix(core): restore mount lectern and command container custody`) — it created
`crates/mc-core/src/lectern.rs` and wired it into three decision points: the block use dispatch
(`tick/input/interact.rs`), the menu-kind table (`container_kind_for_state`, then in `tick.rs`, now
`tick/containers.rs` after `db1dc0403` split the god files) and the container-button dispatch
(`tick/input/inventory.rs`, which owns page turning and the take-book button).

The whole commit does not reverse on HEAD, and the module itself cannot be reversed at all
(`tick.rs` keeps a `lecterns` field, `hydrate_chunk` and the bake path that reference it, so deleting the
module stops the control from compiling). The row therefore moves to a scoped patch:
`fix_commit = "patch:d769f8ec9dad1aa2b956a395b8a4fa01e0a5951066e50b80e6fc719f53a4eb1c"`, the HEAD
equivalents of the three dispatch/decision hunks, in
`docs/bugs/promotions/CONTAINERS-002.patch`. Reversing it leaves `lectern.rs` in place but unreachable, so
a held book is neither stored nor does it open the menu: the row's symptom. Re-audit:
`python3 -m tools.harness ledger promote CONTAINERS-002 --fix-patch docs/bugs/promotions/CONTAINERS-002.patch --dry-run`.

Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's
or owner's behaviour (a), API the kept tests call (b), or code a later commit rewrote (c).

## Carried

- `tick.rs @@ -4194,9 +4204,11 @@ fn container_kind_for_state` — the added
  `"minecraft:lectern" => Some((MenuKind::Lectern, "Lectern".into(), 1)),` line. The hunk also changed the
  crafting table's backing size, which is CONTAINERS-003's half: this patch carries only the lectern line
  (an (a)-split of a mixed hunk; the surviving half is the real pre-image line order). HEAD equivalent:
  `crates/mc-core/src/tick/containers.rs:469`.
- `tick/input/interact.rs @@ -181,6 +181,9 @@ pub(in crate::tick) fn use_item_on` — the
  `crate::lectern::use_on(...)` dispatch term. HEAD: `crates/mc-core/src/tick/input/interact.rs:201-203`.
- `tick/input/inventory.rs @@ -420,6 +426,9 @@ fn container_button` — the `crate::lectern::button(...)`
  dispatch term. HEAD: `crates/mc-core/src/tick/input/inventory.rs:512-514`.

## Original non-test hunks of `2ccf8cc1a` (complete list)

The full per-file hunk list is in `CONTAINERS-003.hunks.md` ("Original non-test hunks"); the same commit
is the original fix for both rows.

## Not carried

- `crates/mc-core/src/lectern.rs` (new, 342 lines) — (b) kept API: `TickState.lecterns`,
  `sync_chunk_block_entities` and the chunk bake path call into it, so the reversed tree would not build.
  Its callers are all among the carried hunks, which is the accepted kept-helper class.
- `tick.rs` hunks for the `lecterns` field, `hydrate_chunk`, the removal hook and the merge path — (b) the
  same kept API; (a) the rest of the hunk belongs to command-block custody.
- `tick_tests.rs`, `books_tests.rs` — (b) test files kept so the control compiles and runs the ref.
- Everything else in the commit (mounts, command blocks, redstone, world, edge, protocol) — (a) other
  rows' behaviour, each carried by its own row's control, and the complete list is in
  `CONTAINERS-003.hunks.md`.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:tick::book_actions::tests::lectern_insert_read_turn_take_preserves_exact_book` — with the
  use dispatch reversed the book is never moved onto the lectern, so the ref fails at its own
  `assert!(state.players[&actor].inventory.slots[0].is_empty())` (`crates/mc-core/src/books_tests.rs:171`)
  and, without the fix at all, at `expect("lectern opens")` (`:173`); `tick/book_actions.rs:79` declares
  the module.
- `harness:entities:native-v2-ledger-lectern` — the real client inserts a written book, receives
  `LecternMenu` with one slot holding the book and takes it back
  (`tools/harness/backends/native_ledger_lectern.py`). Passed receipt: see the promotion JSON.
