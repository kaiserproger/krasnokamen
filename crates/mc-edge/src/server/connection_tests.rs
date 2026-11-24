//! Vanilla Netty tunes every accepted child channel. These tests pin the
//! server-side socket options a client cannot observe directly.
use super::*;

/// An accepted game socket must report `TCP_NODELAY`: otherwise a small
/// clientbound frame queues behind an unacknowledged world batch for the
/// delayed-ACK timer (~40 ms) on every interaction burst.
#[tokio::test]
async fn accepted_game_socket_reports_nodelay() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let client = tokio::spawn(async move { TcpStream::connect(address).await.unwrap() });
    let (stream, _peer) = listener.accept().await.unwrap();
    assert!(
        !stream.nodelay().expect("nodelay is readable"),
        "std starts with Nagle on"
    );
    tune_game_socket(&stream).expect("accepted socket accepts TCP_NODELAY");
    assert!(stream.nodelay().expect("nodelay is readable"));
    // Keep the client connection alive until after the assertion: the option
    // belongs to the accepted stream, not to the connect half.
    let _client = client.await.unwrap();
}

/// REVIEW-wave2-11 minor-1: a cancelling session drops the play loop's keepalive
/// sender, so the writer's `recv()` answers `None`. That is the session ending on
/// its own — a normal stage — not an invalid reply, which would enter the error
/// rollup for every cancelled connection. A reply that is not the outstanding
/// challenge stays the protocol fault it always was.
#[test]
fn a_dropped_keepalive_sender_ends_the_writer_normally() {
    use super::connection::{KeepaliveReply, keepalive_reply};
    assert!(
        matches!(
            keepalive_reply(None, None),
            KeepaliveReply::End("keepalive-closed")
        ),
        "no challenge outstanding and the sender gone: a cancel, not a bad reply"
    );
    assert!(matches!(
        keepalive_reply(None, Some(3)),
        KeepaliveReply::End("keepalive-closed")
    ));
    assert!(matches!(
        keepalive_reply(Some(2), Some(3)),
        KeepaliveReply::End("keepalive-invalid-reply")
    ));
    assert!(matches!(
        keepalive_reply(Some(3), Some(3)),
        KeepaliveReply::Reply
    ));
    assert!(
        !ConnectionRegistry::close_cause_counts("keepalive-closed"),
        "a cancel is not an error"
    );
    assert!(ConnectionRegistry::close_cause_counts(
        "keepalive-invalid-reply"
    ));
}
