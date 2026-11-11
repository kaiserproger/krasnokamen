use super::*;

#[test]
fn painting_variant_holder_wire_roundtrip() {
    // Local 26.1.2 EntityDataSerializers + HolderCodec javap: serializer 34,
    // registry references encode id + 1; zero introduces an inline holder.
    let packet = ClientboundSetEntityData {
        entity_id: 17,
        values: vec![EntityDataValue::PaintingVariant { index: 9, value: 4 }],
    };
    let mut bytes = Vec::new();
    packet.encode(&mut bytes).unwrap();
    assert_eq!(bytes, [17, 9, 34, 5, 255]);
    assert_eq!(
        ClientboundSetEntityData::decode(&mut bytes.as_slice()).unwrap(),
        packet
    );
    assert!(ClientboundSetEntityData::decode(&mut [17, 9, 34, 0, 255].as_slice()).is_err());
}

#[test]
fn chicken_variant_holder_wire_roundtrip() {
    let packet = ClientboundSetEntityData {
        entity_id: 17,
        values: vec![EntityDataValue::ChickenVariant {
            index: crate::packets::play::CHICKEN_VARIANT_DATA_INDEX,
            value: 2,
        }],
    };
    let mut bytes = Vec::new();
    packet.encode(&mut bytes).unwrap();
    assert_eq!(bytes, [17, 18, 30, 2, 255]);
    assert_eq!(
        ClientboundSetEntityData::decode(&mut bytes.as_slice()).unwrap(),
        packet
    );
}

#[test]
fn climate_variant_holders_use_their_own_serializer_and_direct_registry_id() {
    // 26.1.2/26.3 EntityDataSerializers.<clinit> order and defineId indices:
    // receipts/Q04CoreFix/derived-indices.txt.
    for (value, index, serializer) in [
        (
            EntityDataValue::CowVariant {
                index: 18,
                value: 2,
            },
            18,
            23,
        ),
        (
            EntityDataValue::PigVariant {
                index: 19,
                value: 2,
            },
            19,
            28,
        ),
        (
            EntityDataValue::ZombieNautilusVariant {
                index: 21,
                value: 2,
            },
            21,
            32,
        ),
    ] {
        let packet = ClientboundSetEntityData {
            entity_id: 17,
            values: vec![value],
        };
        let mut bytes = Vec::new();
        packet.encode(&mut bytes).unwrap();
        assert_eq!(bytes, [17, index, serializer, 2, 255]);
        assert_eq!(
            ClientboundSetEntityData::decode(&mut bytes.as_slice()).unwrap(),
            packet,
        );
        // Registry zero is a reference, not an inline holder sentinel.
        assert!(
            ClientboundSetEntityData::decode(&mut [17, index, serializer, 0, 255].as_slice())
                .is_ok()
        );
        assert!(
            ClientboundSetEntityData::decode(
                &mut [17, index, serializer, 255, 255, 255, 255, 15, 255].as_slice(),
            )
            .is_err()
        );
    }
}
