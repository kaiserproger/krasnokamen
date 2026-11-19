use super::*;
use crate::fanout::{AddressedPacket, Priority};

fn packet(connection: ConnectionId, data: &'static [u8], priority: Priority) -> AddressedPacket {
    AddressedPacket {
        connection,
        data: bytes::Bytes::from_static(data),
        priority,
    }
}

#[tokio::test]
async fn world_refusal_stops_camera_suffix_even_when_writer_frees_capacity() {
    let root = tempfile::tempdir().unwrap();
    let server = roster_seed_tests::test_server(root.path());
    for (missing, dependent) in [
        (b"spawn".as_slice(), b"bind".as_slice()),
        (b"release".as_slice(), b"remove".as_slice()),
    ] {
        let a = ConnectionId(1);
        let b = ConnectionId(2);
        let (reliable, mut reliable_rx) = mpsc::channel(4);
        let (world, mut world_rx) = world_queue(3, WORLD_QUEUE_BYTES);
        let handles = ConnectionHandles::new(reliable, world, crate::plan::CHUNKS_PER_TICK);
        assert_eq!(handles.chunk_batches.lock().allowance(), 9);
        let pacing = Arc::clone(&handles.chunk_batches);
        let (other_reliable, _other_reliable_rx) = mpsc::channel(4);
        let (other_world, mut other_world_rx) = world_queue(3, WORLD_QUEUE_BYTES);
        let mut connections = HashMap::from([
            (a, handles),
            (
                b,
                ConnectionHandles::new(other_reliable, other_world, crate::plan::CHUNKS_PER_TICK),
            ),
        ]);
        let mut viewers = HashMap::from([
            (a, crate::plan::ViewerState::new(a, EntityId(1), [0.0; 3])),
            (b, crate::plan::ViewerState::new(b, EntityId(2), [0.0; 3])),
        ]);
        let packets = [
            packet(a, b"prefix1", Priority::BestEffort),
            packet(a, b"prefix2", Priority::BestEffort),
            packet(a, b"prefix3", Priority::BestEffort),
            packet(a, missing, Priority::BestEffort),
            packet(a, dependent, Priority::BestEffort),
            packet(a, b"start", Priority::ChunkBatch(1)),
            packet(a, b"chunk", Priority::BestEffort),
            packet(a, b"finish", Priority::BestEffort),
            packet(b, b"other viewer", Priority::BestEffort),
            packet(a, b"reliable", Priority::Reliable),
        ];
        let mut drained = false;
        // Exact producer/consumer interleaving: the first missing frame was
        // refused, then the writer frees every slot before its dependent frame.
        let packets = packets.into_iter().inspect(|packet| {
            if packet.data.as_ref() == dependent {
                for prefix in [b"prefix1", b"prefix2", b"prefix3"] {
                    assert_eq!(world_rx.try_recv().unwrap().as_ref(), prefix);
                }
                drained = true;
            }
        });
        route_planned_packets(&server, &mut viewers, &mut connections, packets);
        assert!(drained);
        assert!(
            world_rx.try_recv().is_err(),
            "dependent frame escaped after {missing:?} refusal"
        );
        assert_eq!(
            pacing.lock().allowance(),
            9,
            "skipped complete chunk group creates no ACK debt"
        );
        assert_eq!(other_world_rx.try_recv().unwrap().as_ref(), b"other viewer");
        assert_eq!(reliable_rx.try_recv().unwrap().as_ref(), b"reliable");
        route_planned_packets(
            &server,
            &mut viewers,
            &mut connections,
            [packet(a, b"next publication", Priority::BestEffort)],
        );
        assert_eq!(world_rx.try_recv().unwrap().as_ref(), b"next publication");
    }
    server.hub.shutdown();
}

#[tokio::test]
async fn refused_atomic_chunk_group_stops_later_camera_frame_without_ack_debt() {
    let root = tempfile::tempdir().unwrap();
    let server = roster_seed_tests::test_server(root.path());
    let id = ConnectionId(1);
    let (reliable, _reliable_rx) = mpsc::channel(4);
    let (world, mut world_rx) = world_queue(3, WORLD_QUEUE_BYTES);
    let handles = ConnectionHandles::new(reliable, world, crate::plan::CHUNKS_PER_TICK);
    let pacing = Arc::clone(&handles.chunk_batches);
    assert_eq!(pacing.lock().allowance(), 9);
    let mut connections = HashMap::from([(id, handles)]);
    let mut viewers =
        HashMap::from([(id, crate::plan::ViewerState::new(id, EntityId(1), [0.0; 3]))]);
    let packets = [
        packet(id, b"prefix", Priority::BestEffort),
        packet(id, b"start", Priority::ChunkBatch(1)),
        packet(id, b"chunk", Priority::BestEffort),
        packet(id, b"finish", Priority::BestEffort),
        packet(id, b"camera", Priority::BestEffort),
    ]
    .into_iter()
    .inspect(|packet| {
        if packet.data.as_ref() == b"camera" {
            assert_eq!(world_rx.try_recv().unwrap().as_ref(), b"prefix");
        }
    });
    route_planned_packets(&server, &mut viewers, &mut connections, packets);
    assert!(
        world_rx.try_recv().is_err(),
        "camera escaped after atomic chunk refusal"
    );
    assert_eq!(pacing.lock().allowance(), 9);
    assert_eq!(connections[&id].world_tx.capacity(), 3);
    server.hub.shutdown();
}
