//! Deterministic creature groups in newly generated Overworld chunks.
use std::collections::HashMap;
use std::sync::Arc;

use mc_physics::BlockMaterial;
use mc_world::{Chunk, GeneratedEntityMarker};

pub(crate) struct Population {
    seed: i64,
    rules: mc_data::biomes::BiomeSpawnRules,
    blocks: Arc<mc_world::BlockRegistry>,
    materials: mc_physics::BlockMaterialIds,
    dimensions: HashMap<String, (f64, f64)>,
}

impl Population {
    pub(crate) fn new(seed: i64, registry: &crate::registry::CoreRegistry) -> Self {
        Self {
            seed,
            rules: registry.biome_spawns.clone(),
            blocks: Arc::clone(&registry.blocks),
            materials: registry.materials.clone(),
            dimensions: registry
                .blueprints
                .iter()
                .map(|(_, blueprint)| {
                    (
                        blueprint.name.clone(),
                        (blueprint.half_width, blueprint.height),
                    )
                })
                .collect(),
        }
    }

    pub(super) fn populate(&self, chunk: &mut Chunk) {
        let pos = chunk.pos;
        let mut random = Random(
            (self.seed as u64)
                ^ (pos.x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ (pos.z as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
                ^ 0xa076_1d64_78bd_642f,
        );
        // Chunk-generation spawning uses the north-west biome at world top,
        // not the cave or the surface biome at an individual placement.
        let Some(section) = chunk.biomes.last() else {
            return;
        };
        let biome = section.get(0, 3, 0);
        let entries = self.rules.exact_entries(biome, "creature");
        let probability = self
            .rules
            .creature_spawn_probability(biome)
            .clamp(0.0, 0.999);
        let total: u64 = entries.iter().map(|entry| u64::from(entry.weight)).sum();
        if total == 0 || random.unit() >= probability {
            return;
        }
        let mut markers = chunk.generated_entities();
        let original_len = markers.len();
        let mut group = 0_u32;
        loop {
            let mut roll = random.below(total);
            let Some(entry) = entries.iter().find(|entry| {
                if roll < u64::from(entry.weight) {
                    true
                } else {
                    roll -= u64::from(entry.weight);
                    false
                }
            }) else {
                break;
            };
            if let Some(&(half_width, height)) = self.dimensions.get(entry.entity_type.as_str())
                && entry.max_count >= entry.min_count
            {
                let count = entry.min_count
                    + random.below(u64::from(entry.max_count - entry.min_count) + 1) as u32;
                let origin_x = random.below(16) as i32;
                let origin_z = random.below(16) as i32;
                for member in 0..count {
                    for _ in 0..4 {
                        let x = (origin_x + random.below(5) as i32 - random.below(5) as i32)
                            .clamp(1, 14) as u8;
                        let z = (origin_z + random.below(5) as i32 - random.below(5) as i32)
                            .clamp(1, 14) as u8;
                        let Some(y) = self.surface(
                            chunk,
                            x,
                            z,
                            entry.entity_type.as_str(),
                            half_width,
                            height,
                        ) else {
                            continue;
                        };
                        let position = [
                            f64::from(pos.x) * 16.0 + f64::from(x) + 0.5,
                            f64::from(y),
                            f64::from(pos.z) * 16.0 + f64::from(z) + 0.5,
                        ];
                        if markers.iter().any(|marker| {
                            (marker.position[0] - position[0]).abs() < 1.0
                                && (marker.position[2] - position[2]).abs() < 1.0
                                && (marker.position[1] - position[1]).abs() < 2.0
                        }) {
                            continue;
                        }
                        markers.push(GeneratedEntityMarker {
                            claim: format!(
                                "creature:{}:{}:{}:{}:{}",
                                self.seed, pos.x, pos.z, group, member
                            ),
                            entity_type: entry.entity_type.as_str().to_owned(),
                            position,
                            persistent: false,
                            villager_kind: String::new(),
                            profession: String::new(),
                            level: 1,
                            age: 0,
                            yaw: random.below(360) as f32,
                            pitch: 0.0,
                        });
                        break;
                    }
                }
            }
            group += 1;
            if random.unit() >= probability {
                break;
            }
        }
        if markers.len() != original_len {
            chunk.set_generated_entities(&markers);
        }
    }

    fn surface(
        &self,
        chunk: &Chunk,
        x: u8,
        z: u8,
        entity: &str,
        half_width: f64,
        height: f64,
    ) -> Option<i32> {
        let geometry = chunk.geometry();
        let top = geometry.min_y() + chunk.heightmaps.get("MOTION_BLOCKING")?.get(x, z) as i32;
        for y in (geometry.min_y() + 1..=top.min(geometry.max_y() - 2)).rev() {
            let floor = chunk.get_block(x, y - 1, z)?;
            if self.materials.classify(floor.0) == BlockMaterial::Air {
                continue;
            }
            let name = self.blocks.by_id(floor)?.block.id.as_str();
            if !valid_ground(entity, name)
                || (y - geometry.min_y()) < chunk.highest_opaque.get(x, z) as i32
            {
                return None;
            }
            let min_x = (f64::from(x) + 0.5 - half_width).floor() as i32;
            let max_x = (f64::from(x) + 0.5 + half_width - f64::EPSILON).floor() as i32;
            let min_z = (f64::from(z) + 0.5 - half_width).floor() as i32;
            let max_z = (f64::from(z) + 0.5 + half_width - f64::EPSILON).floor() as i32;
            for dx in min_x..=max_x {
                for dz in min_z..=max_z {
                    if !(0..16).contains(&dx) || !(0..16).contains(&dz) {
                        return None;
                    }
                    for h in y..y + height.ceil() as i32 {
                        if chunk
                            .get_block(dx as u8, h, dz as u8)
                            .is_none_or(|id| self.materials.classify(id.0) != BlockMaterial::Air)
                        {
                            return None;
                        }
                    }
                }
            }
            return Some(y);
        }
        None
    }
}

fn valid_ground(entity: &str, ground: &str) -> bool {
    match entity {
        "minecraft:camel" | "minecraft:turtle" => ground == "minecraft:sand",
        "minecraft:mooshroom" => ground == "minecraft:mycelium",
        "minecraft:polar_bear" => ground == "minecraft:ice",
        "minecraft:rabbit" => matches!(
            ground,
            "minecraft:grass_block" | "minecraft:sand" | "minecraft:snow" | "minecraft:snow_block"
        ),
        "minecraft:fox" => matches!(
            ground,
            "minecraft:grass_block"
                | "minecraft:snow"
                | "minecraft:snow_block"
                | "minecraft:podzol"
                | "minecraft:coarse_dirt"
        ),
        "minecraft:goat" => matches!(
            ground,
            "minecraft:grass_block"
                | "minecraft:stone"
                | "minecraft:snow"
                | "minecraft:snow_block"
                | "minecraft:packed_ice"
                | "minecraft:gravel"
        ),
        "minecraft:frog" => matches!(
            ground,
            "minecraft:grass_block"
                | "minecraft:mud"
                | "minecraft:mangrove_roots"
                | "minecraft:muddy_mangrove_roots"
        ),
        "minecraft:armadillo" => matches!(
            ground,
            "minecraft:grass_block"
                | "minecraft:coarse_dirt"
                | "minecraft:red_sand"
                | "minecraft:terracotta"
                | "minecraft:orange_terracotta"
                | "minecraft:yellow_terracotta"
                | "minecraft:brown_terracotta"
                | "minecraft:red_terracotta"
                | "minecraft:white_terracotta"
                | "minecraft:light_gray_terracotta"
        ),
        "minecraft:parrot" => {
            ground == "minecraft:grass_block"
                || ground.ends_with("_leaves")
                || ground.ends_with("_log")
        }
        _ => ground == "minecraft:grass_block",
    }
}

struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut x = self.0;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^ (x >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / ((1_u64 << 53) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creature_positions_reject_wrong_ground_darkness_and_obstructions() {
        let registry = crate::registry::CoreRegistry::load(None).unwrap();
        let population = Population::new(42, &registry);
        let biome = mc_data::Identifier::parse("minecraft:plains").unwrap();
        let mut chunk = Chunk::empty(
            mc_world::ChunkPos { x: 0, z: 0 },
            mc_world::BlockStateId(registry.states.air),
            biome,
        );
        let mut height = mc_world::chunk::Heightmap::zeroed();
        height.set(8, 8, 129);
        chunk
            .heightmaps
            .insert("MOTION_BLOCKING".into(), height.clone());
        chunk.highest_opaque = height;
        chunk.set_block(
            8,
            64,
            8,
            mc_world::BlockStateId(registry.states.grass_block),
        );
        assert_eq!(
            population.surface(&chunk, 8, 8, "minecraft:sheep", 0.45, 1.3),
            Some(65)
        );
        assert_eq!(
            population.surface(&chunk, 8, 8, "minecraft:camel", 0.45, 1.3),
            None
        );
        chunk.highest_opaque.set(8, 8, 131);
        assert_eq!(
            population.surface(&chunk, 8, 8, "minecraft:sheep", 0.45, 1.3),
            None
        );
        chunk.highest_opaque.set(8, 8, 129);
        chunk.set_block(8, 66, 8, mc_world::BlockStateId(registry.states.stone));
        assert_eq!(
            population.surface(&chunk, 8, 8, "minecraft:sheep", 0.45, 1.3),
            None
        );
    }
}
