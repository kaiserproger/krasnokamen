//! Deterministic publish-cost scenario for the R3k-5 cliff.
//!
//! The R3k-5 leg showed the pump's per-viewer publish cost step 351 µs → 28 367 µs across
//! 33 → 37 viewers and then stay flat. This scenario answers the only two questions that
//! separate the candidate paths, with **counts, never timings**:
//!
//! * does the shared move cache actually share (`move_hits`), or does every viewer encode its
//!   own frames because its baseline differs (`move_misses ≈ moves`)?
//! * does `entity_interest::prepare` rescan the whole square every publish (`rescans ≈
//!   publishes`), or does it walk the entity delta (rescans on the first publish only)?
//!
//! The scene mirrors the leg: ~1 008 residents of one pen in a 4×4-chunk square, a viewer whose
//! 5×5 window is that pen, residents moving each publish, snapshots chained through one
//! `EntityStore` so the entity revisions are contiguous exactly as the pump's are.
//!
//! The counts are printed with `--nocapture` and asserted where they pin a contract.
use super::*;
use mc_core::blueprint::MovementClass;
use mc_core::entities::{ColdFields, EntityKind, EntityStore};

const RESIDENTS: i32 = 1008;
const VIEW_DISTANCE: i32 = 2;
const PUBLICATIONS: usize = 4;

fn cow_type_id() -> u32 {
    mc_data::entity_types::entity_type_contract_26_1_2_by_name("minecraft:cow")
        .expect("cow contract")
        .protocol_id
}

fn pen_chunks() -> Vec<(ChunkPos, Arc<mc_world::chunk::Chunk>)> {
    (0..4)
        .flat_map(|x| (0..4).map(move |z| chunk_at(x, z)))
        .collect()
}

/// 1 008 residents on a 2-block grid inside the 4×4-chunk pen, all chained in one store.
fn pen_store() -> (EntityStore, Vec<mc_core::entities::EntitySlot>) {
    let mut store = EntityStore::default();
    let type_id = cow_type_id();
    let mut slots = Vec::with_capacity(RESIDENTS as usize);
    let mut placed = 0;
    for row in 0..32 {
        for column in 0..32 {
            if placed == RESIDENTS {
                break;
            }
            let pos = [2.0 + column as f64 * 2.0, 64.0, 2.0 + row as f64 * 2.0];
            let id = EntityId(1_000 + placed);
            slots.push(store.spawn(
                id,
                uuid::Uuid::from_u128(1_000 + placed as u128),
                EntityKind::Mob { type_id },
                ColdFields {
                    pos,
                    vel: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    ground: true,
                    health: 20.0,
                    breed_cooldown: 0,
                    pickup_delay: 0,
                    item: None,
                    appearance: Arc::new(EntityAppearance::default()),
                    movement: MovementClass::Walker,
                    half_width: 0.45,
                    height: 1.4,
                    speed: 0.1,
                    attack_damage: 1.0,
                    sight_range: 16.0,
                    attack_reach: 1.0,
                },
            ));
            placed += 1;
        }
        if placed == RESIDENTS {
            break;
        }
    }
    (store, slots)
}

fn pen_viewer(connection: u64, pos: [f64; 3]) -> ViewerState {
    let mut viewer = viewer_at(connection, pos);
    for x in 0..4 {
        for z in 0..4 {
            viewer.loaded_chunks.insert((x, z), 0);
        }
    }
    viewer
}

#[derive(Debug, Default, Clone, Copy)]
struct Publish {
    packets: usize,
    spawns: usize,
    moves: usize,
    hits: u64,
    misses: u64,
    rescans: u64,
    scanned: u64,
}

/// One publish over a shared index (one move cache, as the pump's fan-out has), counts only.
fn publish(
    out: &TickOutput,
    viewers: &mut [ViewerState],
    previous: Option<&mc_core::boundary::EntityViews>,
) -> Publish {
    let index = TickPlanIndex::with_previous(out, previous);
    let rescans_before: u64 = viewers.iter().map(|viewer| viewer.rescans).sum();
    let scanned_before: u64 = viewers.iter().map(|viewer| viewer.entities_scanned).sum();
    let mut row = Publish::default();
    for viewer in viewers.iter_mut() {
        let mut scratch = PlanScratch::default();
        let mut packets = Vec::new();
        let mut credit = WorldCredit::frames(usize::MAX);
        plan_entities(
            &out.snapshot,
            viewer,
            VIEW_DISTANCE,
            &mut scratch,
            &index,
            &mut packets,
            &mut credit,
        );
        row.spawns += packets.iter().filter(|packet| packet.id == 0x01).count();
        row.moves += packets
            .iter()
            .filter(|packet| {
                packet.id == mc_protocol::packets::play::MoveEntityPos::ID
                    || packet.id == mc_protocol::packets::play::MoveEntityRot::ID
                    || packet.id == mc_protocol::packets::play::MoveEntityPosRot::ID
                    || packet.id == mc_protocol::packets::play::EntityPositionSync::ID
            })
            .count();
        row.packets += packets.len();
    }
    let (hits, misses) = index.move_tallies();
    row.hits = hits;
    row.misses = misses;
    row.rescans = viewers.iter().map(|viewer| viewer.rescans).sum::<u64>() - rescans_before;
    row.scanned = viewers.iter().map(|viewer| viewer.entities_scanned).sum::<u64>() - scanned_before;
    row
}

/// Drive `PUBLICATIONS` publishes: every resident moves 0.5 blocks per publish, the snapshot
/// chain is contiguous through the store.
fn drive(viewers: &mut Vec<ViewerState>) -> Vec<Publish> {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut rows = Vec::new();
    for round in 0..PUBLICATIONS {
        if round > 0 {
            for (index, &slot) in slots.iter().enumerate() {
                let x = 2.0 + (index % 32) as f64 * 2.0 + round as f64 * 0.5;
                let z = 2.0 + (index / 32) as f64 * 2.0;
                store.set_pos(slot, [x, 64.0, z]);
            }
        }
        {
            let snapshot = Arc::get_mut(&mut out.snapshot).expect("sole snapshot owner");
            snapshot.tick += 1;
            snapshot.entities = store.views();
        }
        // The pump builds one index from the published tree's previous revision; clone the
        // handle so the viewers stay free for the mutable planning pass.
        let previous = viewers[0].previous_entities().cloned();
        let row = publish(&out, viewers, previous.as_ref());
        println!("round {round}: {row:?}");
        rows.push(row);
    }
    rows
}

/// Plan one publish, optionally skipping viewers (a skip is a publish that viewer never saw, so
/// its tracked baselines lag the others: the case the leg's staggered joins create).
fn publish_selected(
    out: &TickOutput,
    viewers: &mut [ViewerState],
    previous: Option<&mc_core::boundary::EntityViews>,
    selected: &[usize],
) -> Publish {
    let index = TickPlanIndex::with_previous(out, previous);
    let rescans_before: u64 = viewers.iter().map(|viewer| viewer.rescans).sum();
    let scanned_before: u64 = viewers.iter().map(|viewer| viewer.entities_scanned).sum();
    let mut row = Publish::default();
    for &slot in selected {
        let viewer = &mut viewers[slot];
        let mut scratch = PlanScratch::default();
        let mut packets = Vec::new();
        let mut credit = WorldCredit::frames(usize::MAX);
        plan_entities(
            &out.snapshot,
            viewer,
            VIEW_DISTANCE,
            &mut scratch,
            &index,
            &mut packets,
            &mut credit,
        );
        row.spawns += packets.iter().filter(|packet| packet.id == 0x01).count();
        row.moves += packets
            .iter()
            .filter(|packet| {
                packet.id == mc_protocol::packets::play::MoveEntityPos::ID
                    || packet.id == mc_protocol::packets::play::MoveEntityRot::ID
                    || packet.id == mc_protocol::packets::play::MoveEntityPosRot::ID
                    || packet.id == mc_protocol::packets::play::EntityPositionSync::ID
            })
            .count();
        row.packets += packets.len();
    }
    let (hits, misses) = index.move_tallies();
    row.hits = hits;
    row.misses = misses;
    row.rescans = viewers.iter().map(|viewer| viewer.rescans).sum::<u64>() - rescans_before;
    row.scanned = viewers.iter().map(|viewer| viewer.entities_scanned).sum::<u64>() - scanned_before;
    row
}

/// Drive `PUBLICATIONS` publishes where every viewer plans every round.
fn drive_all(viewers: &mut Vec<ViewerState>) -> Vec<Publish> {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut rows = Vec::new();
    for round in 0..PUBLICATIONS {
        advance(&mut store, &slots, &mut out, round);
        let previous = viewers[0].previous_entities().cloned();
        let all: Vec<usize> = (0..viewers.len()).collect();
        let row = publish_selected(&out, viewers, previous.as_ref(), &all);
        println!("round {round}: {row:?}");
        rows.push(row);
    }
    rows
}

fn advance(store: &mut EntityStore, slots: &[mc_core::entities::EntitySlot], out: &mut TickOutput,
           round: usize) {
    if round > 0 {
        for (index, &slot) in slots.iter().enumerate() {
            let x = 2.0 + (index % 32) as f64 * 2.0 + round as f64 * 0.5;
            let z = 2.0 + (index / 32) as f64 * 2.0;
            store.set_pos(slot, [x, 64.0, z]);
        }
    }
    let snapshot = Arc::get_mut(&mut out.snapshot).expect("sole snapshot owner");
    snapshot.tick += 1;
    snapshot.entities = store.views();
}

#[test]
fn viewers_with_the_same_delivery_history_share_almost_every_move_frame() {
    // The shared cache is keyed on the tracked baseline the CLIENT holds (plan/shared_moves.rs
    // key = id + position/yaw/pitch/head bits), not on where the viewer stands: viewers that
    // received the same publishes hold the same baselines and share the frame.
    let mut viewers: Vec<ViewerState> = (1..=8u64)
        .map(|connection| pen_viewer(connection, [8.0 + connection as f64, 64.0, 32.0]))
        .collect();
    let rows = drive_all(&mut viewers);
    let after = &rows[1..];
    let moves: usize = after.iter().map(|row| row.moves).sum();
    let misses: u64 = after.iter().map(|row| row.misses).sum();
    let hits: u64 = after.iter().map(|row| row.hits).sum();
    assert!(moves > 0, "residents must move: {rows:?}");
    assert_eq!(
        misses + hits,
        moves as u64,
        "every planned move is one cache lookup: {rows:?}"
    );
    assert!(
        hits > misses * 5,
        "8 identical histories must share ~7 of 8 frames: {rows:?}"
    );
}

#[test]
fn a_viewer_that_missed_a_publish_pays_its_own_encodes() {
    // A viewer planned every round tracks the newest baseline; one that skipped a round holds an
    // older one, so its keys are unique and it cannot share. This is what staggered joins and
    // deferred credit do to the shared cache in a real leg.
    let mut viewers: Vec<ViewerState> = (1..=8u64)
        .map(|connection| pen_viewer(connection, [8.0 + connection as f64, 64.0, 32.0]))
        .collect();
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut same = Publish::default();
    let mut staggered = Publish::default();
    for round in 0..PUBLICATIONS {
        advance(&mut store, &slots, &mut out, round);
        let previous = viewers[0].previous_entities().cloned();
        let all: Vec<usize> = (0..viewers.len()).collect();
        let row = publish_selected(&out, &mut viewers, previous.as_ref(), &all);
        println!("round {round} all: {row:?}");
        if round == 1 {
            same = row;
        }
        if round == 2 {
            // Round 2: four viewers are skipped (deferred by credit in a real leg).
            let keep: Vec<usize> = (0..4).collect();
            let row = publish_selected(&out, &mut viewers, previous.as_ref(), &keep);
            println!("round {round} half: {row:?}");
        }
        if round == 3 {
            staggered = row;
        }
    }
    assert!(same.moves > 0 && staggered.moves > 0, "both rounds must move residents");
    let hits_per_miss = |row: &Publish| row.hits as f64 / row.misses as f64;
    assert!(
        hits_per_miss(&same) > 3.0,
        "identical histories share most frames: {same:?}"
    );
    assert!(
        hits_per_miss(&staggered) < hits_per_miss(&same),
        "four lagging viewers must cost more encodes than none: {same:?} vs {staggered:?}"
    );
}

#[test]
fn contiguous_snapshots_rescan_only_until_the_window_is_known() {
    // A rescan is the only per-publish O(the whole square) pass (plan/entity_interest.rs:74-79).
    // With contiguous entity revisions and an unchanged chunk set it happens once per window,
    // not once per publish.
    let mut viewers: Vec<ViewerState> = (1..=8u64)
        .map(|connection| pen_viewer(connection, [8.0 + connection as f64, 64.0, 32.0]))
        .collect();
    let rows = drive_all(&mut viewers);
    assert_eq!(rows[0].rescans, 8, "one rescan per viewer on the first publish: {rows:?}");
    assert!(rows[0].scanned > 0, "the first scan must walk the pen: {rows:?}");
    for (round, row) in rows.iter().enumerate().skip(1) {
        assert_eq!(row.rescans, 0, "round {round} rescanned: {rows:?}");
        assert_eq!(row.scanned, 0, "round {round} walked the square: {rows:?}");
    }
}
