# DEATH-PROTECTION-COMPONENT: patch-mode fix identity

Original fix: `de780c882` (`fix(core): honor held death protection components`) — the lethal branch of
`TickState::commit_damage` now asks `totem::rescue`, which honours any held stack carrying the
`DEATH_PROTECTION` component (not only the default totem's effects). Its own diff no longer
reverse-applies after later rewrites of the same block, and a control tree at `fix_commit` fails to load
the current registry (the rejection's red was a fixture panic, not an assertion). The patch is the HEAD
equivalent of the fix's behaviour: the root `died |= player.vitals.hurt(...)` form in `tick/damage.rs`.
`fix_commit = "patch:11f2526265bfb37f…"` names it in
`docs/bugs/promotions/DEATH-PROTECTION-COMPONENT.patch` (a diff from the pre-fix form to the candidate;
`git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote DEATH-PROTECTION-COMPONENT --fix-patch docs/bugs/promotions/DEATH-PROTECTION-COMPONENT.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `tick/damage.rs @@ -256,7 +260,8 @@`, `@@ -272,9 +277,16 @@` and the rescue effect on
  `health_changed`/`damage` — the HEAD equivalent is `crates/mc-core/src/tick/damage.rs:660-675`
  (`let lethal = ...`, the `if lethal { rescue = totem::rescue(...); ... }` block, and the two derived
  lines). Reversing them restores the pre-fix lethal handling; the a/-side is prior code.
- `tick/damage.rs @@ -550,7 +550,7 @@` — **compile-support a/-side line**: `let mut rescue = None` →
  `let mut rescue: Option<totem::Rescue> = None`. The later `b7020b427` restructure added the kept
  application block (`crates/mc-core/src/tick/damage.rs:693-720`) whose `rescue.used` leaves the local
  untyped once the assignment is reversed (E0282, observed 2026-11-25 in the control tree). Same local,
  same initialiser, annotation only; the b/+ side is the untyped declaration.
- Sibling note: TOTEM carries the identical patch body because both rows come from `de780c882`'s one
  decision point; each control reds its own test (default totem vs held component) — see the proving refs.

## Original non-test hunks (complete)

```
## de780c882 non-test hunks (kept test files: command_items_tests.rs, tick/combat_parity_tests.rs, copied_components_tests.rs)
- crates/mc-core/src/survival.rs: -183,6 +183,10 (bypasses_invulnerability)
- crates/mc-core/src/tick/damage.rs: -5,6 +5,9; -152,6 +155,7; -256,7 +260,8; -272,9 +277,16; -295,6 +307,32
- crates/mc-core/src/tick/totem.rs: -0,0 +1,79
- crates/mc-data/src/data_component_types_26_1_2.rs: -20,6 +20,7
- crates/mc-data/src/item_component_values.rs: -303,6 +303,7
- crates/mc-data/src/item_stack.rs: -21,6 +21,22
- crates/mc-protocol/src/packets/play/copied_components.rs: -179,6 +179,12; -342,6 +348,14
```

## Not carried

- `tick/totem.rs` and its `#[path] mod` line, `survival.rs::bypasses_invulnerability`, the mc-data
  component declarations and the mc-protocol copied-component hunks — (b) API the kept tests call: the
  test authors `ItemArgument::parse("{item}[death_protection={death_effects:[]}]")` and asserts the
  rescue/consumption, so the component parsing and the totem module must compile in the control. The
  component's *presence* is therefore not reversed; the rescue's decision point is, which is where this
  row's behaviour lives (a held `stick` with the component must rescue — without the decision no item
  can).
- `tick/damage.rs @@ -152,6/-295,6` — (b) the `totems` vector/apply wiring, inert once `rescue` is never
  assigned.
- `docs/ARCHITECTURE.md` — (b) docs; test files — (b) kept.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:tick_tests::combat_parity_tests::death_protection_component_rescues_any_held_item_without_default_totem_effects`
  (`crates/mc-core/src/tick/combat_parity_tests.rs:565`) — with the rescue gone the lethal fall hit kills
  the player, so `crates/mc-core/src/tick/combat_parity_tests.rs:582`
  `assert!(deaths.is_empty(), "{item} in slot {hand}")` fails on the first `stick` case (a held `stick`
  with the component can no longer rescue), and `:589-590` (health 1.0, item consumed by one) never
  arrive.
- Client ref `harness:entities:native-v2-p4-death-protection` green receipt on the pinned candidate tree
  (`.analysis/validation/20261123T222939-native-v2-p4-death-protection-t2oyivrz/result.json`): a held item
  with a custom `death_protection` component survives a lethal hit with its own effect
  (`custom_death_protection_prevents_death`, `custom_death_protection_effects_received`,
  `no_generic_totem_effects`), then the initial effect clears.
