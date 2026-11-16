//! Common biome conventions referenced by the selected structure datapacks.
//!
//! These authored defaults describe vanilla biome membership. Optional legacy
//! `forge:` references retain additions from other enabled packs. The defaults
//! are a lower-priority layer; ordinary pack additions and replacement apply.
//!
//! Meanings: <https://maven.fabricmc.net/docs/fabric-api-0.115.6+1.21.1/net/fabricmc/fabric/api/tag/convention/v2/ConventionalBiomeTags.html>.
//! Vanilla membership was checked against the local NeoForge 21.1.259 data
//! inventory recorded in `receipts/AtmPilot/MVS-DATA.md`; no loader code is used.

/// A tag entry and whether a missing referenced resource is an error.
pub type BiomeTagEntry = (&'static str, bool);

/// Lowest-priority authored common-biome tag defaults.
#[must_use]
pub fn default_biome_tags() -> &'static [(&'static str, &'static [BiomeTagEntry])] {
    &[
        ("c:is_beach", &[("#minecraft:is_beach", true)]),
        (
            "c:is_birch_forest",
            &[
                ("minecraft:birch_forest", true),
                ("minecraft:old_growth_birch_forest", true),
            ],
        ),
        ("c:is_deep_ocean", &[("#minecraft:is_deep_ocean", true)]),
        (
            "c:is_desert",
            &[("minecraft:desert", true), ("#forge:is_desert", false)],
        ),
        (
            "c:is_floral",
            &[
                ("#c:is_flower_forest", true),
                ("minecraft:cherry_grove", true),
                ("minecraft:meadow", true),
                ("minecraft:sunflower_plains", true),
                ("#forge:is_floral", false),
            ],
        ),
        ("c:is_flower_forest", &[("minecraft:flower_forest", true)]),
        ("c:is_forest", &[("#minecraft:is_forest", true)]),
        ("c:is_jungle", &[("#minecraft:is_jungle", true)]),
        (
            "c:is_mountain",
            &[
                ("#minecraft:is_mountain", true),
                ("#c:is_mountain/peak", true),
                ("#c:is_mountain/slope", true),
                ("#forge:is_mountain", false),
            ],
        ),
        (
            "c:is_mountain/peak",
            &[
                ("minecraft:frozen_peaks", true),
                ("minecraft:jagged_peaks", true),
                ("minecraft:stony_peaks", true),
                ("#forge:is_peak", false),
            ],
        ),
        (
            "c:is_mountain/slope",
            &[
                ("minecraft:cherry_grove", true),
                ("minecraft:grove", true),
                ("minecraft:meadow", true),
                ("minecraft:snowy_slopes", true),
                ("#forge:is_slope", false),
            ],
        ),
        (
            "c:is_mushroom",
            &[
                ("minecraft:mushroom_fields", true),
                ("#forge:is_mushroom", false),
            ],
        ),
        (
            "c:is_nether",
            &[("#minecraft:is_nether", true), ("#forge:is_nether", false)],
        ),
        (
            "c:is_ocean",
            &[
                ("#minecraft:is_ocean", true),
                ("#c:is_deep_ocean", true),
                ("#c:is_shallow_ocean", true),
            ],
        ),
        (
            "c:is_overworld",
            &[
                ("#minecraft:is_overworld", true),
                ("#forge:is_overworld", false),
            ],
        ),
        (
            "c:is_plains",
            &[
                ("minecraft:plains", true),
                ("minecraft:sunflower_plains", true),
                ("#forge:is_plains", false),
            ],
        ),
        ("c:is_savanna", &[("#minecraft:is_savanna", true)]),
        (
            "c:is_shallow_ocean",
            &[
                ("minecraft:cold_ocean", true),
                ("minecraft:frozen_ocean", true),
                ("minecraft:lukewarm_ocean", true),
                ("minecraft:ocean", true),
                ("minecraft:warm_ocean", true),
            ],
        ),
        (
            "c:is_snowy",
            &[
                ("minecraft:frozen_peaks", true),
                ("minecraft:grove", true),
                ("minecraft:ice_spikes", true),
                ("minecraft:jagged_peaks", true),
                ("minecraft:snowy_beach", true),
                ("minecraft:snowy_plains", true),
                ("minecraft:snowy_slopes", true),
                ("minecraft:snowy_taiga", true),
                ("#forge:is_snowy", false),
            ],
        ),
        (
            "c:is_swamp",
            &[
                ("minecraft:mangrove_swamp", true),
                ("minecraft:swamp", true),
                ("#forge:is_swamp", false),
            ],
        ),
        ("c:is_taiga", &[("#minecraft:is_taiga", true)]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn common_biome_defaults_cover_mvs_and_keep_optional_references() {
        let defaults = default_biome_tags();
        let tags: BTreeSet<_> = defaults.iter().map(|(id, _)| *id).collect();
        for id in [
            "c:is_beach",
            "c:is_birch_forest",
            "c:is_desert",
            "c:is_floral",
            "c:is_forest",
            "c:is_jungle",
            "c:is_mountain",
            "c:is_mushroom",
            "c:is_nether",
            "c:is_ocean",
            "c:is_overworld",
            "c:is_plains",
            "c:is_savanna",
            "c:is_snowy",
            "c:is_swamp",
            "c:is_taiga",
        ] {
            assert!(tags.contains(id), "missing {id}");
        }
        let overworld = defaults
            .iter()
            .find(|(id, _)| *id == "c:is_overworld")
            .unwrap()
            .1;
        assert!(overworld.contains(&("#minecraft:is_overworld", true)));
        assert!(overworld.contains(&("#forge:is_overworld", false)));
        for (_, entries) in defaults {
            for (id, required) in *entries {
                if *required && id.starts_with("#c:") {
                    assert!(tags.contains(&id[1..]), "missing referenced tag {id}");
                }
            }
        }
    }
}
