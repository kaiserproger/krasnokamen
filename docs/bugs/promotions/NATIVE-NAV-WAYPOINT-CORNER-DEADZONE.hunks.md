# NATIVE-NAV-WAYPOINT-CORNER-DEADZONE — hunk map for the committed reversal pair

Promotion receipt: `promotions/NATIVE-NAV-WAYPOINT-CORNER-DEADZONE.json`.
Client pair: reversal `client:8c7ece043a` → re-land `client:00da7b10df` (the row's `fix_commit`).

## Why a reversal and not the pre-image pin

`895c63f3` fixes five rows at once (no single-row control from its whole patch) and its historical
pre-image client cannot start in the current private graph. The committed reversal pair reverses
this row's two production hunks at the tip and re-lands them.

## Original commit

`client:895c63f3` (this row's part is the centering step in `src/minecraft/navigation.rs` plus the
`open_door_requires_centering_before_the_next_path_segment` test, which stays).

## Hunks

| file | hunk | content | disposition |
|---|---|---|---|
| `src/minecraft/navigation.rs` | `@@ -467,12 +467,27 @@ impl Task` | `center_before_turn`: when the near waypoint cannot reach the next segment but the waypoint itself can, stop advancing and finish centering on the planned segment | REVERSED |
| `src/minecraft/navigation.rs` | `@@ -523,7 +538,10 @@ impl Task` | the movement request becomes `[center_before_turn \|\| distance > 0.12 * 0.12, jump]` | REVERSED — the pre-fix `self.movement(session, [distance > 0.12 * 0.12, jump])?;` single line is restored |

## a/-side adaptation

The second hunk's post-image had been re-wrapped into three lines by
`client:9a645aa99` "fix(client): clippy-1.95 clean and rustfmt clean across the stage0 union"
(2026-11-14), which is the commit `git blame -C -C` names for those three lines at the wave-2 tip
`530556e8` (`src/minecraft/navigation.rs:541-544`; `895c63f3` itself wrote the single 88-column line).
So `git apply -R` of the historical patch fails on context; the `a/-` side written in
`8c7ece043a` is still the historical single line from `895c63f3^` (66 columns, unchanged text),
and every other reversed line is the fix's own added line. `git diff 8c7ece043a 00da7b10df` is
exactly the two hunks above (patch-id `1b07f639240eeac9e0721eeee0b837b174681ddb`).

## Proving ref

`harness:entities:native-v2-lease-navigation` → `open_door_navigation_arrived`: from the 0.416 /
-0.498 doorway offset, `minecraft_navigate_to_block` must arrive through the open one-block door and
the observation must be grounded past it. The client unit ref
`minecraft::navigation::tests::open_door_requires_centering_before_the_next_path_segment` stays as
the fast oracle.
