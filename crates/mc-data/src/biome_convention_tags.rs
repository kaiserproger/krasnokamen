//! Documented common-namespace biome conventions used by supported datapacks.
//!
//! Values are tag references (`#namespace:path`) or biome identifiers. They
//! form the lowest-priority layer, so enabled packs may add or replace entries.

/// Authored defaults for common biome tags with established vanilla equivalents.
///
/// Fabric's documented `c:is_overworld` means biomes that spawn in the
/// Overworld; vanilla's `minecraft:is_overworld` names that same set:
/// <https://maven.fabricmc.net/docs/fabric-api-0.115.6+1.21.1/net/fabricmc/fabric/api/tag/convention/v2/ConventionalBiomeTags.html>
#[must_use]
pub fn default_biome_tags() -> &'static [(&'static str, &'static [&'static str])] {
    &[("c:is_overworld", &["#minecraft:is_overworld"])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overworld_convention_uses_vanilla_biome_tag() {
        assert_eq!(default_biome_tags(), &[("c:is_overworld", &["#minecraft:is_overworld"][..])]);
    }
}
