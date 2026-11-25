# HAPPY-GHAST-HARNESS: patch-mode fix identity

Original fix: `7d30f2991` (`fix(core): vanilla mob behaviours — … camel and happy-ghast multi-rider mounts`),
which gave the happy ghast a seat per passenger (`seat_capacity`), accepted `*_harness` equipment as its
`Action::Armor`, and required a harness before any ride. The commit's hunks no longer reverse on HEAD: later
commits rewrote `mounts.rs`'s rider bookkeeping (`mount.rider` → `mount.riders` + `live_riders`), the seat
helper's shape (`contract.behavior.vehicle_passenger.max_passengers` → `seat_capacity`) and the surrounding
context, so the row uses a scoped patch:
`fix_commit = "patch:0c07255492152b53fcd7453104062e1be91f08ca82afd1f5d4063ee67a955178"` —
`docs/bugs/promotions/HAPPY-GHAST-HARNESS.patch`, three hunks of `crates/mc-core/src/mounts.rs`.

## Carried

1. `seat_capacity` (`@@ -191,12 +191,12 @@`) — the multi-seat arm. **Class (b), mechanical adaptation**, stated openly: the
   pre-image before `7d30f2991` had no multi-seat arm at all (`_ => 1` for every mount, single rider). HEAD
   shares the arm with the camel (CAMEL-DASH-ABSENT's own row), so removing the whole arm would reverse the
   camel fix too. The a-side therefore removes only the happy-ghast pattern from the or-pattern
   (`"minecraft:camel" | "minecraft:happy_ghast"` → `"minecraft:camel"`) and restores the pre-fix doc line,
   which is the pre-image semantics as it reads in HEAD's vocabulary.
2. `action` (`@@ -265,6 +265,12 @@`) — the `Action::Armor` branch for `kind == "minecraft:happy_ghast" &&
   item.ends_with("_harness") && row.loot.body_armor.is_none()`. **Class (a)**: those six lines are
   `7d30f2991`'s own text, still unchanged at HEAD (`mounts.rs:268-273`), so the reversal deletes exactly what
   the fix added.
3. `action` (`@@ -340,6 +346,7 @@`) — the ride predicate's `&& (kind != "minecraft:happy_ghast" ||
   row.loot.body_armor.is_some())`. **Class (a)**, same reasoning (`mounts.rs:349`).

Verified: `git apply --check -R docs/bugs/promotions/HAPPY-GHAST-HARNESS.patch` passes on HEAD, i.e. the patch
applied forward is the fix and reversed is the pre-image.

## Proving refs

- `cargo:mc-core:vanilla_mob_behavior_tests::happy_ghast_harness_admits_four_riders_and_survives_rider_codec`
  — the control's red: with the patch reversed the ghast accepts one rider and no harness, so the four-rider
  assertion fails.
- `harness:entities:native-v2-happy-ghast-harness` — the candidate's green: gate
  `20261125T034806-native-v2-happy-ghast-harness-lcyel49r` on the clean pin (`f2b291c99`, 39.2 s, status
  passed). Its assertions: `no_harness_no_riders`, `rider_1..4_distinct_and_keeps_others`,
  `fifth_rider_refused_before_ordered_dismount`, `other_riders_retained`; the passenger trace in the artifact
  shows `1` → `1,83` → `1,83,88` → `1,83,88,90` → `1,83,88` (harness leaves) → `83,88`, and
  `screenshots/four-riders-mounted.png` is the graphical half.
- The control cannot run the graphical ref (no reverted-pin client build); its red is the cargo ref, as in
  HS-2 (POL-1 practice).
