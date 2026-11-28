# CLIENT-WARM-NAUTILUS-ROOT — reversal pair map

Real fix: `client:f4bd26d4`, a bulk ClientQ04 asset commit. Client fixes that predate the
rename (`f0a6abf6`) have no buildable pre-fix client, so the row is proved by a committed
reversal on the tip whose diff equals the forward-ported real fix:

- control `client:47caa48c7dc752e5e3d133d0bb3afe88b8e061c5` removes exactly the carried hunks below;
- re-land `client:dddee26e8ac92b7d90a67bff834033939e23ee9e` restores them (`fix_commit`).

Carried hunks: `assets/solaris/models/entity/warm_zombie_nautilus.json` — the warm zombie nautilus row.
Everything else is a model this row does not own; the runtime selection is data-driven
from the variant registry, so no production code hunk belongs to this row.

| original file | HEAD equivalent | reason |
|---|---|---|
| `assets/solaris/models/entity/armor_baby_zombie_chest.json` | none | (a) CLIENT-ZOMBIE-BABY-HELMET-HEIGHT's armour shell, another row of the same bulk commit; reversing it would turn that row's assertions red |
| `assets/solaris/models/entity/armor_baby_zombie_feet.json` | none | (a) same armour family, another row |
| `assets/solaris/models/entity/armor_baby_zombie_head.json` | none | (a) same armour family, another row |
| `assets/solaris/models/entity/armor_baby_zombie_legs.json` | none | (a) same armour family, another row |
| `assets/solaris/models/entity/bed_foot.json` | none | (c) deleted again by client:109ec84a (26.3 has no bed block entity), so no HEAD equivalent exists |
| `assets/solaris/models/entity/bed_head.json` | none | (c) deleted again by client:109ec84a |
| `assets/solaris/models/entity/bell.json` | none | (a) CLIENT-BELL-AUTHORED-SILHOUETTE, another row |
| `assets/solaris/models/entity/chicken_baby.json` | `assets/krasnokamen/models/entity/chicken_baby.json` (carried by another row's map) | this row |
| `assets/solaris/models/entity/cold_chicken.json` | none | (a) the cold *adult* chicken rig serves the adult scene, not this row's baby |
| `assets/solaris/models/entity/cold_chicken_baby.json` | `assets/krasnokamen/models/entity/cold_chicken_baby.json` (carried by another row's map) | this row |
| `assets/solaris/models/entity/cold_cow.json` | `assets/krasnokamen/models/entity/cold_cow.json` (carried by another row's map) | this row |
| `assets/solaris/models/entity/cold_cow_baby.json` | `assets/krasnokamen/models/entity/cold_cow_baby.json` (carried by another row's map) | this row |
| `assets/solaris/models/entity/cold_pig.json` | none | (a) the cold pig is not this row's subject |
| `assets/solaris/models/entity/cold_pig_baby.json` | none | (a) same |
| `assets/solaris/models/entity/decorated_pot_base.json` | none | (a) CLIENT-DECORATED-POT-NECK, another row |
| `assets/solaris/models/entity/decorated_pot_sides.json` | none | (a) CLIENT-DECORATED-POT-NECK, another row |
| `assets/solaris/models/entity/husk_baby.json` | none | (a) the shared husk body belongs to CLIENT-ZOMBIE-BABY-HELMET-HEIGHT |
| `assets/solaris/models/entity/trident.json` | none | (a) CLIENT-HELD-TRIDENT-ORIENTATION, another row |
| `assets/solaris/models/entity/warm_cow.json` | `assets/krasnokamen/models/entity/warm_cow.json` (carried by another row's map) | this row |
| `assets/solaris/models/entity/warm_cow_baby.json` | `assets/krasnokamen/models/entity/warm_cow_baby.json` (carried by another row's map) | this row |
| `assets/solaris/models/entity/warm_zombie_nautilus.json` | `assets/krasnokamen/models/entity/warm_zombie_nautilus.json` (carried verbatim) | this row |
| `assets/solaris/models/entity/zombie_baby.json` | none | (a) the child humanoid body belongs to CLIENT-ZOMBIE-BABY-HELMET-HEIGHT |
| `docs/ENTITY_MODELS.md` | none | (a) documents every row of the bulk commit |
| `src/entities/asset_model_regression_tests.rs` | none | (a) tests other rows' model wiring |
| `src/entities/presentation.rs` | none | (a) wither-skull material, the bed item's special route and the trident's display frame: other rows' behaviour |
| `src/entities/render_blocks.rs` | none | (a) the bed item's block-model halves: CLIENT-BED-DEFAULT-TEXTURE-PATH |
| `src/entities/render_blocks/village_tests.rs` | none | (a) the bed item's test, another row |
| `src/entities/resources.rs` | none | (a) a `#[cfg(test)]` selection inventory only; it changes no runtime selection |
| `src/entities/solaris_models/tests.rs` | none | (a) the other rows' inline tests (test hunks stay out of a production patch by rule) |
| `tools/entity_models.py` | none | (a) the modeller's tooling, no runtime effect |
| `tools/test_entity_models.py` | none | (a) the modeller's tooling test |

No a/-side adaptation: the control is a plain removal/restore of the data files, and the
kept tests (the row's `client:` refs) read the embedded assets, so nothing had to be
reconstructed to compile.

Proving refs and assertions are recorded on the row (`proving_assertions`).
