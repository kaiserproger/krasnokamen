use super::*;

fn chat(message: &str) -> PlayerChange {
    PlayerChange::Chat {
        message: message.into(),
    }
}

#[test]
fn reliable_handoff_coalesces_fifo_batches_and_keeps_disconnect_last() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let outbox = ReliableOutbox(parking_lot::Mutex::new(OutboxReceiver {
        receiver,
        pending: None,
    }));
    let first = ConnectionId(1);
    let second = ConnectionId(2);
    sender
        .send((
            1,
            vec![(first, vec![chat("one")]), (second, vec![chat("other")])],
        ))
        .unwrap();
    sender
        .send((
            2,
            vec![(
                first,
                vec![
                    chat("two"),
                    PlayerChange::Disconnect {
                        reason: "sealed".into(),
                    },
                ],
            )],
        ))
        .unwrap();
    sender
        .send((3, vec![(first, vec![chat("not delivered")])]))
        .unwrap();
    // A stopped authority leaves its terminal batch available to the edge.
    drop(sender);
    let queues: HashMap<_, _> = outbox.drain(u64::MAX).into_iter().collect();
    assert_eq!(
        queues[&first],
        vec![
            chat("one"),
            chat("two"),
            PlayerChange::Disconnect {
                reason: "sealed".into()
            }
        ]
    );
    assert_eq!(queues[&second], vec![chat("other")]);
    assert!(outbox.drain(u64::MAX).is_empty());
}

#[test]
fn draining_does_not_consume_later_batches() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let outbox = ReliableOutbox(parking_lot::Mutex::new(OutboxReceiver {
        receiver,
        pending: None,
    }));
    let connection = ConnectionId(1);
    sender
        .send((1, vec![(connection, vec![chat("first")])]))
        .unwrap();
    assert_eq!(outbox.drain(1), vec![(connection, vec![chat("first")])]);
    sender
        .send((3, vec![(connection, vec![chat("later")])]))
        .unwrap();
    assert!(outbox.drain(2).is_empty());
    assert_eq!(outbox.drain(3), vec![(connection, vec![chat("later")])]);
}

#[test]
fn snapshot_watermark_drains_covered_batches_but_not_future_batches() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let outbox = ReliableOutbox(parking_lot::Mutex::new(OutboxReceiver {
        receiver,
        pending: None,
    }));
    let connection = ConnectionId(1);
    for (tick, text) in [(1, "one"), (2, "two"), (3, "three")] {
        sender
            .send((tick, vec![(connection, vec![chat(text)])]))
            .unwrap();
    }
    assert_eq!(
        outbox.drain(2),
        vec![(connection, vec![chat("one"), chat("two")])]
    );
    assert_eq!(outbox.drain(2), Vec::new());
    assert_eq!(outbox.drain(3), vec![(connection, vec![chat("three")])]);
}
