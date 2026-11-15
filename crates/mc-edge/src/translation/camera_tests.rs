use super::*;
use mc_protocol::{Packet, packets::play::ClientboundSetCamera};

fn camera_step(upper: i32, lower: i32) -> Step {
    let report = |version| {
        let mut packets: serde_json::Value = serde_json::from_str(PACKETS).unwrap();
        // Local javap registrations, receipts/SpectateFix/ids-<release>.txt.
        packets["play"]["clientbound"]["minecraft:set_camera"] = serde_json::json!({
            "protocol_id": if version == 777 { 0x5f } else { 0x5d }
        });
        Reports::parse(&packets.to_string(), "{}", "{}").unwrap()
    };
    Step {
        upper,
        lower,
        factory: Box::new(adjacent::test_profile(Tables {
            formats: Formats { upper, lower },
            packets: Packets::new(&report(upper), &report(lower)),
            ..Tables::default()
        })),
    }
}

#[test]
fn set_camera_encodes_for_every_supported_version_through_adjacent_chain() {
    for (version, wire_id) in [(775, 0x5d), (776, 0x5d), (777, 0x5f)] {
        let mut session = session(
            vec![camera_step(777, 776), camera_step(776, 775)],
            777,
            version,
        );
        enter_play(&mut session);
        for entity_id in [1, 300, i32::MAX] {
            let mut body = Vec::new();
            ClientboundSetCamera { entity_id }
                .encode(&mut body)
                .unwrap();
            let (id, translated) = if version == 777 {
                (ClientboundSetCamera::ID, body.clone())
            } else {
                session
                    .clientbound(ClientboundSetCamera::ID, &body)
                    .unwrap()
                    .unwrap()
            };
            assert_eq!(id, wire_id, "protocol={version}");
            assert_eq!(translated, body);
            assert_eq!(
                ClientboundSetCamera::decode(&mut translated.as_slice())
                    .unwrap()
                    .entity_id,
                entity_id
            );
        }
    }
}
