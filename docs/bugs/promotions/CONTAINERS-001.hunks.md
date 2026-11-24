# CONTAINERS-001: patch-mode fix identity

Original fix: `2ccf8cc1a` (`fix(core): restore mount lectern and command container custody`) — it created
`crates/mc-core/src/mounts/menus.rs` and wired the mount menus into the mount action plan (`Action::Inventory`
and `Action::Chest`), the per-tick maintenance pass and the click/select dispatch. Before it, a tamed mount's
secondary interaction produced no menu and a chest in hand attached nothing, which is the row's symptom
(chested donkey/mule cargo menu absent).

The whole commit does not reverse on HEAD and the module cannot be reversed at all (`MountState`'s
`cargo`/`chested`/`strength` fields and `menus.rs`'s own slots/sync helpers are kept API). The row is a
scoped patch: `fix_commit = "patch:534ed846e3dae342a44edbbf05f95b940d337857d86907feb1b5efb02eb5920b"`, the
HEAD equivalents of the decision and dispatch hunks, in
`docs/bugs/promotions/CONTAINERS-001.patch`. Reversing it leaves `mounts/menus.rs` in place but unreachable:
a tamed mount opens no inventory and a chest is never attached. Re-audit:
`python3 -m tools.harness ledger promote CONTAINERS-001 --fix-patch docs/bugs/promotions/CONTAINERS-001.patch --dry-run`.

## Carried

- `mounts.rs` `Action` enum — the `Inventory` and `Chest` variants (`crates/mc-core/src/mounts.rs:47-48`).
- `mounts.rs` `action()` — the tamed-mount menu branch (secondary -> `Inventory`, chest in hand on a
  donkey/mule/llama -> `Chest`) (`:240-256`).
- `mounts.rs` `below_native_cost_floors` — `Action::Chest` consumes one item (`:434`).
- `mounts.rs` `apply()` — the `Action::Inventory` (`menus::open`) and `Action::Chest` (chested flag + cargo
  resize) arms (`:465-492`).
- `mounts.rs` `apply()` — the `menus::sync` pass after an appearance change (`:617-619`).
- `tick/dimension.rs` — the per-tick `crate::mounts::menus::maintain` pass and its lap (`:263-264`).
- `tick/input/inventory.rs` — the mount branch of `select_bundle_item` (`:74-75`) and of `container_click`
  (`:232-237`).

## Not carried

- `crates/mc-core/src/mounts/menus.rs` (new, 223 lines) and `MountState`'s new fields with
  `mounts/persistence.rs`'s hydration of them — (b) kept API: `tick.rs` keeps the `lecterns`-style field and
  the module's helpers, so reversing them stops the control from compiling. Their callers are all among the
  carried hunks (the accepted kept-helper class).
- The lectern hunks of the same commit (`lectern.rs`, `tick/input/interact.rs`, the lectern line of
  `container_kind_for_state`, `tick/input/inventory.rs`'s button dispatch) — (a) CONTAINERS-002, whose own
  control carries them.
- The crafting-table backing hunk of the same commit — (a) CONTAINERS-003.
- Everything else (command-block custody, redstone, world, edge, protocol, docs) — (a) other rows; the full
  hunk list is in `CONTAINERS-003.hunks.md`.
- Test files — (b) kept so the control compiles and runs the refs.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:mounts::tests::mount_inventory_opens_for_each_tamed_mount` — with the branch reversed a
  tamed horse/donkey/mule/llama/camel secondary interaction opens nothing and the ref fails at its own
  `assert!(state.players[&actor].open.is_some(), "{name} inventory")`
  (`crates/mc-core/src/mounts_tests.rs:288-312`).
- `cargo:mc-core:mounts::tests::mount_inventory_equipment_cargo_and_ridden_open_share_native_custody` — the
  chest attach and the 17-slot cargo menu are gone, so its own chest/cargo assertions fail
  (`crates/mc-core/src/mounts_tests.rs:316` onward).
- `harness:entities:native-v2-ledger-donkey-cargo` — a real client attaches a chest to a tamed donkey,
  receives `HorseInventoryMenu` with 53 slots, and moves seven diamonds into the first cargo slot and back
  through the received menu (`tools/harness/backends/native_ledger_donkey_cargo.py`). Passed receipt: see
  the promotion JSON. The older `native-v2-custody-mount-export` entry is unrunnable on this host: its
  backend requires a client hook (`promotion_mount_export_reimport`) that the client repository does not
  implement.
