# DATAPACK-HOT-RELOAD-RECIPES-LOOT: patch-mode fix identity

Original fix: `32069cbb9` (`feat(core,edge): datapack list/enable/disable/create/zip/hot reload, …`) —
before it, a reload whose pack set changed recipes or loot was refused with
`Recipe/loot changes require restart; reload refused, old packs and selection retained`
(`crates/mc-core/src/datapacks.rs` at `32069cbb9^`, the `tick()` completion match). The fix
recompiles the changed resources (`PackSet::changes_registry` → `install_content`) and installs
them: `install_registry` publishes the new registry in every world, `adopt_registry` adopts it and
pushes `RecipesReloaded` plus the player's own recipe-book reload to every connection. The
worker/job architecture those functions live in was rewritten by later commits, so the commit no
longer reverses at HEAD; the row is a scoped patch:
`fix_commit = "patch:40b2cc0302b514d50d9b1afc1afd8dd1e671adde67297431b8a90ec4f163d671"` in
`docs/bugs/promotions/DATAPACK-HOT-RELOAD-RECIPES-LOOT.patch`. Re-audit:
`python3 -m tools.harness ledger promote DATAPACK-HOT-RELOAD-RECIPES-LOOT --fix-patch docs/bugs/promotions/DATAPACK-HOT-RELOAD-RECIPES-LOOT.patch --dry-run`.

## Carried

- `datapacks.rs` `finish()` — the `(Job::Reload | Job::Activation { .. }, Completed::Packs(..))`
  branch: the HEAD equivalent of the pre-fix decision. Its a/-side reinstates the pre-image
  comparison (`packs.changes_registry(&state.datapacks.packs)`, the HEAD name of the pre-fix
  `packs.resources != state.datapacks.packs.resources`), the pre-image refusal message verbatim,
  and the `(false, 0)` short-circuit before `install_registry`. Reversed, a reload whose pack set
  changes recipes or loot installs nothing and keeps the old registry and selection.

## a/-side adaptation

The pre-image's refusal sat in the old `tick()` completion match, which the worker architecture
replaced. The only text outside the original pre-image is the guard's own skeleton
(`if … { if deliver { feedback(…) } return (false, 0); }`) around the pre-image comparison and
message; it changes no behaviour beyond the refusal itself. Class (c) rewrite, documented here per
the README's a/-side rule.

## Not carried

- The rest of `32069cbb9` (datapack list/enable/disable/create/zip command surface, the debug
  profiler, `data entity` edits, selector predicates, client chat visibility) — (a) other rows' and
  other lanes' behaviour.
- `install_content`, `prepare_packs` and the worker body — (c) they are the fix's own machinery and
  are needed by both trees; the observable outcome the row names is the install/adoption, which the
  carried hunk gates.
- Test files — (b) kept so the control compiles and runs the refs.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:datapacks::command_tests::enabling_and_disabling_a_pack_installs_its_recipes_and_loot_and_resyncs_clients`
  — with the patch reversed an enable whose pack adds a recipe answers the pre-fix refusal, the
  registry is never adopted and the ref fails at its own
  `assert!(Arc::ptr_eq(&published.read().unwrap(), &state.registry), "the registry joiners see the
  reloaded one")` (`crates/mc-core/src/datapack_commands_tests.rs:923`); its next assertion `:927`
  (`a recipe the pack adds`) and `:949` (the `RecipesReloaded` announcement) fail for the same
  reason.
- `cargo:mc-core:datapacks::command_tests::a_reload_with_an_unusable_recipe_keeps_the_old_registry_and_selection`
  — the refusal half; green in both trees by design and kept as the fence.
- `harness:datapack-reload:native-datapack-reload` — a real client's recipe book shows the pack's
  recipe after a hot enable and vanilla's after a hot disable. Passed receipt
  `.analysis/validation/20261124T125619-native-datapack-reload-a1eyl7_g` (candidate).
