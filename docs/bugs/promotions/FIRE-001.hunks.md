# FIRE-001: patch-mode fix identity

Original fix: `35308579c` (`fix(interact): rebase onto 7fe49052 — axe strip/scrape/unwax, sponge,
brushing, painting, spawn protection, dragon egg, special storage`) — its one `fire.rs` hunk widened
`eternal()` from `netherrack` to `netherrack | magma_block`, plus End `bedrock`. That hunk reverse-applies
verbatim at HEAD because later commits did not touch `eternal()`. `fix_commit =
"patch:0cda3ab729ec0178…"` names it in `docs/bugs/promotions/FIRE-001.patch` (also kept at
`docs/bugs/fix-patches/FIRE-001.patch`); `git apply -R` restores the pre-fix code. Re-audit:
`python3 -m tools.harness ledger promote FIRE-001 --fix-patch docs/bugs/promotions/FIRE-001.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Original non-test hunks (complete)

```
## 35308579c: only this commit's fire.rs hunk belongs to FIRE-001
- crates/mc-core/src/fire.rs: -174,7 +174,11 (eternal(): netherrack → netherrack | magma_block + End bedrock)
```

## Carried

- `fire.rs @@ -174,7 +174,11 @@` — verbatim at HEAD `crates/mc-core/src/fire.rs:174-185`
  (`fn eternal`): `matches!(block.block.id.path(), "netherrack" | "magma_block")` and the
  `Dimension::End && bedrock` disjunct. Reversing it restores the single `netherrack` predicate (a/-side
  is the prior code of `35308579c^`, not an invented line).

## Not carried

- Every other hunk of `35308579c` (interact.rs, brushing.rs, painting, spawn protection, dragon egg,
  special storage, sponge, chest/lectern custody, edge/config plumbing) — (a) other rows of the same
  rebase; each has its own row/control. They are not fire behaviour.
- Test files of that commit — (b) kept in the candidate; reversing test wiring would stop the control
  from compiling.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:interact_parity_tests::interact_parity_eternal_fire_magma_and_end_bedrock`
  (`crates/mc-core/src/interact_parity_tests.rs:110`) — with the patch reversed the eternal check only
  accepts `netherrack`, so for the `magma_block` support case
  `crates/mc-core/src/interact_parity_tests.rs:132-135`
  `assert_ne!(state.world.block_at(POS.x, POS.y, POS.z), Some(state.registry.states.air))` fails: the
  age-15 fire is not infinite and expires to air.
- Negative control observed 2026-11-23: `cargo:…interact_parity_tests::interact_parity_eternal_fire_magma_and_end_bedrock=red`;
  promotion rests on that assertion. The registered harness ref
  `harness:entities:native-v2-ledger-infiniburn` was `unrunnable` in that promotion (the reverted tree had
  no pin-release server); its green client receipt on the candidate tree is
  `.analysis/validation/20261123T192355-native-v2-ledger-infiniburn-d8azx4af/result.json`.
