use super::*;
use bytes::BytesMut;

#[test]
fn sniffer_state_uses_serializer_35_and_roundtrips_all_vanilla_states() {
    for value in 0..=6 {
        let packet = ClientboundSetEntityData {
            entity_id: 28,
            values: vec![EntityDataValue::SnifferState { index: 17, value }],
        };
        let mut bytes = BytesMut::new();
        packet.encode(&mut bytes).unwrap();
        assert_eq!(&bytes[..], &[28, 17, 35, value, 255]);
        assert_eq!(
            ClientboundSetEntityData::decode(&mut bytes.freeze()).unwrap(),
            packet
        );
    }
    assert!(ClientboundSetEntityData::decode(&mut &[28, 17, 35, 7, 255][..]).is_err());
}
