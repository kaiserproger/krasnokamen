# MAP-FRAME-AUTO-EXPLORE: patch-mode fix identity

The row says a map merely carried in a pocket keeps painting terrain, so standing near a wall map
(or carrying a clone) can reveal pixels the player never viewed. The fix is `33812b466`
(`fix(core): selectors span dimensions … held-only map exploration`): it builds an `explorers` set from
`selected`/`OFF_HAND` only and gates the sampling loop on it.

`33812b466` cannot be reversed on HEAD any more — the same commit rewrote water/nether sampling in the file and
its hunks no longer apply (`git apply -R --check` fails at `maps/runtime.rs:237`), and it also carries selector,
plan and ledger changes that must not be reverted. So the row uses a scoped patch of the three exploration
statements, each the **pre-image text verbatim**:

1. `let mut explorers: BTreeMap<i32, Vec<(f64, f64)>> = BTreeMap::new();` — did not exist before the fix
   (class (c): the fix added the set, so the reversal deletes it).
2. the `if index == usize::from(player.inventory.selected) || index == crate::inventory::OFF_HAND { explorers…
   push((pos[0], pos[2])) }` block — likewise added by the fix (class (c) deletion).
3. the gate itself — **class (b), pre-image text verbatim** from `33812b466^`:
   `let holders = &markers[&id]; let held = holders.iter().any(|&(_, _, _, frame, _, _)| !frame); if !map.locked &&
   map.dimension == state.dimension && held {` and the holder selection `let (x, z, _, _, _, _) = holders.iter()
   .enumerate().filter(|(_, holder)| !holder.3).nth(start % holders.iter().filter(|holder| !holder.3).count()
   .max(1)).map(|(_, holder)| *holder).unwrap_or(holders[start % holders.len()]);`.

`fix_commit = "patch:16b582af0e074f9621034e53ff1df7d4a3d54b595dddb2c3a4aca62408bf4179"`,
`docs/bugs/promotions/MAP-FRAME-AUTO-EXPLORE.patch`; `git apply --check -R` passes on the candidate.

## Proving refs

- `cargo:mc-core:tick_tests::maps_tests::framed_and_pocket_maps_do_not_explore_until_held` — with the patch
  reversed (i.e. the pre-image gate in place) the pocket-carried map paints and the test's own assertion on the
  explored count fails.
- `harness:entities:native-v2-ledger-map-pocket` — the candidate's green: the client fills a map, drags it to
  inventory slot 20, walks six legs and the `maps` observation keeps reporting zero explored pixels
  (`pocket_map_never_explores`, `pocket_map_still_unexplored`). The control's graphical half is `unrunnable`
  (no reverted-pin client build), so the red comes from the cargo ref — POL-1 practice.

## Scope notes

- A framed copy is **not** a discriminator: before the fix a wall map alone never explored, because the
  pre-image gate already required a carried (`frame=false`) copy. The scenario therefore uses a pocket map only.
- The fix does not change what viewers *receive*: markers and received pixels still reach frame viewers, which
  is why the assertion is on the exploring player's own record.
