use super::*;
use mc_protocol::codec::{ReadMc, WriteMc};
use mc_protocol::frame::{Compression, encode_frame, try_decode_frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn capture_login_uses_report_ids_and_release_specific_profile_layout() {
    for version in mc_data::protocol_versions::SUPPORTED {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let protocol = Protocol {
            version: version.protocol,
            hello: 17,
            success: 18,
            acknowledged: 19,
            compression: 20,
            disconnect: 21,
            ..Protocol::canonical()
        };
        let version = *version;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut input = bytes::BytesMut::new();
            let mut received = 0;
            while received < 2 {
                if let Some(frame) = try_decode_frame(&mut input, Compression::Disabled).unwrap() {
                    if received == 0 {
                        assert_eq!(frame.id, 0);
                        assert_eq!(frame.body.as_ref().read_varint().unwrap(), version.protocol);
                    } else {
                        assert_eq!(frame.id, 17, "report-driven hello id");
                        let mut body = frame.body.as_ref();
                        assert_eq!(body.read_string(16).unwrap(), "RegistryCapture");
                        body.read_uuid().unwrap();
                        assert!(body.is_empty());
                    }
                    received += 1;
                } else {
                    assert_ne!(stream.read_buf(&mut input).await.unwrap(), 0);
                }
            }
            let compression = Compression::Threshold(1);
            stream
                .write_all(&encode_frame(20, &[1], Compression::Disabled).unwrap())
                .await
                .unwrap();
            let mut body = Vec::new();
            body.write_uuid(uuid::Uuid::nil());
            body.write_string("RegistryCapture", 16).unwrap();
            body.write_varint(0);
            if version.protocol >= 776 {
                body.write_uuid(uuid::Uuid::nil());
            }
            stream
                .write_all(&encode_frame(18, &body, compression).unwrap())
                .await
                .unwrap();
            loop {
                if let Some(frame) = try_decode_frame(&mut input, compression).unwrap() {
                    assert_eq!(frame.id, 19);
                    assert!(frame.body.is_empty());
                    break;
                }
                assert_ne!(stream.read_buf(&mut input).await.unwrap(), 0);
            }
        });
        let mut client = Client::connect(addr).await.unwrap();
        protocol.login(&mut client, addr).await.unwrap();
        server.await.unwrap();
    }
}

#[test]
fn importing_protocol_ids_requires_all_named_reports() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("version.json"),
        br#"{"protocol_version":776}"#,
    )
    .unwrap();
    fs::create_dir_all(root.path().join("reports")).unwrap();
    fs::write(root.path().join("reports/packets.json"), b"{}").unwrap();
    assert!(
        Protocol::from_cache(root.path())
            .err()
            .unwrap()
            .to_string()
            .contains("select_known_packs")
    );
}
