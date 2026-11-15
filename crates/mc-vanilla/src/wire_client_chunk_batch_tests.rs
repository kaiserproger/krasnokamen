use super::*;

#[tokio::test]
async fn chunk_batch_ack_is_play_only_and_once_even_when_frame_is_replayed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (connected, accepted) = tokio::join!(TcpStream::connect(addr), listener.accept());
    let mut client = Client::from_stream(connected.unwrap()).unwrap();
    let (mut server, _) = accepted.unwrap();
    let finished = encode_frame(0x0b, &[1], Compression::Disabled).unwrap();
    server.write_all(&finished).await.unwrap();
    server.write_all(&finished).await.unwrap();
    // Configuration's same-numbered packet is not a chunk batch.
    client.read_frame().await.unwrap();
    client.play = true;
    let received = client.read_frame().await.unwrap();
    client.pending.push_back(received);
    client.read_frame().await.unwrap();
    drop(client);
    let mut replies = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), server.read_to_end(&mut replies))
        .await
        .unwrap()
        .unwrap();
    let mut replies = BytesMut::from(replies.as_slice());
    let ack = try_decode_frame(&mut replies, Compression::Disabled)
        .unwrap()
        .unwrap();
    assert_eq!(
        ack.id,
        mc_protocol::packets::play::ServerboundChunkBatchReceived::ID
    );
    assert_eq!(ack.body.as_ref(), &8.0f32.to_be_bytes());
    assert!(
        replies.is_empty(),
        "one network batch must produce exactly one ACK"
    );
}
