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

/// The counters exist only while the plan target is enabled (one check per publish), so the
/// scenario runs under that exact filter, like the leg does.
fn with_plan_debug<T>(body: impl FnOnce() -> T) -> T {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::filter::EnvFilter::new(
            "mc_edge::server::plan=debug",
        ))
        .with_writer(std::io::sink)
        .finish();
    tracing::subscriber::with_default(subscriber, body)
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
    row.scanned = viewers
        .iter()
        .map(|viewer| viewer.entities_scanned)
        .sum::<u64>()
        - scanned_before;
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
    row.scanned = viewers
        .iter()
        .map(|viewer| viewer.entities_scanned)
        .sum::<u64>()
        - scanned_before;
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
        let row = with_plan_debug(|| publish_selected(&out, viewers, previous.as_ref(), &all));
        println!("round {round}: {row:?}");
        rows.push(row);
    }
    rows
}

fn advance(
    store: &mut EntityStore,
    slots: &[mc_core::entities::EntitySlot],
    out: &mut TickOutput,
    round: usize,
) {
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
        let row = with_plan_debug(|| publish_selected(&out, &mut viewers, previous.as_ref(), &all));
        println!("round {round} all: {row:?}");
        if round == 1 {
            same = row;
        }
        if round == 2 {
            // Round 2: four viewers are skipped (deferred by credit in a real leg).
            let keep: Vec<usize> = (0..4).collect();
            let row =
                with_plan_debug(|| publish_selected(&out, &mut viewers, previous.as_ref(), &keep));
            println!("round {round} half: {row:?}");
        }
        if round == 3 {
            staggered = row;
        }
    }
    assert!(
        same.moves > 0 && staggered.moves > 0,
        "both rounds must move residents"
    );
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
    assert_eq!(
        rows[0].rescans, 8,
        "one rescan per viewer on the first publish: {rows:?}"
    );
    assert!(
        rows[0].scanned > 0,
        "the first scan must walk the pen: {rows:?}"
    );
    for (round, row) in rows.iter().enumerate().skip(1) {
        assert_eq!(row.rescans, 0, "round {round} rescanned: {rows:?}");
        assert_eq!(row.scanned, 0, "round {round} walked the square: {rows:?}");
    }
}

/// EDGE-PUMP-SKIP-FORCES-FULL-RESCAN, index level: a snapshot the pump coalesced
/// over must not cost a full candidate scan of every viewer.
///
/// The pump skips tick outputs (`next_publication`, server/pump.rs). A viewer keeps the last
/// snapshot it actually planned, so after a skip its previous revision is the base of the skipped
/// output, not of the next one, and `entity_interest::prepare` (plan/entity_interest.rs) treats it
/// as non-contiguous and rescans the whole square. The chain keeps those steps as deltas, so the
/// same viewer stays contiguous. The pump-level path is fenced separately
/// (`server/pump_chain_tests.rs`); this one pins the index contract and its cost.
#[test]
fn an_index_given_the_chain_must_not_rescan_after_a_skip() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut viewers = vec![pen_viewer(1, [32.0, 64.0, 32.0])];
    let mut chain = crate::plan::PublicationChain::new(8);

    advance(&mut store, &slots, &mut out, 0);
    chain.record(&out.snapshot.entities);
    let previous = viewers[0].previous_entities().cloned();
    let first = with_plan_debug(|| publish_selected(&out, &mut viewers, previous.as_ref(), &[0]));
    assert_eq!(
        first.rescans, 1,
        "the first publish has no baseline: {first:?}"
    );

    // A contiguous publish (the normal case) walks the delta, no rescan.
    advance(&mut store, &slots, &mut out, 1);
    chain.record(&out.snapshot.entities);
    let previous = viewers[0].previous_entities().cloned();
    let contiguous =
        with_plan_debug(|| publish_selected(&out, &mut viewers, previous.as_ref(), &[0]));
    assert_eq!(
        contiguous.rescans, 0,
        "a contiguous publish must not rescan: {contiguous:?}"
    );

    // The pump skips the next publication: it drained it, so the chain holds
    // its step, but no viewer planned it.
    advance(&mut store, &slots, &mut out, 2);
    chain.record(&out.snapshot.entities);
    advance(&mut store, &slots, &mut out, 3);
    chain.record(&out.snapshot.entities);
    let previous = viewers[0].previous_entities().cloned();
    let after_skip =
        with_plan_debug(|| publish_chained(&out, &mut viewers, previous.as_ref(), &[0], &chain));
    assert_eq!(
        after_skip.rescans, 0,
        "one skipped publication must not cost a full rescan of every viewer: {after_skip:?}"
    );
    assert_eq!(
        after_skip.scanned, 0,
        "no candidate scan may run after a skip: {after_skip:?}"
    );
    assert!(
        after_skip.moves > 0,
        "the skipped window's moves must still be delivered: {after_skip:?}"
    );
}

/// Plan `selected` viewers against `chain`: the shared fan-out selector, so
/// this costs what the pump's publish costs.
fn publish_chained(
    out: &TickOutput,
    viewers: &mut [ViewerState],
    previous: Option<&mc_core::boundary::EntityViews>,
    selected: &[usize],
    chain: &crate::plan::PublicationChain,
) -> Publish {
    let mut index = TickPlanIndex::with_previous(out, previous);
    if index.entity_base.is_none() {
        let bridge = TickPlanIndex::bridge(
            chain,
            out.snapshot.entities.revision(),
            viewers
                .iter()
                .filter_map(|viewer| viewer.previous_entities())
                .map(mc_core::boundary::EntityViews::revision),
        );
        if let Some((base, steps)) = bridge {
            index = TickPlanIndex::with_chain(out, &steps, base);
        }
    }
    let index = index.with_chain_info(chain.info());
    let rescans_before: u64 = viewers.iter().map(|viewer| viewer.rescans).sum();
    let scanned_before: u64 = viewers.iter().map(|viewer| viewer.entities_scanned).sum();
    let mut row = Publish::default();
    for &slot in selected {
        let (packets, _) = plan_packets(out, &mut viewers[slot], &index);
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
    row.scanned = viewers
        .iter()
        .map(|viewer| viewer.entities_scanned)
        .sum::<u64>()
        - scanned_before;
    row
}

/// Plan one viewer against a prepared index and keep its packets.
fn plan_packets(
    out: &TickOutput,
    viewer: &mut ViewerState,
    index: &TickPlanIndex,
) -> (Vec<PlannedPacket>, u64) {
    let mut scratch = PlanScratch::default();
    let mut packets = Vec::new();
    let mut credit = WorldCredit::frames(usize::MAX);
    plan_entities(
        &out.snapshot,
        viewer,
        VIEW_DISTANCE,
        &mut scratch,
        index,
        &mut packets,
        &mut credit,
    );
    (packets, scratch.visible.len() as u64)
}

/// The packets as the client sees them: id and framed bytes, order-insensitive.
fn wire(packets: &[PlannedPacket]) -> Vec<(i32, Vec<u8>)> {
    let mut frames: Vec<(i32, Vec<u8>)> = packets
        .iter()
        .filter(|packet| packet.chunk.is_none())
        .map(|packet| (packet.id, packet.data.to_vec()))
        .collect();
    frames.sort_unstable();
    frames
}

/// EDGE-PUMP-SKIP-FORCES-FULL-RESCAN fence: a viewer whose base predates the
/// published snapshot must receive exactly what a full rescan of that snapshot
/// would deliver — including a spawn, a removal and a move that happened inside
/// the skipped outputs.
#[test]
fn a_skipped_window_delivers_what_a_full_rescan_delivers() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut chain = crate::plan::PublicationChain::new(8);
    let position = [32.0, 64.0, 32.0];
    // Two identical viewers: one bridges the skipped window through the chain,
    // the other is forced onto the full rescan of the same snapshot.
    let mut bridged = pen_viewer(1, position);
    let mut rescanned = pen_viewer(2, position);

    advance(&mut store, &slots, &mut out, 0);
    chain.record(&out.snapshot.entities);
    for viewer in [&mut bridged, &mut rescanned] {
        let previous = viewer.previous_entities().cloned();
        let mut index = TickPlanIndex::with_previous(&out, previous.as_ref());
        index = index.with_reveals(&out, None);
        plan_packets(&out, viewer, &index);
    }

    // Step one inside the window: everything moves, one resident is removed and
    // one cow spawns next to the viewer.
    let mut index = 0;
    while index < slots.len() {
        let moved = 2.0 + (index % 32) as f64 * 2.0 + 0.5;
        store.set_pos(slots[index], [moved, 64.0, 2.0 + (index / 32) as f64 * 2.0]);
        index += 1;
    }
    store.remove(slots[0]);
    store.spawn(
        EntityId(9_000),
        uuid::Uuid::from_u128(9_000),
        EntityKind::Mob {
            type_id: cow_type_id(),
        },
        ColdFields {
            pos: [33.0, 64.0, 33.0],
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
    );
    let snapshot = Arc::get_mut(&mut out.snapshot).expect("sole snapshot owner");
    snapshot.tick += 1;
    snapshot.entities = store.views();
    chain.record(&out.snapshot.entities);

    // Step two inside the window: the survivors move again.
    let mut index = 1;
    while index < slots.len() {
        let moved = 2.0 + (index % 32) as f64 * 2.0 + 1.0;
        store.set_pos(slots[index], [moved, 64.0, 2.0 + (index / 32) as f64 * 2.0]);
        index += 1;
    }
    let snapshot = Arc::get_mut(&mut out.snapshot).expect("sole snapshot owner");
    snapshot.tick += 1;
    snapshot.entities = store.views();
    chain.record(&out.snapshot.entities);

    // The publish neither viewer planned: the chain bridges it, the other
    // viewer plans it as a full rescan from the same base.
    let previous = bridged.previous_entities().cloned();
    let (base, steps) = TickPlanIndex::bridge(
        &chain,
        out.snapshot.entities.revision(),
        std::iter::once(previous.as_ref().expect("bridged baseline").revision()),
    )
    .expect("the chain covers the viewer's base");
    assert!(
        steps.iter().any(|link| link.tracks(EntityId(9_000))),
        "the cow spawned inside the skipped window must be in the chain"
    );
    assert!(
        steps.iter().any(|link| link.tracks(EntityId(1_000))),
        "the resident removed inside the skipped window must be in the chain"
    );
    let bridged_index = TickPlanIndex::with_chain(&out, &steps, base).with_chain_info(chain.info());
    let (bridged_packets, _) = plan_packets(&out, &mut bridged, &bridged_index);

    let rescanned_index = TickPlanIndex::new(&out);
    let (rescanned_packets, _) = plan_packets(&out, &mut rescanned, &rescanned_index);

    assert_eq!(
        wire(&bridged_packets),
        wire(&rescanned_packets),
        "the bridged window must deliver exactly the rescan's frames"
    );
    let tracked = |viewer: &ViewerState| {
        let mut ids: Vec<EntityId> = viewer.tracked_entities.keys().copied().collect();
        ids.sort_unstable();
        ids
    };
    assert_eq!(
        tracked(&bridged),
        tracked(&rescanned),
        "the bridged window must track exactly what the rescan tracks"
    );
    assert!(
        !tracked(&bridged).contains(&EntityId(9_000))
            || tracked(&rescanned).contains(&EntityId(9_000)),
        "a spawn the rescan tracks must also reach the bridged viewer"
    );
    assert!(
        !tracked(&bridged).contains(&EntityId(1_000)),
        "the resident removed inside the window must be untracked"
    );
}

/// EDGE-PUMP-SKIP-FORCES-FULL-RESCAN cost: a skipped publish costs a rescan of
/// every viewer without the chain, and the ring holds a bounded number of bytes.
#[test]
fn a_skipped_publish_costs_one_rescan_without_the_chain_and_none_with_it() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut chain = crate::plan::PublicationChain::new(32);
    let position = [32.0, 64.0, 32.0];
    // Paired runs: each round plans two fresh, identical viewers so neither
    // path warms the other and the timing order cannot bias the numbers.
    let mut without_viewer = pen_viewer(1, position);
    let mut with_viewer = pen_viewer(2, position);

    advance(&mut store, &slots, &mut out, 0);
    chain.record(&out.snapshot.entities);
    for viewer in [&mut without_viewer, &mut with_viewer] {
        let previous = viewer.previous_entities().cloned();
        let mut index = TickPlanIndex::with_previous(&out, previous.as_ref());
        index = index.with_reveals(&out, None);
        plan_packets(&out, viewer, &index);
    }
    let baseline_scan = without_viewer.entities_scanned;

    for round in 1..=3 {
        advance(&mut store, &slots, &mut out, round);
        chain.record(&out.snapshot.entities);
    }
    let ring_bytes = chain.bytes();
    let ring_links = chain.len();

    // The index is built once per publish and shared by every viewer; the plan
    // is per viewer. Both are reported: that is the trade the chain makes.
    let previous = without_viewer.previous_entities().cloned();
    let without = {
        let started = std::time::Instant::now();
        let mut index = TickPlanIndex::with_previous(&out, previous.as_ref());
        index = index.with_reveals(&out, None);
        let build_us = std::time::Instant::now()
            .duration_since(started)
            .as_micros();
        let rescans = without_viewer.rescans;
        let scanned = without_viewer.entities_scanned;
        let started = std::time::Instant::now();
        let _ = plan_packets(&out, &mut without_viewer, &index);
        let plan_us = std::time::Instant::now()
            .duration_since(started)
            .as_micros();
        (
            build_us,
            plan_us,
            without_viewer.rescans - rescans,
            without_viewer.entities_scanned - scanned,
        )
    };

    let previous = with_viewer.previous_entities().cloned();
    let with = {
        let started = std::time::Instant::now();
        let (base, steps) = TickPlanIndex::bridge(
            &chain,
            out.snapshot.entities.revision(),
            std::iter::once(previous.as_ref().expect("baseline").revision()),
        )
        .expect("chain covers the base");
        let mut index = TickPlanIndex::with_chain(&out, &steps, base).with_chain_info(chain.info());
        index = index.with_reveals(&out, None);
        let build_us = std::time::Instant::now()
            .duration_since(started)
            .as_micros();
        let rescans = with_viewer.rescans;
        let scanned = with_viewer.entities_scanned;
        let started = std::time::Instant::now();
        let _ = plan_packets(&out, &mut with_viewer, &index);
        let plan_us = std::time::Instant::now()
            .duration_since(started)
            .as_micros();
        (
            build_us,
            plan_us,
            with_viewer.rescans - rescans,
            with_viewer.entities_scanned - scanned,
        )
    };
    println!(
        "skipped publish: without chain build_us={} plan_us={} rescans={} scanned={}; \
         with chain build_us={} plan_us={} rescans={} scanned={}; \
         ring links={ring_links} bytes={ring_bytes}; first scan={baseline_scan}",
        without.0, without.1, without.2, without.3, with.0, with.1, with.2, with.3
    );
    assert_eq!(
        (without.2, with.2),
        (1, 0),
        "the chain must remove the rescan and nothing else: without={without:?} with={with:?}"
    );
    assert_eq!(
        with.3, 0,
        "the bridged publish scans no candidates: {with:?}"
    );
    assert!(without.3 > 0, "the rescan walks the square: {without:?}");
    assert!(
        ring_bytes > 0 && ring_links == 3,
        "three drained steps at capacity: links={ring_links} bytes={ring_bytes}"
    );
}
