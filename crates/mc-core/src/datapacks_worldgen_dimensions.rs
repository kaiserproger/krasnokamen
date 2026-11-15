//! Resolve selected jigsaw consumers from their effective biome sets.
use super::*;
use crate::dimensions::Dimension;
use mc_data::village_data::{BiomeSet, StructureSpec, VillageDataLoader};

impl BiomeTags {
    pub(crate) fn structure_dimension(&self, spec: &StructureSpec) -> Result<Dimension, String> {
        let dimension = match &spec.biomes {
            BiomeSet::Tag(id) => biome_dimension(
                self.0
                    .get(id)
                    .ok_or_else(|| format!("Missing structure biome tag {id}"))?
                    .iter(),
            )?,
            BiomeSet::Biomes(values) => biome_dimension(values.iter())?,
        };
        let nether = mc_data::worldgen_structure_types::is_moog_nether(&spec.structure_type);
        let dimension = dimension.unwrap_or(if nether {
            Dimension::Nether
        } else {
            Dimension::Overworld
        });
        if spec.structure_type.namespace() == "moogs_structures"
            && nether != (dimension == Dimension::Nether)
        {
            return Err(format!(
                "Structure {} codec {} disagrees with its biome dimension",
                spec.id, spec.structure_type
            ));
        }
        Ok(dimension)
    }
}

fn biome_dimension<'a>(
    values: impl Iterator<Item = &'a mc_data::Identifier>,
) -> Result<Option<Dimension>, String> {
    let mut dimension = None;
    for biome in values {
        let current = if biome.namespace() == "minecraft" {
            match biome.path() {
                "nether_wastes" | "warped_forest" | "crimson_forest" | "soul_sand_valley"
                | "basalt_deltas" => Dimension::Nether,
                "the_end" | "end_highlands" | "end_midlands" | "small_end_islands"
                | "end_barrens" => {
                    return Err(format!(
                        "Datapack jigsaw biome {biome} requires an End jigsaw consumer"
                    ));
                }
                _ => Dimension::Overworld,
            }
        } else {
            Dimension::Overworld
        };
        if dimension.is_some_and(|previous| previous != current) {
            return Err("Datapack jigsaw biome set spans multiple dimensions".into());
        }
        dimension = Some(current);
    }
    Ok(dimension)
}

pub(super) fn route_sets(
    root: &Path,
    tags: &BiomeTags,
    sets: Vec<mc_data::Identifier>,
    mut dimensions: BTreeMap<mc_data::Identifier, Dimension>,
) -> Result<BTreeMap<Dimension, Vec<mc_data::Identifier>>, String> {
    let loader = VillageDataLoader::new(root.join("data/minecraft/worldgen"));
    let mut routes: BTreeMap<Dimension, Vec<mc_data::Identifier>> = BTreeMap::new();
    for id in sets {
        let set = loader.load_structure_set(&id).map_err(|e| e.to_string())?;
        let mut dimension = None;
        for entry in set.structures {
            let current = match dimensions.get(&entry.structure) {
                Some(dimension) => *dimension,
                None => {
                    let spec = loader
                        .load_structure(&entry.structure)
                        .map_err(|e| e.to_string())?;
                    let dimension = tags.structure_dimension(&spec)?;
                    dimensions.insert(entry.structure, dimension);
                    dimension
                }
            };
            if dimension.is_some_and(|previous| previous != current) {
                return Err(format!("Structure set {id} spans multiple dimensions"));
            }
            dimension = Some(current);
        }
        routes
            .entry(dimension.unwrap_or(Dimension::Overworld))
            .or_default()
            .push(id);
    }
    Ok(routes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn classify(names: &[&str]) -> Result<Option<Dimension>, String> {
        let ids: Vec<_> = names
            .iter()
            .map(|s| mc_data::Identifier::parse(*s).unwrap())
            .collect();
        biome_dimension(ids.iter())
    }
    #[test]
    fn nether_biomes_have_a_consumer_without_overworld_leakage() {
        assert_eq!(
            classify(&["minecraft:crimson_forest", "minecraft:nether_wastes"]).unwrap(),
            Some(Dimension::Nether)
        );
        assert_eq!(
            classify(&["minecraft:plains"]).unwrap(),
            Some(Dimension::Overworld)
        );
        assert!(
            classify(&["minecraft:plains", "minecraft:crimson_forest"])
                .unwrap_err()
                .contains("multiple dimensions")
        );
        assert!(
            classify(&["minecraft:the_end"])
                .unwrap_err()
                .contains("End jigsaw consumer")
        );
    }

    fn write_structure(root: &Path, name: &str, nether: bool, biome: &str) -> mc_data::Identifier {
        let codec = if nether {
            "moogs_structures:moogs_structures_generic_nether_jigsaw_structure"
        } else {
            "moogs_structures:moogs_structures_generic_jigsaw_structure"
        };
        let mut value = serde_json::json!({
            "type": codec, "start_pool":"test:start", "size":1,
            "biomes":[biome], "step":"surface_structures", "start_height":{"absolute":0},
            "spawn_overrides": {}
        });
        if nether {
            value["land_search_direction"] = serde_json::json!("HIGHEST_LAND");
        }
        let path = root.join(format!("data/test/worldgen/structure/{name}.json"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
        mc_data::Identifier::parse(format!("test:{name}")).unwrap()
    }

    #[test]
    fn moog_codec_dimension_mismatches_and_mixed_sets_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let loader = VillageDataLoader::new(root.path().join("data/minecraft/worldgen"));
        let tags = BiomeTags::default();
        for (name, nether, biome) in [
            ("wrong_nether", true, "minecraft:plains"),
            ("wrong_overworld", false, "minecraft:crimson_forest"),
        ] {
            let id = write_structure(root.path(), name, nether, biome);
            let spec = loader.load_structure(&id).unwrap();
            assert!(
                tags.structure_dimension(&spec)
                    .unwrap_err()
                    .contains("disagrees")
            );
        }
        let nether = write_structure(root.path(), "nether", true, "minecraft:nether_wastes");
        let overworld = write_structure(root.path(), "overworld", false, "minecraft:plains");
        let path = root
            .path()
            .join("data/test/worldgen/structure_set/mixed.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::json!({
            "placement":{"type":"minecraft:random_spread", "spacing":10, "separation":5, "salt":1},
            "structures":[{"structure":nether.as_str(), "weight":1}, {"structure":overworld.as_str(), "weight":1}]
        }).to_string()).unwrap();
        assert!(
            route_sets(
                root.path(),
                &tags,
                vec![mc_data::Identifier::parse("test:mixed").unwrap()],
                BTreeMap::new(),
            )
            .unwrap_err()
            .contains("multiple dimensions")
        );
    }
}
