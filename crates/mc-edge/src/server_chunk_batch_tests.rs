use super::*;

#[tokio::test]
async fn chunk_batch_edge_ack_validates_before_mutation_and_is_connection_local() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let mut states = Vec::new();
    let mut receivers = Vec::new();
    for _ in 0..2 {
        let id = server.connections.joining_id();
        let (reliable, rx) = mpsc::channel(16);
        let (world, world_rx) = world_queue(32, 1 << 20);
        let handles = ConnectionHandles::new(reliable, world);
        let state = Arc::clone(&handles.chunk_batches);
        assert_eq!(state.lock().allowance(), 9);
        state.lock().sent(9);
        server.connections.register(id, handles, || 0).unwrap();
        states.push((id, state));
        receivers.push((rx, world_rx));
    }
    let (id, first) = &states[0];
    for body in [&[][..], &[0, 0, 0, 0, 0][..]] {
        assert!(
            server
                .handle_edge_frame(*id, ServerboundChunkBatchReceived::ID, body)
                .await
                .is_err()
        );
        assert_eq!(first.lock().allowance(), 0);
    }
    assert!(
        server
            .handle_edge_frame(
                *id,
                ServerboundChunkBatchReceived::ID,
                &f32::NAN.to_be_bytes()
            )
            .await
            .unwrap()
    );
    assert_eq!(first.lock().allowance(), 1);
    first.lock().sent(1);
    assert_eq!(
        first.lock().allowance(),
        0,
        "NaN selects minimum rate, not unlimited sending"
    );
    assert_eq!(
        states[1].1.lock().allowance(),
        0,
        "another connection still owes its ACK"
    );
    assert!(
        server
            .handle_edge_frame(
                *id,
                ServerboundChunkBatchReceived::ID,
                &f32::MAX.to_be_bytes()
            )
            .await
            .unwrap()
    );
    assert_eq!(first.lock().allowance(), 16);
    server.hub.shutdown();
}
