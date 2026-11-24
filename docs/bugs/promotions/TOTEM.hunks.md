# TOTEM: patch-mode fix identity

Original fix: `de780c882` (`fix(core): honor held death protection components`) — it turned the lethal
branch of `TickState::commit_damage` into the totem rescue (`totem::rescue` + `totem::apply`), added
`crates/mc-core/src/tick/totem.rs`, wired the `DEATH_PROTECTION` item component, and exposed
`DamageKind::bypasses_invulnerability`. Its own diff no longer reverse-applies after later rewrites of the
same block, and a control tree at `fix_commit` fails to load the current registry (the rejection's red
was a fixture panic, not an assertion). The patch is the HEAD equivalent of the fix's behaviour: the
root `died |= player.vitals.hurt(...)` form in `tick/damage.rs`. `fix_commit =
"patch:11f2526265bfb37f…"` names it in `docs/bugs/promotions/TOTEM.patch` (a diff from the pre-fix form to
the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote TOTEM --fix-patch docs/bugs/promotions/TOTEM.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `tick/damage.rs @@ -256,7 +260,8 @@`, `@@ -272,9 +277,16 @@` and the `totems` loop's effect on
  `health_changed`/`damage` — the HEAD equivalent is `crates/mc-core/src/tick/damage.rs:660-675`:
  reversing `let lethal = ...; let health_after_hit = ...` back to `died |= player.vitals.hurt(...)`,
  removing the `if lethal { rescue = totem::rescue(...); if rescue.is_none() { died = true; } }` block and
  restoring `health_changed |= player.vitals.health != before` / `... - player.vitals.health ...`. The
  a/-side is that prior code (from `de780c882^`), not an invented stub.
- `tick/damage.rs @@ -550,7 +550,7 @@` — **compile-support a/-side line**: the declaration ten lines
  before the hurt call, `let mut rescue = None` → `let mut rescue: Option<totem::Rescue> = None` in the
  a/-side. The later `b7020b427` restructure added the kept `if let Some(mut rescue) = rescue { … }`
  application block (`crates/mc-core/src/tick/damage.rs:693-720`), whose `rescue.used` leaves `rescue`
  otherwise untyped once the assignment is reversed (E0282, observed 2026-11-24 in the control tree). The
  a/-side line is the same local, same initialiser, annotated only so the reversed tree compiles; it adds
  no behaviour and never reaches the candidate (the patch's b/+ side is the untyped declaration).

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

- `tick/damage.rs @@ -5,6 +5,9 @@` (the `#[path = "totem.rs"] mod totem;` line) and
  `tick/totem.rs @@ -0,0 +1,79 @@` — (b) the kept tests' rescue assertions need the module compiled;
  reversing only the rescue call leaves `totem::apply`/`entity_events` wiring intact, so the pre-image
  path (no rescue) is what the control runs. The module's `rescue` becomes unreferenced in the reversed
  tree (dead-code warning only).
- `survival.rs (@@ -183,6)` and the mc-data/mc-protocol component hunks — (b) API the kept tests call:
  `bypasses_invulnerability` is used by the totem module and the damage pipeline, and the
  `death_protection` component parsing is what the kept tests author
  (`ItemArgument::parse("stick[death_protection=...]")`); reversing them stops the control from
  compiling.
- `tick/damage.rs @@ -152,6/-295,6` (the `totems` vector and the post-loop apply) and the kept
  `if let Some(mut rescue) = rescue { … }` application block (`crates/mc-core/src/tick/damage.rs:693-720`)
  — (c) a later commit (`b7020b427`, "integrate death protection and entity damage state") rewrote that
  region around the decision point, so the pre-fix shape is "decision gone, application inert with
  `rescue == None`". No kept test calls the application block directly; it is left in place so the reverse
  stays one decision-point hunk. The `rescue.is_none() &&` term in `combat_hit`
  (`crates/mc-core/src/tick/damage.rs:689`) is neutral at `None`, so the reversed tree records the same
  hit identity up to the death decision.
- `docs/ARCHITECTURE.md` — (b) docs.
- Test files — (b) kept.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:tick_tests::combat_parity_tests::fix_mobs_totem_intercepts_lethal_damage_in_either_hand_but_not_void`
  (`crates/mc-core/src/tick/combat_parity_tests.rs:489`) — with the rescue gone, the lethal fall hit kills
  the player, so `crates/mc-core/src/tick/combat_parity_tests.rs:503`
  `assert!(deaths.is_empty(), "held totem must prevent death")` fails on the first hand.
- Client ref `harness:entities:native-v2-p4-totem` green receipt on the pinned candidate tree
  (`pin-release` of `f78cba26f`; `.analysis/validation/20261123T222754-native-v2-p4-totem-65mn_ngj/result.json`,
  all fifteen assertions true: rescue at one health, totem consumed, the three effects,
  `default_totem_survives_at_one_health`, and `no_totem_lethal_damage_kills_the_player`). The scenario is
  registered in `tools/harness/scenarios.py` + `backends/native_v2.py` and implemented by
  `_totem` in `backends/native_p4.py`; the reflective server assertion is
  `default_totem_applies_regeneration_absorption_and_fire_resistance` on the ordered effect receipt.
