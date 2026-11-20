# WAL-BATCH-HAS-NO-STORE-IDENTITY — negative-control patch

Fix: `docs/bugs/fix-patches/WAL-BATCH-HAS-NO-STORE-IDENTITY.patch`
(`fix_commit = "patch:<sha256 of that file>"`).

## Original commits

- `b23a43d60`, `1e29ed137` — WAL frame v2: the frame carries `store` (16 bytes)
  and `palette` (8 bytes); the reader parses both. Not reversed here: reversing
  the frame format would remove the field the kept tests write and read, i.e.
  the control would not compile.
- `ab8972dc8` — the refusal itself.

## Hunk mapping

| Patch hunk | HEAD `file:line` | What it is |
|---|---|---|
| the only hunk | `crates/mc-core/src/journal.rs:1032-1039` | the `if store != identity.store { … }` refusal inside `verify_bindings` |

`git apply -R` on the candidate deletes exactly that block; `verify_bindings`
still compares the palette, so the reversed tree compiles and the palette
refusal still works.

## Exclusions

- The palette refusal (`crates/mc-core/src/journal.rs:1040-1047`) belongs to
  `WAL-BLOCK-STATE-IDS-HAVE-NO-PALETTE-IDENTITY`; reversing it here would turn a
  second row's assertion red.
- The frame format, `WalIdentity`, `identity_hex` and the writer plumbing are API
  the kept tests call (reason (b) of the ledger contract).
- The doc comment above `verify_bindings` is shared by both refusals and is not
  behaviour; it is left in place.

## a/-side adaptation

None.

## Proving refs

- `cargo:mc-core:journal_tests::recovery_refuses_a_journal_copied_from_another_world` —
  red at `crates/mc-core/src/journal_tests.rs:2214` (`.expect("a foreign tail is
  never replayed")`) and at `:2218` (`text.contains("<source store uuid>")`): the
  row's own symptom, a tail of another world being replayed.
- `cargo:mc-core:journal_tests::recovery_refuses_a_batch_whose_store_id_is_zero` —
  red at `crates/mc-core/src/journal_tests.rs:2301`
  (`.expect("a zero store id is not a wildcard")`) and `:2303`
  (`text.contains("belongs to store")`): the zero-id wildcard the review found.
  With the wildcard present this assertion fails because no refusal is produced.

Both refs are registered on the row; a control is valid when either is red.
