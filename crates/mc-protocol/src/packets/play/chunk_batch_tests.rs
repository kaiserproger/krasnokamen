use super::*;

#[test]
fn chunk_batch_layouts_match_all_three_local_reports() {
    // `content import --cache data/vanilla` layout: the canonical cache is
    // data/vanilla, each other window release data/vanilla-<release>.
    for version in ["26.1.2", "26.2", "26.3"] {
        let cache = if version == "26.3" {
            "vanilla".to_owned()
        } else {
            format!("vanilla-{version}")
        };
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data")
            .join(cache)
            .join("reports/packets.json");
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            report["play"]["clientbound"]["minecraft:chunk_batch_start"]["protocol_id"],
            ClientboundChunkBatchStart::ID
        );
        assert_eq!(
            report["play"]["clientbound"]["minecraft:chunk_batch_finished"]["protocol_id"],
            ClientboundChunkBatchFinished::ID
        );
        let mut bytes = Vec::new();
        ClientboundChunkBatchStart.encode(&mut bytes).unwrap();
        assert!(bytes.is_empty());
        assert_eq!(
            ClientboundChunkBatchStart::decode(&mut &bytes[..]).unwrap(),
            ClientboundChunkBatchStart
        );
        ClientboundChunkBatchFinished { batch_size: 300 }
            .encode(&mut bytes)
            .unwrap();
        assert_eq!(bytes, [0xac, 0x02]);
        let mut input = bytes.as_slice();
        assert_eq!(
            ClientboundChunkBatchFinished::decode(&mut input)
                .unwrap()
                .batch_size,
            300
        );
        assert!(input.is_empty());
    }
    assert!(ClientboundChunkBatchFinished::decode(&mut &[0x80][..]).is_err());
}
