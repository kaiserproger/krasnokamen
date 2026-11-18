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
