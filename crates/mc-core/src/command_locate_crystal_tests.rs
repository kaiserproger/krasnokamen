use super::*;
use std::path::{Path, PathBuf};

fn copy_pack(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(!kind.is_symlink());
        if kind.is_dir() {
            copy_pack(&entry.path(), &destination.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
#[ignore = "requires KRASNOKAMEN_CRYSTAL_DATAPACK actual seven-file fixture and local vanilla cache"]
fn locate_actual_mvs_crystal_from_counted_origin() {
    let source = PathBuf::from(
        std::env::var_os("KRASNOKAMEN_CRYSTAL_DATAPACK").expect("actual crystal pack"),
    );
    assert!(source.join("data/mvs/loot_table/crystal.json").is_file());
    let directory = tempfile::tempdir().unwrap();
    copy_pack(
        &source,
        &directory.path().join("datapacks/mvs-5.1.3-partial-crystal"),
    );
    let mut config = crate::CoreConfig::new(directory.path().to_owned(), 20261115);
    config.checkpoint_every_ticks = u64::MAX;
    config.vanilla_data_dir = Some(PathBuf::from(
        std::env::var_os("KRASNOKAMEN_CONTENT_CACHE").expect("local vanilla cache"),
    ));
    let structure = id("mvs:crystal");
    let loader =
        mc_data::village_data::VillageDataLoader::new(source.join("data/minecraft/worldgen"));
    let set = loader.load_structure_set(&structure).unwrap();
    let mc_data::village_data::PlacementSpec::RandomSpread(spec) = set.placement;
    let placement = mc_worldgen::village::placement::RandomSpreadPlacement {
        spacing: spec.spacing,
        separation: spec.separation,
        salt: i64::from(spec.salt),
        triangular: matches!(
            spec.spread_type,
            mc_data::village_data::RandomSpreadType::Triangular
        ),
    };
    let core = crate::Core::open(config.clone()).unwrap();
    assert!(core.registry().datapack_structures().contains(&structure));
    assert!(core.registry().chest_loot.get(&structure).is_some());
    let generator = core.overworld_generator().unwrap();
    let origin = (-256, 68, -256); // Floor of the counted client's [-255.5,68,-255.5].
    let centre = (-16_i32, -16_i32);
    let options = StructureSearchOptions::default();
    let found = search_structure(
        generator.as_ref(),
        origin,
        std::slice::from_ref(&structure),
        &SavedStarts::new(),
        &options,
        None,
    )
    .unwrap();
    eprintln!(
        "counted locate seed={} origin={origin:?} default_radius_rings={} result={found:?}",
        config.seed, options.radius_rings
    );
    let min = (centre.0 - 256, centre.1 - 256);
    let max = (centre.0 + 256, centre.1 + 256);
    let mut candidates = 0;
    let mut accepted = 0;
    for z in min.1.div_euclid(placement.spacing)..=max.1.div_euclid(placement.spacing) {
        for x in min.0.div_euclid(placement.spacing)..=max.0.div_euclid(placement.spacing) {
            let candidate = placement.potential_chunk(
                config.seed,
                x * placement.spacing,
                z * placement.spacing,
            );
            if candidate.0 < min.0
                || candidate.0 > max.0
                || candidate.1 < min.1
                || candidate.1 > max.1
            {
                continue;
            }
            candidates += 1;
            let sites = generator.structure_sites_in_region(candidate, candidate);
            let admitted = sites.iter().any(|site| site.structure == structure);
            accepted += usize::from(admitted);
            eprintln!("chunk_square_candidate={candidate:?} admitted={admitted}");
        }
    }
    eprintln!(
        "chunk_square={min:?}..={max:?} placement_candidates={candidates} admitted={accepted}"
    );
    let mut vanilla_ring_hit = None;
    'rings: for radius in 0_i32..=4 {
        // Vanilla samples origin_chunk + ring_offset * placement.spacing,
        // then getPotentialStructureChunk; the bound is regions, not chunks.
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x.abs().max(z.abs()) != radius {
                    continue;
                }
                let candidate = placement.potential_chunk(
                    config.seed,
                    centre.0 + x * placement.spacing,
                    centre.1 + z * placement.spacing,
                );
                if let Some(site) = generator
                    .structure_sites_in_region(candidate, candidate)
                    .into_iter()
                    .find(|site| site.structure == structure)
                {
                    eprintln!(
                        "vanilla_ring={radius} candidate={candidate:?} position={:?}",
                        site.position
                    );
                    vanilla_ring_hit = Some(site);
                    break 'rings;
                }
            }
        }
    }
    core.shutdown();
    assert!(
        vanilla_ring_hit.is_some(),
        "fixture must generate in vanilla's search envelope without changing seed/origin"
    );
    assert!(
        found.is_some(),
        "default /locate rejected an actual MVS start reachable by vanilla placement rings"
    );
}
