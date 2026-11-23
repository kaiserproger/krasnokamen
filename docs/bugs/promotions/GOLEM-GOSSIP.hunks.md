# GOLEM-GOSSIP — HEAD re-break control patch

Patch: `docs/bugs/promotions/GOLEM-GOSSIP.patch` (`patch:923e76ca48eae6c74031259c38ba4c73ca12bbc1c7f8991b7a41ffe6aae36f7b`), the row's
`fix_commit`. It reverses on the committed tree (`git apply -R --check`).

## Guard older than the finding

The row's recorded `fix_commit` `1a53ca2a6` is tests only ("integrate
villagers"), and the summon path predates it; `git log -S 'fn golem_quorum'`
names `db1dc0403` as the last restructure of the file. This patch is therefore
the guarding code at HEAD, not a historical fix: the spawn inside
`villager/brain/social.rs` `golems()` (`state.spawn_mob(type_id, pos, 100.0)`
plus the `entities_spawned` change and cache push).

## Reverse and red

Reversed, an agreeing quorum still selects a standing position but no golem is
summoned, so `harness:entities:native-v2-golem-gossip` ("an iron golem appears
within 32 blocks") is red. The kept quorum helpers
(`wants_golem`/`golem_quorum`) stay, so the control compiles and the unit
regression still runs.

## Blocker resolved in tree

`/data merge entity` is implemented (`crates/mc-core/src/command_data.rs:428`,
seeded with Brain memories by
`tools/harness/backends/native_promotion_wave1.py:449`) and the gate is
registered (`tools/harness/scenarios.py:420`, profile `entities`, requires
xvfb). The row's old "blocked until wt/LedgerCommandsB10 lands" text is stale.
