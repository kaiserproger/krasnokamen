use crate::fanout::{Priority, publish};
use crate::plan::ViewerState;
use crate::server::world_queue;
use mc_core::boundary::{
    ChangeSet, ConnectionId, EntityId, SnapshotChunk, TickOutput, WorldSnapshot,
};
use mc_protocol::frame::{Compression, try_decode_frame};
use mc_protocol::packets::{
    Packet,
    play::{ClientboundChunkBatchFinished, ClientboundChunkBatchStart, LevelChunkWithLight},
};
use std::sync::Arc;

fn world() -> TickOutput {
    let mut chunks = Vec::new();
    for x in -3..=3 {
        for z in -3..=3 {
            let pos = mc_world::chunk::ChunkPos { x, z };
            let mut chunk = mc_world::chunk::Chunk::empty(
                pos,
                mc_world::BlockStateId(0),
                mc_data::Identifier::parse("minecraft:plains").unwrap(),
            );
            chunk.section_lights.fill(mc_world::chunk::SectionLight {
                block: Some(mc_world::chunk::LightSection::uniform(0)),
                sky: Some(mc_world::chunk::LightSection::uniform(15)),
            });
            chunks.push(SnapshotChunk {
                pos,
                revision: 1,
                chunk: Arc::new(chunk),
            });
        }
    }
    TickOutput {
        tick: 1,
        snapshot: Arc::new(WorldSnapshot {
            tick: 1,
            chunks: Arc::new(chunks),
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet::default()),
        times: Default::default(),
    }
}

#[test]
fn chunk_batch_fanout_frames_counts_and_queue_admission_paces_no_ack_client() {
    let output = world();
    let mut viewer = ViewerState::new(ConnectionId(1), EntityId(1), [0.5, 64.0, 0.5]);
    let packets = publish(&output, [&mut viewer].into_iter(), 3);
    let start = packets
        .iter()
        .position(|packet| matches!(packet.priority, Priority::ChunkBatch(_)))
        .expect("chunk batch start");
    assert_eq!(packets[start].priority, Priority::ChunkBatch(9));
    let frames: Vec<_> = packets[start..start + 11]
        .iter()
        .map(|packet| packet.data.clone())
        .collect();
    let (tx, mut rx) = world_queue(11, 1 << 20);
    tx.try_send_chunk_batch(&frames, 9, &viewer.chunk_batches)
        .unwrap();
    assert_eq!(tx.capacity(), 0, "all eleven wire frame slots charged");
    assert_eq!(
        viewer.chunk_batches.lock().allowance(),
        0,
        "never-ACK client stops after one batch"
    );
    let first = rx.try_recv().unwrap();
    assert_eq!(first.as_ptr(), frames[0].as_ptr());
    let mut data = bytes::BytesMut::from(first.as_ref());
    for expected in &frames[1..] {
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame.as_ptr(), expected.as_ptr(), "reuse encoded buffers");
        data.extend_from_slice(&frame);
    }
    assert_eq!(tx.capacity(), 11);
    assert_eq!(
        try_decode_frame(&mut data, Compression::Threshold(256))
            .unwrap()
            .unwrap()
            .id,
        ClientboundChunkBatchStart::ID
    );
    for _ in 0..9 {
        assert_eq!(
            try_decode_frame(&mut data, Compression::Threshold(256))
                .unwrap()
                .unwrap()
                .id,
            LevelChunkWithLight::ID
        );
    }
    let finished = try_decode_frame(&mut data, Compression::Threshold(256))
        .unwrap()
        .unwrap();
    assert_eq!(finished.id, ClientboundChunkBatchFinished::ID);
    assert_eq!(
        ClientboundChunkBatchFinished::decode(&mut finished.body.as_ref())
            .unwrap()
            .batch_size,
        9
    );
    assert!(data.is_empty());
    let blocked = publish(&output, [&mut viewer].into_iter(), 3);
    assert!(
        !blocked
            .iter()
            .any(|packet| matches!(packet.priority, Priority::ChunkBatch(_)))
    );
    viewer.chunk_batches.lock().acknowledge(4.0);
    assert_eq!(viewer.chunk_batches.lock().allowance(), 4);
    let resumed = publish(&output, [&mut viewer].into_iter(), 3);
    assert!(
        resumed
            .iter()
            .any(|packet| packet.priority == Priority::ChunkBatch(4))
    );
}

#[test]
fn chunk_batch_queue_race_is_all_or_nothing_and_dimension_fence_returns_debt() {
    let state = batch_state();
    let frames = vec![bytes::Bytes::from_static(b"a"); 3];
    let (tx, mut rx) = world_queue(3, 2);
    assert_eq!(state.lock().allowance(), 9);
    tx.try_send(bytes::Bytes::from_static(b"busy")).unwrap();
    assert!(tx.try_send_chunk_batch(&frames, 1, &state).is_err());
    assert_eq!(tx.capacity(), 2, "failed reservation leaked no slots");
    assert_eq!(rx.try_recv().unwrap().as_ref(), b"busy");
    tx.try_send_chunk_batch(&frames, 1, &state).unwrap();
    assert_eq!(tx.queued_bytes(), 3, "soft bytes count complete group");
    assert_eq!(state.lock().allowance(), 0);
    tx.reset();
    assert!(rx.try_recv().is_err());
    assert_eq!(tx.queued_bytes(), 0);
    assert_eq!(state.lock().allowance(), 9, "discarded batch cannot ACK");
}

#[test]
fn chunk_batch_rate_and_existing_publication_credit_take_the_minimum() {
    let output = world();
    let prelude = TickOutput {
        tick: 0,
        snapshot: Arc::default(),
        changes: Arc::default(),
        times: Default::default(),
    };
    for (rate, slots, expected) in [(64.0, 100, 16), (4.0, 100, 4), (64.0, 7, 5), (64.0, 2, 0)] {
        let mut viewer = ViewerState::new(ConnectionId(1), EntityId(1), [0.5, 64.0, 0.5]);
        // Account for initial border/weather separately; this publish's slots
        // are now available to the chunk batch itself.
        publish(&prelude, [&mut viewer].into_iter(), 3);
        viewer.chunk_batches.lock().acknowledge(rate);
        viewer.world_credit = slots;
        let packets = publish(&output, [&mut viewer].into_iter(), 3);
        let chunks: usize = packets
            .iter()
            .filter_map(|packet| match packet.priority {
                Priority::ChunkBatch(count) => Some(count),
                _ => None,
            })
            .sum();
        assert_eq!(chunks, expected);
        assert!(
            packets
                .iter()
                .filter(|packet| packet.priority != Priority::Reliable)
                .count()
                <= slots
        );
    }
}

fn batch_state() -> Arc<parking_lot::Mutex<crate::plan::chunk_batch::ChunkBatchState>> {
    Arc::default()
}

#[test]
fn chunk_batch_admission_rechecks_last_ack_between_plan_and_send() {
    let state = batch_state();
    state.lock().acknowledge(64.0);
    assert_eq!(state.lock().allowance(), 16);
    let large = vec![bytes::Bytes::from_static(b"x"); 18];
    let (tx, mut rx) = world_queue(36, 1 << 20);
    tx.try_send_chunk_batch(&large, 16, &state).unwrap();
    while rx.try_recv().is_ok() {}
    assert_eq!(
        state.lock().allowance(),
        16,
        "the next plan selected sixteen"
    );
    std::thread::scope(|scope| {
        scope
            .spawn(|| state.lock().acknowledge(0.01))
            .join()
            .unwrap();
    });
    assert!(
        tx.try_send_chunk_batch(&large, 16, &state).is_err(),
        "last ACK reset quota to one; stale sixteen-chunk plan cannot enter"
    );
    assert!(rx.is_empty());
    assert_eq!(tx.capacity(), 36);
    assert_eq!(state.lock().allowance(), 1);
    let small = vec![bytes::Bytes::from_static(b"x"); 3];
    tx.try_send_chunk_batch(&small, 1, &state).unwrap();
}

#[test]
fn chunk_batch_fractional_fence_refunds_only_unacknowledged_group() {
    let state = batch_state();
    let frames = vec![bytes::Bytes::from_static(b"x"); 3];
    let (tx, mut rx) = world_queue(12, 1 << 20);
    state.lock().acknowledge(0.01);
    assert_eq!(state.lock().allowance(), 1);
    tx.try_send_chunk_batch(&frames, 1, &state).unwrap();
    assert_eq!(state.lock().allowance(), 0);
    tx.reset();
    assert!(rx.try_recv().is_err());
    assert_eq!(
        state.lock().allowance(),
        1,
        "unwritten chunk needs no 100-tick refill"
    );
    tx.try_send_chunk_batch(&frames, 1, &state).unwrap();
    state.lock().acknowledge(4.0);
    tx.reset();
    assert_eq!(state.lock().allowance(), 4);
    tx.try_send_chunk_batch(&frames, 1, &state).unwrap();
    assert!(!state.lock().can_send(4));
    rx.try_recv().unwrap(); // Discard old acknowledged group; take new live group.
    assert!(
        !state.lock().can_send(4),
        "old ACKed work cannot mint a second refund"
    );
    assert_eq!(rx.next_in_batch().unwrap().as_ptr(), frames[1].as_ptr());
    assert_eq!(rx.next_in_batch().unwrap().as_ptr(), frames[2].as_ptr());
    assert!(rx.next_in_batch().is_none());
}

#[test]
fn chunk_batch_cold_cache_actual_bytes_admit_only_one_oversize_chunk() {
    let prelude = TickOutput {
        tick: 0,
        snapshot: Arc::default(),
        changes: Arc::default(),
        times: Default::default(),
    };
    let mut viewer = ViewerState::new(ConnectionId(1), EntityId(1), [0.5, 64.0, 0.5]);
    publish(&prelude, [&mut viewer].into_iter(), 3);
    viewer.chunk_batches.lock().acknowledge(64.0);
    viewer.world_bytes = 1;
    let packets = publish(&world(), [&mut viewer].into_iter(), 3);
    let batches: Vec<_> = packets
        .iter()
        .filter_map(|packet| match packet.priority {
            Priority::ChunkBatch(chunks) => Some(chunks),
            _ => None,
        })
        .collect();
    assert_eq!(
        batches,
        [1],
        "post-fill bytes, not zero-byte placeholders, bound the batch"
    );
    assert_eq!(
        packets
            .iter()
            .filter(|p| p.priority != Priority::Reliable)
            .count(),
        3
    );
}
