# WAL-BLOCK-STATE-IDS-HAVE-NO-PALETTE-IDENTITY — negative-control patch

Fix: `docs/bugs/fix-patches/WAL-BLOCK-STATE-IDS-HAVE-NO-PALETTE-IDENTITY.patch`
(`fix_commit = "patch:<sha256 of that file>"`).

## Original commits

- `b23a43d60`, `1e29ed137` — WAL frame v2 carries the 8-byte palette digest and
  the reader parses it. Not reversed here: the kept tests write and read that
  field, so reversing the format stops the control from compiling.
- `ab8972dc8` — the refusal itself.

## Hunk mapping

| Patch hunk | HEAD `file:line` | What it is |
|---|---|---|
| the only hunk | `crates/mc-core/src/journal.rs:1040-1047` | the `if palette != identity.palette { … }` refusal inside `verify_bindings` |

`git apply -R` deletes exactly that block; the store refusal that precedes it
stays, so the reversed tree compiles and keeps the store binding.

## Exclusions

- The store refusal (`crates/mc-core/src/journal.rs:1032-1039`) is
  `WAL-BATCH-HAS-NO-STORE-IDENTITY`'s behaviour (reason (a)).
- `block.rs::palette_digest`, `WalIdentity` and the writer plumbing are API the
  kept tests call (reason (b)).

## a/-side adaptation

None.

## Proving refs

- `cargo:mc-core:journal_tests::recovery_refuses_a_tail_written_under_another_block_palette` —
  red at `crates/mc-core/src/journal_tests.rs:2251` (`.expect("a foreign palette
  is never replayed")`) and `:2253` (`text.contains("block palette")`).
- `cargo:mc-core:journal_tests::recovery_refuses_a_batch_whose_palette_digest_is_zero` —
  red at `crates/mc-core/src/journal_tests.rs:2334`
  (`.expect("a zero palette digest is not a wildcard")`) and `:2336`
  (`text.contains("block palette")`): the zero-digest wildcard.

Both refs are registered on the row; a control is valid when either is red.
