use super::*;

#[test]
#[ignore = "requires the local vanilla 26.1.2 content cache"]
fn fresh_v2_sites_describe_generated_villagers_from_the_same_plans() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_owned(), 81);
    config.vanilla_data_dir = Some(
        std::env::var_os("KRASNOKAMEN_CONTENT_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla")
            }),
    );
    let startup = std::time::Instant::now();
    let registry = Arc::new(CoreRegistry::load(config.vanilla_data_dir.as_deref()).unwrap());
    let store = Arc::new(
        mc_world::native::NativeStore::create(
            &directory.path().join("world.db"),
            crate::native_checkpoint::native_identity(&config),
        )
        .unwrap(),
    );
    let manifest = CheckpointManifest {
        world_spawn: Some([0, 128, 0]),
        ..Default::default()
    };
    let mut state = load_dimension_state(
        &config,
        registry,
        crate::dimensions::Dimension::Overworld,
        &manifest,
        Replay::default(),
        store,
        &std::collections::BTreeMap::new(),
        None,
    )
    .unwrap();
    eprintln!("seed81 registry/dimension startup: {:?}", startup.elapsed());
    assert!(state.world.resident_positions().is_empty());
    let sites = state.world.village_sites_in_region((-68, -68), (101, 101));
    assert!(
        !sites.is_empty(),
        "fresh configured v2 world must expose native villages"
    );
    assert!(
        state.world.resident_positions().is_empty(),
        "site listing must not materialize chunks"
    );
    let site = &sites[0];
    assert!(!site.pieces.is_empty());
    let wanted = (site.min.z.div_euclid(16)..=site.max.z.div_euclid(16))
        .flat_map(|z| {
            (site.min.x.div_euclid(16)..=site.max.x.div_euclid(16))
                .map(move |x| (crate::boundary::ChunkPos { x, z }, 0))
        })
        .collect();
    let loaded = state.world.ensure_blocking(&wanted);
    let markers: Vec<_> = state
        .world
        .resident_positions()
        .iter()
        .flat_map(|pos| state.world.chunk(*pos).unwrap().generated_entities())
        .filter(|marker| marker.entity_type == "minecraft:villager")
        .filter(|marker| {
            site.pieces.iter().any(|piece| {
                marker.claim.contains(&format!(
                    "@{}:{}:{}#",
                    piece.position.x, piece.position.y, piece.position.z
                ))
            })
        })
        .collect();
    assert!(
        !markers.is_empty(),
        "listed village {:?} must place authored villagers",
        site.start_chunk
    );
    let expected: std::collections::HashSet<_> = markers
        .iter()
        .map(|marker| crate::villager::generated_uuid(&marker.claim))
        .collect();
    assert_eq!(
        expected.len(),
        markers.len(),
        "each authored villager has a unique claim"
    );
    let mut changes = crate::boundary::ChangeSet::default();
    crate::villager::generated::load(&mut state, &loaded, &mut changes);
    for marker in &markers {
        let uuid = crate::villager::generated_uuid(&marker.claim);
        let slot = state
            .entities
            .slot_of_uuid(uuid)
            .expect("authored villager materialized");
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        assert!(row.villager.is_some());
        assert_eq!(state.entities.pos(slot), marker.position);
        assert!(changes.entities_spawned.contains(&body.id));
    }
    let count = changes.entities_spawned.len();
    crate::villager::generated::load(&mut state, &loaded, &mut changes);
    assert_eq!(
        changes.entities_spawned.len(),
        count,
        "the same site must not duplicate villagers"
    );
    assert_eq!(
        state
            .world
            .village_sites_in_region(site.start_chunk, site.start_chunk),
        vec![site.clone()]
    );
    eprintln!(
        "seed81 listed {} villages; {:?} materialized {} original UUIDs",
        sites.len(),
        site.start_chunk,
        expected.len()
    );
}

/// Review wave2-10 M2: does the House assessment fit a generated house at all?
///
/// Lists the seed-24 village's pieces, what `piece_kinds` certifies for each and
/// what `assess(House)` still misses, so the adoption fix can be judged against
/// the real templates instead of a box-shaped fixture.
#[test]
#[ignore = "requires the local vanilla content cache"]
fn seed24_generated_pieces_show_which_house_assessment_fails() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_owned(), 24);
    config.vanilla_data_dir = Some(
        std::env::var_os("KRASNOKAMEN_CONTENT_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla")
            }),
    );
    let registry = Arc::new(CoreRegistry::load(config.vanilla_data_dir.as_deref()).unwrap());
    let store = Arc::new(
        mc_world::native::NativeStore::create(
            &directory.path().join("world.db"),
            crate::native_checkpoint::native_identity(&config),
        )
        .unwrap(),
    );
    let manifest = CheckpointManifest {
        world_spawn: Some([0, 128, 0]),
        ..Default::default()
    };
    let mut state = load_dimension_state(
        &config,
        registry,
        crate::dimensions::Dimension::Overworld,
        &manifest,
        Replay::default(),
        store,
        &std::collections::BTreeMap::new(),
        None,
    )
    .unwrap();
    let sites = state.world.village_sites_in_region((0, -24), (16, -8));
    let site = sites
        .iter()
        .find(|site| site.start_chunk == (8, -14))
        .or_else(|| sites.first())
        .expect("the seed-24 village");
    let wanted = (site.min.x.div_euclid(16)..=site.max.x.div_euclid(16))
        .flat_map(|x| {
            (site.min.z.div_euclid(16)..=site.max.z.div_euclid(16))
                .map(move |z| (crate::boundary::ChunkPos { x, z }, 0))
        })
        .collect();
    state.world.ensure_blocking(&wanted);
    let mut house_pieces = 0;
    let mut both = 0;
    let mut misses = Vec::new();
    for piece in &site.pieces {
        let Some((lo, hi)) = piece.bounds else {
            continue;
        };
        let Ok(bounds) = mc_script::ScriptWorldBounds::new([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
        else {
            continue;
        };
        let Some(kinds) = crate::settlements::autonomy::piece_kinds(piece, &state, &bounds) else {
            misses.push(format!("{:?}: unreadable", piece.template));
            continue;
        };
        if kinds.len() > 1 {
            both += 1;
        }
        if kinds.contains(&crate::settlements::buildings::BuildingKind::House) {
            house_pieces += 1;
            match crate::settlements::buildings::assess(
                &state,
                crate::settlements::buildings::BuildingKind::House,
                1,
                bounds,
            ) {
                Ok(_) => {}
                Err(err) => misses.push(format!(
                    "{:?} at {:?}: {:?}",
                    piece.template, lo, err.missing
                )),
            }
        }
    }
    eprintln!(
        "seed24 pieces={} house_pieces={} both={} assess_misses={}",
        site.pieces.len(),
        house_pieces,
        both,
        misses.len()
    );
    for miss in &misses {
        eprintln!("  {miss}");
    }
    assert!(
        house_pieces >= 1,
        "the seed-24 village advertises at least one piece with bed heads: pieces={} misses={:?}",
        site.pieces.len(),
        misses
    );
}
