use super::*;

fn frame(len: usize) -> bytes::Bytes {
    bytes::Bytes::from(vec![7u8; len])
}

#[tokio::test]
async fn queued_bytes_follow_send_and_receive() {
    let (sender, mut receiver) = world_queue(4, 100);
    assert_eq!((sender.capacity(), sender.free_bytes()), (4, 100));
    sender.try_send(frame(30)).unwrap();
    sender.try_send(frame(50)).unwrap();
    assert_eq!(sender.queued_bytes(), 80);
    assert_eq!((sender.capacity(), sender.free_bytes()), (2, 20));
    assert_eq!(receiver.try_recv().unwrap().len(), 30);
    assert_eq!((sender.queued_bytes(), sender.free_bytes()), (50, 50));
    assert_eq!(receiver.recv().await.unwrap().len(), 50);
    assert_eq!((sender.queued_bytes(), receiver.len()), (0, 0));
    assert!(receiver.is_empty());
}

/// The byte bound is planning credit: an oversized frame is still queued
/// (the planner admits one group while any byte credit remains), and the
/// free count saturates instead of wrapping.
#[test]
fn byte_bound_is_soft_and_saturates() {
    let (sender, mut receiver) = world_queue(4, 100);
    sender.try_send(frame(150)).unwrap();
    assert_eq!(sender.free_bytes(), 0);
    receiver.try_recv().unwrap();
    assert_eq!(sender.free_bytes(), 100);
}

/// A refused frame (no slot) leaves the byte count as it was.
#[test]
fn full_frame_slots_refuse_without_counting() {
    let (sender, mut receiver) = world_queue(1, 100);
    sender.try_send(frame(10)).unwrap();
    assert!(matches!(
        sender.try_send(frame(20)),
        Err(TrySendError::Full(_))
    ));
    assert_eq!(sender.queued_bytes(), 10);
    receiver.try_recv().unwrap();
    assert_eq!(sender.queued_bytes(), 0);
    drop(receiver);
    assert!(sender.is_closed());
    assert!(matches!(
        sender.try_send(frame(5)),
        Err(TrySendError::Closed(_))
    ));
    assert_eq!(sender.queued_bytes(), 0);
}

/// A recorder gets the recorder bounds with its configured bytes; anyone
/// else gets the player bounds.
#[test]
fn recorder_and_player_queues_take_their_own_bounds() {
    let config = crate::recorder::RecorderSection {
        world_queue_bytes: 4096,
        ..Default::default()
    };
    let (recorder, _recorder_rx) = world_queue_for(&config, true);
    assert_eq!(
        (recorder.max_capacity(), recorder.free_bytes()),
        (RECORDER_WORLD_QUEUE_FRAMES, 4096)
    );
    let (player, _player_rx) = world_queue_for(&config, false);
    assert_eq!(
        (player.max_capacity(), player.free_bytes()),
        (WORLD_QUEUE_FRAMES, WORLD_QUEUE_BYTES)
    );
}
