# PAINTINGS: c15347e6 hunk map (patch mode)

Reversing all non-test hunks of c15347e6 leaves a tree that does not build: the kept tests in
interact_parity_tests.rs use `Spawners::contains` (monster_spawner.rs) and the shared
`tick::post_write_consequences` (tick/build.rs). The fix patch holds the one behaviour hunk.

| c15347e6 hunk | what it does | in the fix patch |
|---|---|---|
| dimensions_commands.rs @@ -196 (`crate::tick::post_write_consequences(world, &writes, &mut spawned)`) | **behaviour**: the shared command commit runs the post-write consequences (painting/item-frame support, item components, spawner) | **yes** |
| tick/build.rs (new `post_write_consequences`) | extraction of the four calls the tick block phase already made | no: the tick path keeps them either way |
| tick.rs (block phase calls the extracted function) | same calls as before, through the helper | no: behaviour-neutral refactor |
| monster_spawner.rs (`Spawners::contains`) | accessor for the tests | no: test API |
| interact_parity_tests.rs, tools/harness, docs | tests, gate, docs | no |

Control (20261111T164223-ledger-promote-4e5bc4ws): with the hunk reversed,
`painting_drops_when_shared_command_commit_removes_support` is red and
`interact_parity_painting_places_on_wall` stays green (placement is not this fix); the real-client
gate was unrunnable in the control, so the client surface is proven only on the fixed tree
(probe 20261111T162925-ledger-probe-suitl0l0). Restored: both cargo refs green.
