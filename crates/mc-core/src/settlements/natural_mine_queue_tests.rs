//! A plains village's mine zone must survive the generated-village overlap rule,
//! with the village already adopted.
//!
//! Measured on seed 24 (HANDOFF-42): the real site has 117 pieces and every
//! `natural_zones` probe box overlapped 1-12 of them by X/Z, so `queue_natural`'s
//! X/Z-only rule refused the mine before any route proof existed. A mine's box
//! spans the village in Y too (floor `entrance-14`, top `entrance+1`), so the
//! rule is a 3D one: a mine may only take ground where no built piece stands
//! (streets and roads may cross it; `piece_kinds` never certifies them). Depth is
//! no protection - `safe()`/`natural()` are, and `certify_zone` refuses a zone
//! that 3D-intersects any certificate (HANDOFF-43 review).
//!
//! The test adopts the village first, exactly as `advance` does, so the
//! certificate-overlap refusal and the piece queue are both live; it then queues,
//! proves and certifies the mine, and asserts no adopted piece was lost.

use super::*;
use crate::config::CoreConfig;
use crate::dimensions::Dimension;
use crate::villager::VillagerProfession;

const ANCHOR: [i32; 3] = [131, 78, -217];

fn seed24_v2_world() -> (
    tempfile::TempDir,
    TickState,
    SettlementId,
    mc_world::GeneratedVillageSite,
) {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_owned(), 24);
    config.vanilla_data_dir =
        Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    let registry = std::sync::Arc::new(
        crate::registry::CoreRegistry::load(config.vanilla_data_dir.as_deref()).unwrap(),
    );
    let store = std::sync::Arc::new(
        mc_world::native::NativeStore::create(
            &directory.path().join("world.db"),
            crate::native_checkpoint::native_identity(&config),
        )
        .unwrap(),
    );
    let manifest = crate::journal::CheckpointManifest {
        world_spawn: Some([0, 128, 0]),
        ..Default::default()
    };
    let mut state = crate::core::load_dimension_state(
        &config,
        registry,
        Dimension::Overworld,
        &manifest,
        crate::journal::Replay::default(),
        store,
        &std::collections::BTreeMap::new(),
        None,
    )
    .unwrap();
    let mut want: rustc_hash::FxHashMap<crate::boundary::ChunkPos, i32> = Default::default();
    for dx in -10..=10 {
        for dz in -10..=10 {
            let pos = crate::boundary::ChunkPos {
                x: (ANCHOR[0] >> 4) + dx,
                z: (ANCHOR[2] >> 4) + dz,
            };
            want.insert(pos, 0);
            state
                .tickets
                .insert(format!("mine-queue-test:{}:{}", pos.x, pos.z), (pos, 0));
        }
    }
    state.world.ensure_blocking(&want);
    state.refresh_active_chunks(state.tick);
    let site = state
        .world
        .village_sites_in_region(
            ((ANCHOR[0] >> 4) - 2, (ANCHOR[2] >> 4) - 2),
            ((ANCHOR[0] >> 4) + 2, (ANCHOR[2] >> 4) + 2),
        )
        .into_iter()
        .next()
        .expect("the seed-24 plains village site");
    let id = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "seed24".into(),
            "owner".into(),
            ANCHOR,
            136,
        )
        .unwrap();
    {
        let record = state.settlements.get_mut(id).unwrap();
        record.bootstrap_storage = Some([131, 76, -217]);
    }
    for (index, profession) in [
        VillagerProfession::Lumberjack,
        VillagerProfession::Nitwit,
        VillagerProfession::Farmer,
        VillagerProfession::Nitwit,
        VillagerProfession::Weaponsmith,
        VillagerProfession::Librarian,
    ]
    .into_iter()
    .enumerate()
    {
        let x = 133.5 + f64::from(u32::try_from(index).unwrap()) * 0.5;
        let slot = crate::settlements::allocator::tests::villager(&mut state, [x, 79.0, -219.5]);
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, id, entity).unwrap();
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .data
            .profession = profession;
    }
    (directory, state, id, site)
}

/// The village's mine is queued despite the pieces standing over it, proved and
/// certified - with the village adopted first, so the adoption cannot lose a
/// piece and `certify_zone`'s certificate-overlap refusal is exercised.
#[test]
fn a_plains_village_queues_proves_and_certifies_its_mine() {
    let (_directory, mut state, id, site) = seed24_v2_world();
    assert!(
        site.pieces.len() > 20,
        "the site must be the real village, not an empty stand-in"
    );

    // 1. Adopt the village exactly as `advance` does: queue order, loaded chunks
    //    only, function classified by `piece_kinds`, certified through the
    //    adopted-piece caller. This is the gate's state (13 buildings there).
    let mut adopted = 0;
    for piece in &site.pieces {
        let Some((lo, hi)) = piece.bounds else {
            continue;
        };
        let Ok(bounds) = ScriptWorldBounds::new([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]) else {
            continue;
        };
        if !bounds_loaded(&state, &bounds) {
            continue;
        }
        let Some(kinds) = piece_kinds(piece, &state, &bounds) else {
            continue;
        };
        for kind in kinds {
            if super::buildings::certify_generated(&mut state, id, kind, bounds).is_ok() {
                adopted += 1;
            }
        }
    }
    assert!(
        adopted >= 5,
        "only {adopted} pieces adopted: too few to stand for the gate's village"
    );
    let total_adopted = state
        .settlements
        .get(id)
        .expect("settlement")
        .buildings
        .len();
    assert_eq!(total_adopted, adopted, "adoption counted differently");

    // 2. Queue the mine. A cursor queues only where the mine's 3D box holds no
    //    built piece (streets may cross it): this is the measurement the review
    //    asks for, and it prints which cursor first queues.
    let mut candidates = std::collections::VecDeque::new();
    let mut queued = None;
    for cursor in 0..8 {
        natural_zones(&mut state, id, ANCHOR, &site, cursor, &mut candidates);
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.kind == BuildingKind::Mine)
        {
            queued = Some((cursor, candidate.bounds, candidate.entrance));
            break;
        }
    }
    let Some((cursor, bounds, entrance)) = queued else {
        eprintln!(
            "MEASURE no_mine_cursor adopted={adopted} pieces={}",
            site.pieces.len()
        );
        panic!("no mine candidate queued: every probe box holds a built piece in 3D");
    };
    eprintln!(
        "MEASURE mine_queued cursor={cursor} min={:?} max={:?} entrance={entrance:?} adopted={adopted}",
        bounds.min, bounds.max
    );

    // 3. Prove the route and certify: `certify_zone` refuses a zone that
    //    3D-intersects any certificate, so a green here means the box really is
    //    clear of every adopted piece.
    let known: BTreeSet<uuid::Uuid> = state
        .entities
        .slots()
        .filter_map(|slot| {
            let row = state.entities.row(slot);
            row.villager
                .as_ref()
                .filter(|v| v.settlement == Some(id))
                .map(|_| row.uuid)
        })
        .collect();
    for _ in 0..120 {
        state.tick += 20;
        prove_natural(&mut state, id, Some(&site), &known, &mut candidates);
        if has_kind(&state, id, BuildingKind::Mine) {
            break;
        }
        if candidates.is_empty() {
            break;
        }
    }
    assert!(
        has_kind(&state, id, BuildingKind::Mine),
        "the queued mine was never certified"
    );

    // 4. Nothing was lost: the mine is the only added certificate, so the count
    //    without the mine (the adoption) plus one.
    let total = state
        .settlements
        .get(id)
        .expect("settlement")
        .buildings
        .len();
    assert_eq!(
        total,
        total_adopted + 1,
        "certifying the mine changed the {total_adopted} adopted certificates"
    );
}
