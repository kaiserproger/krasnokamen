# CORE-DROP-LEAVES-WORLD-DIR-WRITER: patch-mode fix identity

`fix_commit = "patch:ac21dca494074f48dbc8ef64828f6745734d9989b4cbafbfd253601bd6758bb6"`
(`docs/bugs/fix-patches/CORE-DROP-LEAVES-WORLD-DIR-WRITER.patch`; byte-identical to
the sibling row's `CORE-DROP-RELEASES-LEASE-BEFORE-OWNERS-JOIN.patch`, so the
digest matches).

## This row is a duplicate, closed by the earlier guard

The reported flake is CI run 43340344269 on main `3caeae5c`, **2026-11-13**
(`mc-edge advancements::tests::reloaded_recipes_are_announced_once_and_feed_the_next_book`
panicking on `remove_dir_all` with `DirectoryNotEmpty` right after `drop(core)`).
The guard is newer:

- `84116c517` "fix(core): join owners before releasing world lease" (2026-11-22)
  added `CoreInner::stop_owners` and `impl Drop for CoreInner`
  (`crates/mc-core/src/core.rs:1817-1853`), so a plain drop signals the owners,
  joins the tick and then the journal, and only then releases the world lease.
- The sibling row CORE-DROP-RELEASES-LEASE-BEFORE-OWNERS-JOIN was filed from the
  same class and says in its own `source`: *"Possibly the cause of
  CORE-DROP-LEAVES-WORLD-DIR-WRITER; not yet reproduced."* Its promotion proved
  the same patch with `plain_drop_holds_the_world_lease_until_the_publication_commits`.

So the leftover writer that recreated a file after `drop` returned is the
detached tick/journal owner of the pre-`84116c517` code; this row is that
duplicate and keeps `repro = "cargo"`.

## The patch

One file, `crates/mc-core/src/core.rs`: `Core::shutdown` delegates to the new
`CoreInner::stop_owners`, and `impl CoreInner` + `impl Drop for CoreInner` are
added. The stop order text is unchanged; only its owner moved so a destructor can
run it. It is the guard at HEAD, so its `a/`-side is the pre-guard code and it
reverse-applies on the candidate (`git apply -R --check` rc 0). Test wiring
(`OwnerAliveGuard`, the `#[cfg(test)] owners` field/probes, the test module
registration) is not in the patch: it is the reproducer's API, category (b).

## Proving assertion

`cargo:mc-core:core::core_drop_join_tests::plain_drop_joins_a_journal_writer_holding_the_world_directory`
— the row's own ref, which parks the real journal owner on its exit path
(`JOURNAL_EXIT_HOLD`) and drops the Core on another thread. The test was
restructured for this promotion so the row's loss is reported first and
deterministically: a joined drop cannot return before the hold is released, so a
bounded 2 s window firing is the defect, and the child fails with

```
the plain drop returned while the journal owner was still alive: a writer was left in the world directory
```

at `crates/mc-core/src/core/core_drop_join_tests.rs:392` (named on the row in
`proving_assertions`; the ledger classifies the explicit panic as `unknown`
unless named). The parent re-exec then fails at its
`assert!(child.status.success(), …)` at `:359`. Reversed (no `Drop`), the drop
detaches the owners and the test reds exactly there
(`.analysis/validation/20261125T232421-ledger-promote-z8n4oo47/ledger-1-run.log:573,587-588`).
On the candidate the test passes in 3.5 s
(`.analysis/validation/20261125T205602-test-y1qfp8ae`) and green in the restore.

## Reproduction statistics, honestly

The 2026-11-26 loop (`receipts/D123GameplayP/row3-repro/`, tree `3ee7ab8c60`)
produced 0 failures in 100 four-way runs of the exact CI test. At the original
≈1% four-way rate that is `0.99^100 ≈ 0.37`, so it is **not** evidence of
absence; the 100 serial runs are not comparable (the original never failed
serially). The closure rests on the guard's identity and the deterministic
join test, not on the loop.
