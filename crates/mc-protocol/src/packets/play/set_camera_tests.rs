use super::*;

#[test]
fn set_camera_uses_one_varint() {
    assert_eq!(ClientboundSetCamera::ID, 0x5f);
    for (entity_id, expected) in [
        (0, vec![0]),
        (300, vec![0xac, 0x02]),
        (-1, vec![0xff, 0xff, 0xff, 0xff, 0x0f]),
    ] {
        let packet = ClientboundSetCamera { entity_id };
        let mut bytes = Vec::new();
        packet.encode(&mut bytes).unwrap();
        assert_eq!(bytes, expected);
        let mut input = bytes.as_slice();
        assert_eq!(ClientboundSetCamera::decode(&mut input).unwrap(), packet);
        assert!(input.is_empty());
    }
}

#[test]
fn set_camera_refuses_incomplete_and_oversized_varints() {
    for bytes in [&[][..], &[0x80][..], &[0xff; 6][..]] {
        assert!(ClientboundSetCamera::decode(&mut &bytes[..]).is_err());
    }
}
