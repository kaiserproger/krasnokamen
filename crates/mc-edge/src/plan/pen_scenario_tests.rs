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

/// Move every resident and republish the store's views: the snapshot chain
/// stays contiguous through the store, exactly as the pump's outputs are.
fn advance(
    store: &mut EntityStore,
    slots: &[mc_core::entities::EntitySlot],
    out: &mut TickOutput,
    round: usize,
) {
    if round > 0 {
        for (index, &slot) in slots.iter().enumerate() {
            let x = 2.0 + (index % 32) as f64 * 2.0 + round as f64 * 0.5;
            store.set_pos(slot, [x, 64.0, 2.0 + (index / 32) as f64 * 2.0]);
        }
    }
    let snapshot = Arc::get_mut(&mut out.snapshot).expect("sole snapshot owner");
    snapshot.tick += 1;
    snapshot.entities = store.views();
}

/// Plan `selected` viewers against one index, optionally skipping the others
/// (a skip is a publish those viewers never saw).
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
        let (packets, _, _) = plan_packets(out, &mut viewers[slot], &index);
        count(&mut row, &packets);
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
fn drive_all(viewers: &mut [ViewerState]) -> Vec<Publish> {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut rows = Vec::new();
    for round in 0..PUBLICATIONS {
        advance(&mut store, &slots, &mut out, round);
        let previous = viewers[0].previous_entities().cloned();
        let all: Vec<usize> = (0..viewers.len()).collect();
        let row = with_plan_debug(|| publish_selected(&out, viewers, previous.as_ref(), &all));
        rows.push(row);
    }
    rows
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
/// The pump records every drained output (`PublicationHistory`) and the
/// fan-out bridges the base a viewer still holds, so `entity_interest::prepare`
/// stays contiguous. The pump path is fenced separately
/// (`server/pump_chain_tests.rs`); this one pins the index contract, the net
/// delta it builds, and its cost.
#[test]
fn an_index_given_the_chain_must_not_rescan_after_a_skip() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut viewers = vec![pen_viewer(1, [32.0, 64.0, 32.0])];
    let mut history = crate::plan::PublicationHistory::new(8);

    advance(&mut store, &slots, &mut out, 0);
    history.record(&out);
    let first = with_plan_debug(|| publish_now(&out, &mut viewers, &history, &[0]));
    assert_eq!(
        first.rescans, 1,
        "the first publish has no baseline: {first:?}"
    );

    // A contiguous publish (the normal case) walks the delta, no rescan.
    advance(&mut store, &slots, &mut out, 1);
    history.record(&out);
    let contiguous = with_plan_debug(|| publish_now(&out, &mut viewers, &history, &[0]));
    assert_eq!(
        contiguous.rescans, 0,
        "a contiguous publish must not rescan: {contiguous:?}"
    );

    // The pump drains the next publications without planning them. B repeats:
    // a revision the core did not advance because its wire image did not
    // change, which must be a no-op for the chain, not a cut.
    advance(&mut store, &slots, &mut out, 2);
    history.record(&out);
    history.record(&out);
    advance(&mut store, &slots, &mut out, 3);
    history.record(&out);
    assert_eq!(
        history.chain(out.snapshot.dimension).len(),
        4,
        "the repeated revision recorded once: A, step 1, step 2, step 3"
    );
    let skipped = with_plan_debug(|| publish_now(&out, &mut viewers, &history, &[0]));
    assert_eq!(
        skipped.rescans, 0,
        "one skipped publication must not cost a full rescan of every viewer: {skipped:?}"
    );
    assert_eq!(
        skipped.scanned, 0,
        "no candidate scan may run after a skip: {skipped:?}"
    );
    assert!(
        skipped.moves > 0,
        "the skipped window's moves must still be delivered: {skipped:?}"
    );
}

/// Plan `selected` viewers the way the fan-out does: index from the delivered
/// base, bridged through the pump's history when that base is covered.
fn publish_now(
    out: &TickOutput,
    viewers: &mut [ViewerState],
    history: &crate::plan::PublicationHistory,
    selected: &[usize],
) -> Publish {
    let previous = viewers
        .first()
        .and_then(|viewer| viewer.previous_entities())
        .cloned();
    let mut index = TickPlanIndex::with_previous(out, previous.as_ref());
    let chain = history.chain(out.snapshot.dimension);
    if index.entity_base.is_none()
        && let Some((base, _, changed)) = TickPlanIndex::bridge(
            chain,
            out.snapshot.entities.revision(),
            viewers
                .iter()
                .filter_map(|viewer| viewer.previous_entities()),
        )
    {
        index = TickPlanIndex::with_chain(out, base, &changed);
    }
    let index = index.with_chain_info(chain.info()).with_reveals(out, None);
    let rescans_before: u64 = viewers.iter().map(|viewer| viewer.rescans).sum();
    let scanned_before: u64 = viewers.iter().map(|viewer| viewer.entities_scanned).sum();
    let mut row = Publish::default();
    for &slot in selected {
        let (packets, _, _) = plan_packets(out, &mut viewers[slot], &index);
        count(&mut row, &packets);
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

fn count(row: &mut Publish, packets: &[PlannedPacket]) {
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

/// Plan one viewer with the fan-out's bridging rule and keep its packets.
fn publish_now_packets(
    out: &TickOutput,
    viewer: &mut ViewerState,
    history: &crate::plan::PublicationHistory,
) -> (Vec<PlannedPacket>, usize) {
    let previous = viewer.previous_entities().cloned();
    let mut index = TickPlanIndex::with_previous(out, previous.as_ref());
    let chain = history.chain(out.snapshot.dimension);
    if index.entity_base.is_none()
        && let Some((base, _, changed)) = TickPlanIndex::bridge(
            chain,
            out.snapshot.entities.revision(),
            std::iter::once(previous.as_ref()).flatten(),
        )
    {
        index = TickPlanIndex::with_chain(out, base, &changed);
    }
    let index = index.with_chain_info(chain.info()).with_reveals(out, None);
    let (packets, _, teams) = plan_packets(out, viewer, &index);
    (packets, teams)
}

/// Plan one viewer against a prepared index and keep its packets.
fn plan_packets(
    out: &TickOutput,
    viewer: &mut ViewerState,
    index: &TickPlanIndex,
) -> (Vec<PlannedPacket>, u64, usize) {
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
    let teams_before = packets.len();
    crate::faction_teams::plan_teams(&out.snapshot, index.faction_delta(), viewer, &mut packets);
    let team_frames = packets.len() - teams_before;
    (packets, scratch.visible.len() as u64, team_frames)
}

/// The packets as the client sees them: id and framed bytes in wire order. The
/// order is part of the contract (a respawn must precede the packets that
/// follow it), so nothing is sorted here.
fn wire(packets: &[PlannedPacket]) -> Vec<(i32, Vec<u8>)> {
    packets
        .iter()
        .filter(|packet| packet.chunk.is_none())
        .map(|packet| (packet.id, packet.data.to_vec()))
        .collect()
}

/// One store step with the same scene in both runs: everyone moves, one
/// resident is removed and a cow spawns beside the viewer.
fn step_scene(
    store: &mut EntityStore,
    slots: &[mc_core::entities::EntitySlot],
    out: &mut TickOutput,
    step: usize,
) {
    for (index, &slot) in slots.iter().enumerate().skip(1) {
        let x = 2.0 + (index % 32) as f64 * 2.0 + step as f64 * 0.5;
        store.set_pos(slot, [x, 64.0, 2.0 + (index / 32) as f64 * 2.0]);
    }
    if step == 1 {
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
    }
    let snapshot = Arc::get_mut(&mut out.snapshot).expect("sole snapshot owner");
    snapshot.tick += 1;
    snapshot.entities = store.views();
}

/// EDGE-PUMP-SKIP-FORCES-FULL-RESCAN fence: a viewer whose base predates the
/// published snapshot must receive exactly what a full rescan of that snapshot
/// delivers, frame for frame and in wire order — including a spawn, a removal
/// and moves inside the skipped outputs.
#[test]
fn a_skipped_window_delivers_what_a_full_rescan_delivers() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut history = crate::plan::PublicationHistory::new(8);
    let position = [32.0, 64.0, 32.0];
    let mut bridged = pen_viewer(1, position);
    let mut rescanned = pen_viewer(2, position);

    step_scene(&mut store, &slots, &mut out, 0);
    history.record(&out);
    for viewer in [&mut bridged, &mut rescanned] {
        let previous = viewer.previous_entities().cloned();
        let mut index = TickPlanIndex::with_previous(&out, previous.as_ref());
        index = index.with_reveals(&out, None);
        plan_packets(&out, viewer, &index);
    }

    // Two steps the pump drains but never publishes to these viewers.
    step_scene(&mut store, &slots, &mut out, 1);
    history.record(&out);
    step_scene(&mut store, &slots, &mut out, 2);
    history.record(&out);

    let chain = history.chain(out.snapshot.dimension);
    let previous = bridged.previous_entities().cloned();
    let (base, _, changed) = TickPlanIndex::bridge(
        chain,
        out.snapshot.entities.revision(),
        std::iter::once(previous.as_ref().expect("bridged baseline")),
    )
    .expect("the chain covers the viewer's base");
    assert_eq!(base.revision(), previous.as_ref().unwrap().revision());
    let bridged_index =
        TickPlanIndex::with_chain(&out, base, &changed).with_chain_info(chain.info());
    let (bridged_packets, _, _) = plan_packets(&out, &mut bridged, &bridged_index);

    let rescanned_index = TickPlanIndex::new(&out);
    let (rescanned_packets, _, _) = plan_packets(&out, &mut rescanned, &rescanned_index);

    assert_eq!(
        wire(&bridged_packets),
        wire(&rescanned_packets),
        "the bridged window must deliver exactly the rescan's frames, in order"
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
        !tracked(&bridged).contains(&EntityId(1_000)),
        "the resident removed inside the window must be untracked"
    );
}

/// EDGE-PUMP-SKIP-FORCES-FULL-RESCAN faction fence: the bridge must send the
/// net team state against the delivered base, not replay the steps.
///
/// The baseline really delivers faction 1 (the team and its member packet are
/// in that publish), faction 2 is published in a skipped step, and the final
/// snapshot leaves the resident in faction 2 while a faction-3 mob that spawned
/// and died inside the window leaves nothing behind. A replay would invent
/// team transitions the client never needed; the bridge must match the rescan.
#[test]
fn a_bridged_window_sends_the_net_faction_state() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut history = crate::plan::PublicationHistory::new(8);
    let position = [32.0, 64.0, 32.0];
    let mut bridged = pen_viewer(1, position);
    let mut rescanned = pen_viewer(2, position);

    // Baseline: the resident is already in faction 1, so the viewer's first
    // plan delivers team 1 and its member entry.
    store.set_faction(slots[1], 1);
    step_scene(&mut store, &slots, &mut out, 0);
    history.record(&out);
    let mut baseline_teams = 0;
    for viewer in [&mut bridged, &mut rescanned] {
        let previous = viewer.previous_entities().cloned();
        let mut index = TickPlanIndex::with_previous(&out, previous.as_ref());
        index = index.with_reveals(&out, None);
        let (_, _, teams) = plan_packets(&out, viewer, &index);
        baseline_teams += teams;
    }
    assert!(
        baseline_teams > 0,
        "the baseline publish must deliver faction 1, not just assume it"
    );

    // Skipped step: the resident moves to faction 2 and a faction-3 mob spawns.
    store.set_faction(slots[1], 2);
    let transient = store.spawn(
        EntityId(9_100),
        uuid::Uuid::from_u128(9_100),
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
    store.set_faction(transient, 3);
    step_scene(&mut store, &slots, &mut out, 1);
    history.record(&out);

    // Second skipped step: the transient mob dies and the resident stays where
    // it is, so the faction change is only visible in the first skipped step.
    store.remove(transient);
    step_scene(&mut store, &slots, &mut out, 2);
    history.record(&out);

    let chain = history.chain(out.snapshot.dimension);
    let previous = bridged.previous_entities().cloned();
    let (base, _, changed) = TickPlanIndex::bridge(
        chain,
        out.snapshot.entities.revision(),
        std::iter::once(previous.as_ref().expect("bridged baseline")),
    )
    .expect("the chain covers the viewer's base");
    let bridged_index = TickPlanIndex::with_chain(&out, base, &changed);
    let (bridged_packets, _, bridged_teams) = plan_packets(&out, &mut bridged, &bridged_index);
    let (rescanned_packets, _, rescanned_teams) =
        plan_packets(&out, &mut rescanned, &TickPlanIndex::new(&out));
    assert!(
        rescanned_teams > 0,
        "the final snapshot really moves the resident to faction 2: {rescanned_teams}"
    );
    assert_eq!(
        bridged_teams, rescanned_teams,
        "team frames must match the rescan's exactly: bridged={bridged_teams} rescanned={rescanned_teams}"
    );
    assert_eq!(
        wire(&bridged_packets),
        wire(&rescanned_packets),
        "the bridge must reconcile teams exactly like the rescan"
    );
    assert_eq!(
        format!("{:?}", bridged.teams),
        format!("{:?}", rescanned.teams),
        "the client's team state must end identical, with no team from the mob that died"
    );
}

/// EDGE-PUMP-SKIP-FORCES-FULL-RESCAN travel fence: the real travel handler
/// (`plan_player_changes` -> `plan_respawn`) must rebuild entity interest, so a
/// resident that never moved is delivered again after `A → B → A` instead of
/// being assumed still tracked. The production reset is the only thing under
/// test here: removing it makes this fail.
#[test]
fn a_travelled_viewer_is_never_bridged_from_its_pre_travel_base() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut history = crate::plan::PublicationHistory::new(8);
    let mut viewer = pen_viewer(1, [32.0, 64.0, 32.0]);

    step_scene(&mut store, &slots, &mut out, 0);
    history.record(&out);
    publish_now(&out, std::slice::from_mut(&mut viewer), &history, &[0]);
    let resident = EntityId(1_000 + 1);
    assert!(
        viewer.tracked_entities.contains_key(&resident),
        "the resident is tracked before the travel"
    );

    // The outbound and return legs of the real dimension-change handler.
    let travel = |viewer: &mut ViewerState, dimension| {
        crate::plan::plan_player_changes(
            &[mc_core::boundary::PlayerChange::DimensionChanged {
                teleport_id: 1,
                dimension,
                position: [32.0, 64.0, 32.0],
                yaw: 0.0,
                pitch: 0.0,
                game_mode: 0,
                portal_cooldown: 0,
            }],
            viewer,
        );
    };
    travel(&mut viewer, mc_core::dimensions::Dimension::Nether);
    travel(&mut viewer, mc_core::dimensions::Dimension::Overworld);
    // The client rebuilt its level; the pen it comes back to is unchanged for
    // the resident, so only a rebuilt interest can deliver it again.
    viewer
        .loaded_chunks
        .extend(pen_viewer(1, [32.0, 64.0, 32.0]).loaded_chunks);

    step_scene(&mut store, &slots, &mut out, 1);
    history.record(&out);
    step_scene(&mut store, &slots, &mut out, 2);
    history.record(&out);
    let previous = viewer.previous_entities().cloned();
    assert!(
        previous.is_none(),
        "the travel must have dropped the delivered base, not kept it for a bridge"
    );
    assert!(
        !viewer.tracked_entities.contains_key(&resident),
        "the travel cleared the client's entities"
    );
    let (packets, _) = publish_now_packets(&out, &mut viewer, &history);
    let spawned = packets
        .iter()
        .filter(|packet| packet.id == mc_protocol::packets::play::AddEntity::ID)
        .count();
    assert!(
        spawned > 0,
        "the rebuilt interest must spawn the visible residents again, not assume them"
    );
    assert!(
        viewer.tracked_entities.contains_key(&resident),
        "the unchanged resident must be tracked again after the travel"
    );
}

/// EDGE-PUMP-SKIP-FORCES-FULL-RESCAN held-set fence: a chain never overrides a
/// changed chunk set. A viewer that gained a held chunk rescans even when the
/// chain covers its base, exactly as it would without one.
#[test]
fn a_changed_held_set_rescans_even_when_the_chain_covers_the_base() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut history = crate::plan::PublicationHistory::new(8);
    let mut viewer = pen_viewer(1, [32.0, 64.0, 32.0]);

    step_scene(&mut store, &slots, &mut out, 0);
    history.record(&out);
    publish_now(&out, std::slice::from_mut(&mut viewer), &history, &[0]);

    step_scene(&mut store, &slots, &mut out, 1);
    history.record(&out);
    viewer.loaded_chunks.insert((4, 4), 0);
    let row = publish_now(&out, std::slice::from_mut(&mut viewer), &history, &[0]);
    assert_eq!(row.rescans, 1, "a rebuilt held set must rescan: {row:?}");
}

/// EDGE-PUMP-SKIP-FORCES-FULL-RESCAN cost: what a skipped publish costs per
/// viewer, split into the candidate scan, the bridge build and the framing.
#[test]
fn a_skipped_publish_costs_one_rescan_without_the_chain_and_none_with_it() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut history = crate::plan::PublicationHistory::new(32);
    let position = [32.0, 64.0, 32.0];
    let mut without_viewer = pen_viewer(1, position);
    let mut with_viewer = pen_viewer(2, position);

    step_scene(&mut store, &slots, &mut out, 0);
    history.record(&out);
    for viewer in [&mut without_viewer, &mut with_viewer] {
        let previous = viewer.previous_entities().cloned();
        let mut index = TickPlanIndex::with_previous(&out, previous.as_ref());
        index = index.with_reveals(&out, None);
        plan_packets(&out, viewer, &index);
    }
    let baseline_scan = without_viewer.entities_scanned;

    for round in 1..=3 {
        step_scene(&mut store, &slots, &mut out, round);
        history.record(&out);
    }
    let chain = history.chain(out.snapshot.dimension);
    let links = chain.len();

    // Scan alone: the pass a rescan pays and a bridge does not.
    let scan_us = {
        let mut scratch = PlanScratch::default();
        let started = std::time::Instant::now();
        super::entity_candidates::collect(
            &out.snapshot,
            &mut without_viewer,
            VIEW_DISTANCE,
            &mut scratch,
            usize::MAX,
            &TickPlanIndex::new(&out).spectators,
        );
        std::time::Instant::now()
            .duration_since(started)
            .as_micros()
    };

    // Without the chain: the same base, no bridge, full rescan.
    let previous = without_viewer.previous_entities().cloned();
    let without = {
        let mut index = TickPlanIndex::with_previous(&out, previous.as_ref());
        index = index.with_reveals(&out, None);
        let rescans = without_viewer.rescans;
        let scanned = without_viewer.entities_scanned;
        let started = std::time::Instant::now();
        let _ = plan_packets(&out, &mut without_viewer, &index);
        let plan_us = std::time::Instant::now()
            .duration_since(started)
            .as_micros();
        (
            plan_us,
            without_viewer.rescans - rescans,
            without_viewer.entities_scanned - scanned,
        )
    };

    // With the chain: one bridge build for every viewer that holds the base,
    // then the per-viewer framing walk.
    let previous = with_viewer.previous_entities().cloned();
    let with = {
        let started = std::time::Instant::now();
        let (base, _, changed) = TickPlanIndex::bridge(
            chain,
            out.snapshot.entities.revision(),
            std::iter::once(previous.as_ref().expect("baseline")),
        )
        .expect("chain covers the base");
        let mut index =
            TickPlanIndex::with_chain(&out, base, &changed).with_chain_info(chain.info());
        index = index.with_reveals(&out, None);
        let bridge_us = std::time::Instant::now()
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
            bridge_us,
            plan_us,
            with_viewer.rescans - rescans,
            with_viewer.entities_scanned - scanned,
        )
    };
    println!(
        "skipped publish: scan_us={scan_us} (a rescan pays it, a bridge does not); \
         without chain plan_us={} rescans={} scanned={}; with chain bridge_us={} plan_us={} \
         rescans={} scanned={}; changed ids across {links} links; first scan={baseline_scan}",
        without.0, without.1, without.2, with.0, with.1, with.2, with.3
    );
    assert_eq!(
        (without.1, with.2),
        (1, 0),
        "the chain must remove the rescan and nothing else: without={without:?} with={with:?}"
    );
    assert_eq!(
        with.3, 0,
        "the bridged publish scans no candidates: {with:?}"
    );
    assert!(without.2 > 0, "the rescan walks the square: {without:?}");
    assert!(links >= 3, "every drained step is recorded: {links}");
}

/// One store step that changes only an appearance (a renamed resident), with no moves: the
/// bridge must treat an appearance-only delta like a rescan of the same base.
fn rename_step(
    store: &mut EntityStore,
    slot: mc_core::entities::EntitySlot,
    name: &str,
    out: &mut TickOutput,
) {
    let mut appearance = (*store.row(slot).appearance).clone();
    appearance.custom_name = Some(name.to_owned());
    store.row_mut(slot).appearance = Arc::new(appearance);
    let snapshot = Arc::get_mut(&mut out.snapshot).expect("sole snapshot owner");
    snapshot.tick += 1;
    snapshot.entities = store.views();
}

/// EDGE-PUMP-SKIP-FORCES-FULL-RESCAN, appearance case: a skipped window whose only change is an
/// entity's appearance (a custom name here) must reach a bridged viewer exactly as a full rescan of
/// the same base delivers it — the name frame included, once, and nothing else.
#[test]
fn an_appearance_only_skipped_step_reaches_a_bridged_viewer() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    let mut history = crate::plan::PublicationHistory::new(8);

    advance(&mut store, &slots, &mut out, 0);
    history.record(&out);
    // The delivered base both viewers hold.
    let mut bridged = [pen_viewer(1, [32.0, 64.0, 32.0])];
    let mut reference = [pen_viewer(2, [32.0, 64.0, 32.0])];
    let (first, _) = with_plan_debug(|| publish_now_packets(&out, &mut bridged[0], &history));
    let (base_frames, _) =
        with_plan_debug(|| publish_now_packets(&out, &mut reference[0], &history));
    assert_eq!(wire(&first), wire(&base_frames), "both viewers start equal");

    // A step nobody plans (the pump coalesced over it) that only renames one resident.
    let named = slots[7];
    rename_step(&mut store, named, "Penny", &mut out);
    history.record(&out);
    let renamed_frames = {
        // The bridged viewer's frames for the skipped step (planned against the chain).
        let (packets, _) = with_plan_debug(|| publish_now_packets(&out, &mut bridged[0], &history));
        wire(&packets)
    };
    // The reference viewer holds the same base and is forced to rescan (its previous is not the
    // published snapshot's direct predecessor, so `with_previous` cannot go incremental).
    let rescanned_frames = {
        // No previous views at all: the index has no base, so this plan is a full rescan of the
        // published snapshot against the viewer's own tracked state.
        let index = TickPlanIndex::with_previous(&out, None).with_reveals(&out, None);
        assert!(
            index.entity_base.is_none(),
            "the oracle really walks the whole square"
        );
        let (packets, _, _) = plan_packets(&out, &mut reference[0], &index);
        wire(&packets)
    };
    assert_eq!(
        renamed_frames, rescanned_frames,
        "an appearance-only skipped step must deliver what a rescan of the same base delivers"
    );
    let names = renamed_frames
        .iter()
        .filter(|(id, _)| *id == mc_protocol::packets::play::ClientboundSetEntityData::ID)
        .count();
    assert!(names >= 1, "the renamed row's metadata frame is sent");
}

/// EDGE-PUMP-SKIP-FORCES-FULL-RESCAN, eviction case: the chain is bounded by the broadcast window,
/// so a viewer whose base fell out of it keeps the full rescan and says why (`no_history`), never a
/// silent bridge over steps the pump no longer holds.
#[test]
fn a_base_evicted_from_the_chain_rescans_with_the_no_history_reason() {
    let (mut store, slots) = pen_store();
    let mut out = output(pen_chunks(), vec![], ChangeSet::default());
    // A two-step bound: the third recorded step pushes the first one out.
    let mut history = crate::plan::PublicationHistory::new(2);
    let mut viewer = pen_viewer(1, [32.0, 64.0, 32.0]);

    advance(&mut store, &slots, &mut out, 0);
    history.record(&out);
    let (_, _) = with_plan_debug(|| publish_now_packets(&out, &mut viewer, &history));
    let evicted_base = viewer.previous_entities().cloned().expect("delivered base");

    // Three more steps the pump drained but nobody planned: the window (2) forgets the first.
    for round in 1..=3 {
        advance(&mut store, &slots, &mut out, round);
        history.record(&out);
    }
    assert!(
        history
            .chain(out.snapshot.dimension)
            .steps(evicted_base.revision(), out.snapshot.entities.revision())
            .is_none(),
        "the base has been evicted from the bounded chain"
    );
    let rescans_before = viewer.rescans;
    let no_history_before = viewer.rescans_no_history;
    let mut one = vec![viewer];
    let _ = with_plan_debug(|| publish_now(&out, &mut one, &history, &[0]));
    let viewer = one.remove(0);
    assert!(
        viewer.rescans > rescans_before,
        "an evicted base pays the full rescan"
    );
    assert_eq!(
        viewer.rescans_no_history,
        no_history_before + 1,
        "the rescan names its reason: no history"
    );
}
