# DATAPACK-HOT-RELOAD-RECIPES-LOOT: patch-mode fix identity

The row says runtime recipe/loot changes are refused instead of installed. The fix is the install path
(`prepare_packs` → `install_registry` → `adopt_registry`, all in `crates/mc-core/src/datapacks.rs`), which landed
with the worker/job split in `32069cbb9` and was extended by its follow-ups; the commit cannot be reversed on
HEAD because the file was rewritten around it.

The **real pre-image** is the refusal that split replaced, `32069cbb9^:crates/mc-core/src/datapacks.rs`
(`tick`, the reload-completion match):

```rust
Ok(packs) if packs.resources == state.datapacks.packs.resources => { …install… }
Ok(_) => feedback(
    state,
    connection,
    "Recipe/loot changes require restart; reload refused, old packs and selection retained",
),
```

HEAD moved that decision into `finish(state, reload, result)`, which now receives
`Completed::Packs(packs, registry)` and installs unconditionally. `PackSet.resources` is still the same private
field (`datapacks.rs:149`), so the control is a class-(b) port with the pre-image's **condition verbatim**
(`packs.resources != state.datapacks.packs.resources`) and its **message verbatim**, placed at the install site:

```rust
(Job::Reload | Job::Activation { .. }, Completed::Packs(packs, registry)) => {
    if packs.resources != state.datapacks.packs.resources { … feedback(…); return (false, 0); }
    if let Some(registry) = registry { install_registry(state, registry); }
```

`fix_commit = "patch:de4e8fe97693f20cf9f7a6fdcc53046fb4c84507298ab88bfc3080fd458bc2ab"`,
`docs/bugs/promotions/DATAPACK-HOT-RELOAD-RECIPES-LOOT.patch`. `git apply --check -R` passes on the candidate.

## Why the first attempt was rejected

`f2b291c99`'s patch (REVIEW-wave2-3 B1) inserted a *ten-line* guard that paraphrased the message and compared a
hand-built resource list; README:234-258 refuses an invented guard. This rewrite keeps the pre-image's own
condition and text and nothing else.

## Proving refs

- `harness:datapack-reload:native-datapack-reload` — the candidate's green (receipt in the promotion JSON). With
  the patch reversed the same gate should lose `pack_recipe_after_hot_enable` and the loot/disable halves;
  **[INFERENCE]** — the control records the harness ref as `unrunnable` (no reverted-pin client build), so its red
  was not observed.
- `cargo:mc-core:datapacks::command_tests::enabling_and_disabling_a_pack_installs_its_recipes_and_loot_and_resyncs_clients`
  — the controlled red: with the refusal restored the registry keeps the old recipe, so the test's own
  `assert_eq!` on the installed recipe fails at `crates/mc-core/src/datapack_commands_tests.rs:927` (the `Arc::ptr_eq`
  check at `:923` stays green on the control, because the registry is simply never replaced; `:1478` also reds).
  Two of the row's refs red, the rest stay green.
- The control cannot run the graphical half (no reverted-pin client build), so the red is the cargo ref — the same
  POL-1 shape HS-2 uses.
