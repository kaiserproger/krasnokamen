# POTION-CUSTOM-PAYLOAD: patch-mode fix identity

Original fix: `bafcdb6f0` (`fix(core): … throwable geometry, eggs, pearls, potions, water impact`) —
its `projectiles/payloads.rs` hunks added the custom-payload path: `custom_effects()` reads the
wire-retained `potion_custom_effects` component, `potion_color()` returns the authored
`potion_custom_color` before mixing, and both splash and cloud application chain the custom
effects after the base potion's. `b46ace161` later rewrote the cloud code
(`deliver_effect`/`apply_potion`), so the commit no longer reverses at HEAD; the row is a scoped
patch: `fix_commit = "patch:1e2dc4deb410ac400791a101caa4ab5741f24d9ba37aac297894d87dd1369691"` in
`docs/bugs/promotions/POTION-CUSTOM-PAYLOAD.patch`. Each a/-side is the pre-image statement as it
stood at `bafcdb6f0^`: `potion_color`'s single `for effect in potion_effects(stack) {` loop header
and `apply_potion`'s `for mut effect in potion_effects(&record.stack) {`, plus the absence of the
new helper. Re-audit:
`python3 -m tools.harness ledger promote POTION-CUSTOM-PAYLOAD --fix-patch docs/bugs/promotions/POTION-CUSTOM-PAYLOAD.patch --dry-run`.

## Carried

- `payloads.rs` — the `custom_effects()` helper (new in the fix).
- `payloads.rs` `potion_color()` — the authored `potion_custom_color` early return and the
  `.chain(custom_effects(stack))` in the base-effect loop.
- `payloads.rs` `apply_potion()` — the `.chain(custom_effects(&record.stack))` in the effect loop,
  which is the HEAD shape of the pre-fix `for mut effect in potion_effects(&record.stack) {`.

## Not carried

- `is_water_bottle` / `water_impact` (the same commit) — (a) the water-bottle impact path is not
  this row's symptom; reversing it would turn the row's splash-water refs red instead.
- `targets()` AABB inflation and `zero_damage_reaction` (same commit) — (a) throwable geometry and
  projectile contact belong to other rows.
- `pearl()` endermite, `hatch()` egg variants, `mob_immune_to_effect` (same commit) — (a) other rows.
- `cloud_harmful` per-effect PvP gating as it stands at HEAD — (c) `b46ace161` rewrote it from
  `potion_effects().chain(custom_effects()).any(…)` into the per-effect function the cloud loop
  calls; the custom effects now reach the same gate through `apply_potion`, which this patch carries.
- Test files — (b) kept so the control compiles and runs the refs.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:projectiles::payloads::tests::lingering_custom_payload_and_impact_color_are_preserved`
  — with the patch reversed the cloud applies only the base potion (the fixture has none), so the
  ref fails at its own `active_effects.get(EffectId::Speed).is_some()` assertion
  (`crates/mc-core/src/projectiles/payloads_tests.rs:2252`), and the level-event assertion above it
  (`ProjectileLevelEvent { event: 2002, data: 0x123456 }`, `:2221`) fails for the colour half.
- `harness:entities:native-v2-ledger-potion-custom` — a real client throws a lingering potion whose
  only payload is one authored custom levitation effect and reads its own lifted position
  (`tools/harness/backends/native_ledger_potion_custom.py`). Passed receipt: see the promotion JSON.
