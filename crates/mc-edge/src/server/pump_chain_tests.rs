//! EDGE-PUMP-SKIP-FORCES-FULL-RESCAN at the pump: the real drain
//! ([`next_publication`]) and the real publish path, not a hand-built index.
//!
//! A publish the pump skips must not turn every later publish into a full
//! rescan of every viewer, and history a broadcast `Lagged` dropped must keep
//! that fallback.
use super::*;
use mc_core::boundary::{ChangeSet, EntityAppearance, EntityId, TickOutput, WorldSnapshot};
use mc_core::dimensions::Dimension;
use mc_core::entities::{ColdFields, EntityKind, EntityStore};

fn cow_type_id() -> u32 {
    mc_data::entity_types::entity_type_contract_26_1_2_by_name("minecraft:cow")
        .expect("cow contract")
        .protocol_id
}

fn cowboy(type_id: u32) -> EntityKind {
    EntityKind::Mob { type_id }
}

fn cold(pos: [f64; 3]) -> ColdFields {
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
        movement: mc_core::blueprint::MovementClass::Walker,
        half_width: 0.45,
        height: 1.4,
        speed: 0.1,
        attack_damage: 1.0,
        sight_range: 16.0,
        attack_reach: 1.0,
    }
}

fn output(tick: u64, entities: mc_core::boundary::EntityViews) -> TickOutput {
    TickOutput {
        tick,
        snapshot: Arc::new(WorldSnapshot {
            tick,
            entities,
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet::default()),
        times: Default::default(),
    }
}

/// A registered viewer in the pen's first chunk, holding no baseline yet.
async fn viewer(
    server: &Arc<EdgeServer>,
) -> (
    ConnectionId,
    HashMap<ConnectionId, crate::plan::ViewerState>,
    HashMap<ConnectionId, ConnectionHandles>,
) {
    let id = server.connections.joining_id();
    let (reliable_tx, reliable_rx) = mpsc::channel(crate::fanout::RELIABLE_QUEUE_BOUND);
    let (world_tx, world_rx) = world_queue(8, 1 << 20);
    server
        .connections
        .register(
            id,
            ConnectionHandles::new(reliable_tx, world_tx, crate::plan::CHUNKS_PER_TICK),
            || 0,
        )
        .unwrap();
    // The receivers outlive the publish: a dropped queue would close the
    // connection instead of delivering the plan.
    std::mem::forget((reliable_rx, world_rx));
    let (connections, _) = server.connections.routing_view();
    let mut state = crate::plan::ViewerState::new(id, EntityId(1), [8.0, 64.0, 8.0]);
    state.dimension = Dimension::Overworld;
    state.loaded_chunks.insert((0, 0), 0);
    (id, HashMap::from([(id, state)]), connections)
}

fn pool() -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_skipped_publish_stays_contiguous_through_the_chain() {
    let (server, _world) = crate::server::admin_tests::fixture();
    let server = Arc::new(server);
    let (_id, mut viewers, mut connections) = viewer(&server).await;
    let pool = pool();
    let mut cache = crate::plan::ChunkPacketCache::new();
    let mut chain = crate::plan::PublicationChain::new(server.hub.broadcast_capacity());

    // The pen: two cows in the viewer's chunk.
    let mut store = EntityStore::default();
    let type_id = cow_type_id();
    let a = store.spawn(
        EntityId(1_000),
        uuid::Uuid::from_u128(1_000),
        cowboy(type_id),
        cold([8.0, 64.0, 8.0]),
    );
    let b = store.spawn(
        EntityId(1_001),
        uuid::Uuid::from_u128(1_001),
        cowboy(type_id),
        cold([12.0, 64.0, 8.0]),
    );

    // Publish A: the viewer learns the pen.
    let first = output(1, store.views());
    chain.record(&first.snapshot.entities);
    publish_and_route_chained(
        &server,
        &pool,
        &mut viewers,
        &first,
        &mut cache,
        &mut connections,
        &chain,
    )
    .await;
    let id = viewers.keys().copied().next().unwrap();
    let rescans = viewers[&id].rescans;
    assert_eq!(rescans, 1, "the first publish has no baseline");
    assert_eq!(viewers[&id].tracked_entities.len(), 2, "both cows tracked");

    // B moves one cow, spawns a third and removes the second: it is drained
    // but the pump skips it.
    store.set_pos(a, [20.0, 64.0, 8.0]);
    store.spawn(
        EntityId(1_002),
        uuid::Uuid::from_u128(1_002),
        cowboy(type_id),
        cold([4.0, 64.0, 4.0]),
    );
    store.remove(b);
    let second = output(2, store.views());
    chain.record(&second.snapshot.entities);

    // C moves the cow again: the next publication the pump actually publishes.
    store.set_pos(a, [24.0, 64.0, 8.0]);
    let third = output(3, store.views());
    chain.record(&third.snapshot.entities);
    assert_eq!(chain.len(), 2, "the skipped step stays in the chain");

    publish_and_route_chained(
        &server,
        &pool,
        &mut viewers,
        &third,
        &mut cache,
        &mut connections,
        &chain,
    )
    .await;
    assert_eq!(
        viewers[&id].rescans, rescans,
        "one skipped publish must not rescan the viewer"
    );
    assert_eq!(
        viewers[&id].rescans_no_history, 0,
        "the chain held the step, so the base is not missing history"
    );
    let tracked = &viewers[&id].tracked_entities;
    assert!(
        tracked.contains_key(&EntityId(1_002)),
        "a cow spawned inside the skipped window must be tracked"
    );
    assert!(
        !tracked.contains_key(&EntityId(1_001)),
        "a cow removed inside the skipped window must be untracked"
    );
    server.hub.shutdown();
}

#[tokio::test]
async fn a_lagged_publication_keeps_the_full_rescan_fallback() {
    let (server, _world) = crate::server::admin_tests::fixture();
    let server = Arc::new(server);
    let (_id, mut viewers, mut connections) = viewer(&server).await;
    let pool = pool();
    let mut cache = crate::plan::ChunkPacketCache::new();
    let mut store = EntityStore::default();
    let only = store.spawn(
        EntityId(1_000),
        uuid::Uuid::from_u128(1_000),
        cowboy(cow_type_id()),
        cold([8.0, 64.0, 8.0]),
    );
    let first = output(1, store.views());
    let mut chain = crate::plan::PublicationChain::new(1);
    chain.record(&first.snapshot.entities);
    publish_and_route_chained(
        &server,
        &pool,
        &mut viewers,
        &first,
        &mut cache,
        &mut connections,
        &chain,
    )
    .await;
    let id = viewers.keys().copied().next().unwrap();
    let rescans = viewers[&id].rescans;

    // Two outputs into a one-slot broadcast: the receiver sees `Lagged` and
    // the drained output can no longer be bridged from the viewer's base.
    let (tx, mut rx) = broadcast::channel(1);
    store.set_pos(only, [10.0, 64.0, 8.0]);
    tx.send(output(2, store.views())).unwrap();
    store.set_pos(only, [12.0, 64.0, 8.0]);
    tx.send(output(3, store.views())).unwrap();
    let next = next_publication(&server, &mut rx, &mut viewers, &cache, &mut chain)
        .await
        .expect("the newer output is retained");
    assert_eq!(next.tick, 3, "the dropped output is never published");
    assert!(
        chain
            .steps(
                first.snapshot.entities.revision(),
                next.snapshot.entities.revision()
            )
            .is_none(),
        "a Lagged gap must not be bridged"
    );

    publish_and_route_chained(
        &server,
        &pool,
        &mut viewers,
        &next,
        &mut cache,
        &mut connections,
        &chain,
    )
    .await;
    assert_eq!(
        viewers[&id].rescans,
        rescans + 1,
        "a lost publication must keep the full rescan"
    );
    assert_eq!(
        viewers[&id].rescans_gap, 1,
        "the rescan names the Lagged gap"
    );
    assert_eq!(viewers[&id].rescans_no_history, 0);
    server.hub.shutdown();
}
