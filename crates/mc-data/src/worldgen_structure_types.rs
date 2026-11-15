//! Structure codecs that the datapack jigsaw pipeline can execute.

use crate::Identifier;

/// Whether the structure type is supported by the datapack jigsaw pipeline.
/// Unsupported types must be refused rather than treated as vanilla jigsaws.
#[must_use]
pub fn supports_structure_type(id: &Identifier) -> bool {
    matches!(
        id.as_str(),
        "minecraft:jigsaw" | "moogs_structures:moogs_structures_generic_jigsaw_structure"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure_types_are_explicitly_namespaced() {
        assert!(supports_structure_type(&Identifier::parse("minecraft:jigsaw").unwrap()));
        assert!(!supports_structure_type(&Identifier::parse("example:jigsaw").unwrap()));
        assert!(supports_structure_type(
            &Identifier::parse("moogs_structures:moogs_structures_generic_jigsaw_structure").unwrap()
        ));
        assert!(!supports_structure_type(
            &Identifier::parse("moogs_structures:moogs_structures_generic_nether_jigsaw_structure").unwrap()
        ));
    }
}
