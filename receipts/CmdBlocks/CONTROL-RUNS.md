# Control method (CmdBlocks, base bbf5b71e5)

Each row's control reverses only that row's production hunks on this tree, runs
the row's scoped cargo ref, then restores the tree with the forward patch. The
patch-mode fix patches are the HEAD equivalents of the original fix commits
(D131): every original hunk that no longer exists at HEAD is mapped in the row's
`repro_check` / the promotion `hunks.md` with a class-(a)/(b)/(c) reason.

Patch identities are computed with the ledger's own splitter
(`tools.harness.ledger._split_non_test`), so `fix_commit = patch:<sha256>`
matches what `ledger promote --fix-patch` recomputes.

| row | reversal | patch identity |
|---|---|---|
| DATAPACK-NEW-PACK-WORLDGEN-SKIP | `docs/bugs/fix-patches/DATAPACK-NEW-PACK-WORLDGEN-SKIP.patch` | `patch:7cbe9205537a5db15a7239da475c757211071d08c1c50173a76fa8847613b51b` (unchanged) |
| DATAPACK-WORLDGEN-OVERRIDES | `docs/bugs/fix-patches/DATAPACK-WORLDGEN-OVERRIDES.patch` | `patch:05d3c72bf147c5be1c901d7c01aa9145caba07dad26d7579a5b253e09b0c858a` (unchanged) |
| SETTLEMENT-FURNACE-SITE-ON-ANOTHER-STRUCTURES-ROOF | `docs/bugs/fix-patches/SETTLEMENT-FURNACE-SITE-ON-ANOTHER-STRUCTURES-ROOF.patch` | `patch:822579e1e302205de429cb44cb37b2f93b1c8c9624f75df67bfdd964c84eb88c` (new HEAD equivalent: 9026719a07 + bbfe2e0b9) |
| SETTLEMENT-HARVEST-REOPENS-FARM-REPAIR | `docs/bugs/fix-patches/SETTLEMENT-HARVEST-REOPENS-FARM-REPAIR.patch` | `patch:f0db3e3b2c2cf65a8499ec65842dc9a9b3a9e6057ec2c11f35c00a42b80b87cf` (new HEAD equivalent) |
| SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN | `docs/bugs/promotions/SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN.patch` | `patch:113f3d8cdcd459341498bbacb7401c098a5ed5418ad72e8cf2163ea416e4aa43` (new HEAD equivalent) |
| SETTLEMENT-NATURAL-MINE-ZONE-NEVER-CERTIFIES | `docs/bugs/promotions/SETTLEMENT-NATURAL-MINE-ZONE-NEVER-CERTIFIES.patch` | `patch:3006c98d6f760c6d2184875e2fe2f5296d19aa1fc886ece8bc678ecf4c435e35` (const hunk dropped, class b) |
| CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION | `docs/bugs/promotions/CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION.patch` | `patch:2b697b8843f47e2059d5a7ed52baa8c819ee8cf596646854ea2d51b2b2414a3a` (unchanged) |

All seven `git apply -R --check` pass at `bbf5b71e5` (the furnace row needed the two-commit patch because the later stand rule masks the planner hunk).
