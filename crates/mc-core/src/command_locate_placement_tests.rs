use super::*;
use mc_worldgen::village::placement::RandomSpreadPlacement;

struct VillagePlacement {
    placement: RandomSpreadPlacement,
    candidate: ChunkPos,
}

impl ChunkGenerator for VillagePlacement {
    fn generate(&self, _: ChunkPos) -> Chunk {
        panic!("locate must not generate chunks");
    }

    fn structure_candidates_in_ring(
        &self,
        targets: &[Identifier],
        centre: (i32, i32),
        ring: u32,
    ) -> Vec<ChunkPos> {
        if !targets.contains(&id("minecraft:village_plains")) {
            return Vec::new();
        }
        self.placement
            .candidate_chunks_in_ring(20261115, centre, ring)
    }

    fn structure_sites_in_region(
        &self,
        min: (i32, i32),
        max: (i32, i32),
    ) -> Vec<mc_world::chunk::GeneratedStructureSite> {
        let pos = self.candidate;
        if pos.x < min.0 || pos.x > max.0 || pos.z < min.1 || pos.z > max.1 {
            return Vec::new();
        }
        vec![mc_world::chunk::GeneratedStructureSite {
            structure: id("minecraft:village_plains"),
            start_chunk: (pos.x, pos.z),
            position: BlockPos {
                x: pos.x * 16 + 8,
                y: 72,
                z: pos.z * 16 + 8,
            },
        }]
    }
}

#[test]
fn vanilla_placement_search_uses_rings_and_saved_coverage_beyond_old_chunk_bound() {
    // Ordinary vanilla village placement, including a negative-coordinate origin.
    let placement = RandomSpreadPlacement {
        spacing: 34,
        separation: 8,
        salt: 10_387_312,
        triangular: false,
    };
    let centre = (-16, -16);
    let (x, z) = placement.potential_chunk(20261115, centre.0 - 10 * placement.spacing, centre.1);
    let generator = VillagePlacement {
        placement,
        candidate: ChunkPos { x, z },
    };
    assert!((x - centre.0).abs() > 256);
    let origin = (-256, 68, -256);
    let targets = [id("minecraft:village_plains")];
    let mut options = StructureSearchOptions {
        radius_rings: 9,
        ..Default::default()
    };
    let mut saved = SavedStarts::new();
    assert!(
        search_structure(&generator, origin, &targets, &saved, &options, None)
            .unwrap()
            .is_none()
    );
    options.radius_rings = 10;
    let hit = search_structure(&generator, origin, &targets, &saved, &options, None)
        .unwrap()
        .expect("tenth placement ring");
    assert_eq!(hit.pos, (x * 16 + 8, 72, z * 16 + 8));
    saved.insert(generator.candidate, Vec::new());
    assert!(
        search_structure(&generator, origin, &targets, &saved, &options, None)
            .unwrap()
            .is_none(),
        "covered empty candidate suppresses plans beyond the old 256-chunk snapshot bound"
    );
    let imported = BlockPos {
        x: x * 16 + 3,
        y: -20,
        z: z * 16 + 5,
    };
    saved.insert(
        generator.candidate,
        vec![mc_world::chunk::GeneratedStructureSite {
            structure: targets[0].clone(),
            start_chunk: (x, z),
            position: imported,
        }],
    );
    let hit = search_structure(&generator, origin, &targets, &saved, &options, None)
        .unwrap()
        .unwrap();
    assert_eq!(hit.pos, (imported.x, imported.y, imported.z));
}
