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
/// The seed-24 village's real pieces certify by function and substance. Each
/// piece gets its own settlement record so adjacent pieces cannot overlap-certify
/// against each other; the assertions use only `certify_generated`/`inspect`, the
/// same API adoption calls, so the row's reversal control still compiles.
#[test]
#[ignore = "requires the local vanilla content cache"]
fn seed24_generated_pieces_certify_by_function() {
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
    use crate::settlements::buildings::{BuildingKind, certify_generated, inspect};
    let (mut houses, mut workshops, mut pairs, mut strict_refused) = (0, 0, 0, 0);
    let mut house_refusals = Vec::new();
    let mut stationless = Vec::new();
    let mut unexpected = Vec::new();
    for (index, piece) in site.pieces.iter().enumerate() {
        let Some((lo, hi)) = piece.bounds else {
            continue;
        };
        let Ok(bounds) = mc_script::ScriptWorldBounds::new([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
        else {
            continue;
        };
        let Some(kinds) = crate::settlements::autonomy::piece_kinds(piece, &state, &bounds) else {
            continue;
        };
        if kinds.is_empty() {
            continue;
        }
        if kinds.len() > 1 {
            pairs += 1;
        }
        let anchor = [(lo.x + hi.x) / 2, (lo.y + hi.y) / 2, (lo.z + hi.z) / 2];
        let id = state
            .settlements
            .create(
                &mut state.settlement_sequence,
                format!("piece-{index}"),
                "owner".into(),
                anchor,
                64,
            )
            .expect("a settlement per piece");
        for kind in kinds {
            match certify_generated(&mut state, id, kind, bounds) {
                Ok(_) => {
                    if kind == BuildingKind::House {
                        houses += 1;
                        if inspect(&state, kind, 1, bounds).is_err() {
                            strict_refused += 1;
                        }
                    } else {
                        workshops += 1;
                    }
                }
                // A farm/field piece is classified by its template, and a vanilla
                // plot carries no composter: it can only fail on workstations, and
                // it failed exactly the same way under the strict rule. Anything
                // else is a real refusal and is reported.
                Err(error)
                    if kind != BuildingKind::House
                        && error
                            .missing
                            .iter()
                            .all(|requirement| requirement.description == "workstations") =>
                {
                    stationless.push(format!("{:?} {kind:?}: {error:?}", piece.template));
                }
                Err(error) => {
                    let line = format!("{:?} {kind:?} at {lo:?}: {error:?}", piece.template);
                    if kind == BuildingKind::House {
                        house_refusals.push(line);
                    } else {
                        unexpected.push(line);
                    }
                }
            }
        }
    }
    eprintln!(
        "seed24 generated certification: houses={houses} workshops={workshops} pairs={pairs} \
         strict_refused={strict_refused} stationless={stationless:?} \
         house_refusals={house_refusals:?} unexpected={unexpected:?}"
    );
    assert!(
        strict_refused >= 1,
        "at least one real house box is the jigsaw shape the envelope refuses"
    );
    assert!(
        houses >= 1,
        "the village's houses certify as Houses: {house_refusals:?}"
    );
    assert!(
        house_refusals.is_empty(),
        "every classified House certifies by function: {house_refusals:?}"
    );
    assert!(
        unexpected.is_empty(),
        "a workshop refusal may only be a missing workstation (a stationless plot): {unexpected:?}"
    );
}
