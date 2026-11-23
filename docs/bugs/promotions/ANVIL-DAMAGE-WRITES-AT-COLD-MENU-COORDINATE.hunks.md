# ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE — promotion patch mapping

- patch: `promotions/ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE.patch` (copy of `docs/bugs/fix-patches/ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE.patch`, the reversed form used by `ledger promote --fix-patch`).
- control: reversal on the exact candidate makes the row's own ref red at its assertion; restore green (see `ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE.json`).

## Hunks

tick/containers.rs menu_source_cold, tick/input/inventory.rs (container_click, container_slot_state, container_button), tick/stations.rs damage_anvil guard. Omitted: the old tick.rs `#[path] mod stations_tests;` hunk — (c) 4a54f35e7 rewrote the test wiring (inline module now), and test hunks are (b).

## Omitted hunks and reasons

- Test files / inline `#[cfg(test)]` hunks: (b) kept so the control compiles and runs the ref.
- Other files of earlier fix commits: (a) another row's behaviour.
