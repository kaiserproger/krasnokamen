//! Authoritative place/break/chest observations for each real-cache TCP join.
use super::*;
use mc_protocol::packets::play::{
    ClientboundOpenScreen, Direction, InteractionHand, PlayerActionKind, ServerboundContainerClose,
    ServerboundPlayerAction, ServerboundSetCarriedItem, ServerboundUseItemOn, pack_block_pos,
};

async fn console(core: &mc_core::Core, line: String) {
    let (command, completed) = mc_core::command_console::ConsoleCommand::new(line);
    assert!(core.submit(mc_core::boundary::PlayerInput::ConsoleCommand(command)));
    completed.await.unwrap();
}

async fn send<P: Packet>(client: &mut Client, name: &str, packet: &P) {
    let mut body = Vec::new();
    packet.encode(&mut body).unwrap();
    client.send(client.id("play", false, name), &body).await;
}

async fn block(client: &mut Client, position: [i32; 3], expected: i32) {
    let packed = pack_block_pos(position[0], position[1], position[2]);
    let seen = client
        .play_until(|client, frame| {
            if frame.id != client.id("play", true, "minecraft:block_update") {
                return None;
            }
            let mut body = frame.body.as_ref();
            (body.read_i64().unwrap() == packed).then(|| body.read_varint().unwrap())
        })
        .await;
    assert_eq!(seen, expected, "authoritative block update at {position:?}");
}

fn use_on(position: [i32; 3], sequence: i32) -> ServerboundUseItemOn {
    ServerboundUseItemOn {
        hand: InteractionHand::MainHand,
        position: pack_block_pos(position[0], position[1], position[2]),
        direction: Direction::Up,
        cursor_x: 0.5,
        cursor_y: 1.0,
        cursor_z: 0.5,
        inside: false,
        world_border_hit: false,
        sequence,
    }
}

pub(super) async fn run(client: &mut Client, core: &mc_core::Core, support: [i32; 3]) {
    console(core, "gamemode creative Chain1".into()).await;
    let position = [support[0], support[1] + 1, support[2]];
    console(
        core,
        format!(
            "setblock {} {} {} minecraft:air",
            position[0], position[1], position[2]
        ),
    )
    .await;
    let stone = core
        .registry()
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    let peer_stone = client.profile.tables.items.get(stone as i32).unwrap();
    // A raw creative slot is in the endpoint's item/component id space.
    let mut slot = Vec::new();
    slot.write_i16(36);
    for value in [8, peer_stone, 0, 0] {
        slot.write_varint(value);
    }
    client
        .send(
            client.id("play", false, "minecraft:set_creative_mode_slot"),
            &slot,
        )
        .await;
    send(
        client,
        "minecraft:set_carried_item",
        &ServerboundSetCarriedItem { slot: 0 },
    )
    .await;
    // A per-connection keepalive-shaped command fence would not prove inventory
    // admission. Observe the server's actual slot update before placing.
    client
        .play_until(|client, frame| {
            if frame.id != client.id("play", true, "minecraft:container_set_slot") {
                return None;
            }
            let mut body = frame.body.as_ref();
            let window = body.read_varint().unwrap();
            body.read_varint().unwrap();
            let index = body.read_i16().unwrap();
            if window != 0 || index != 36 {
                return None;
            }
            let count = body.read_varint().unwrap();
            (count == 8 && body.read_varint().unwrap() == peer_stone).then_some(())
        })
        .await;
    send(client, "minecraft:use_item_on", &use_on(support, 1)).await;
    let canonical_stone = core.registry().states.stone as i32;
    let peer_stone_state = client
        .profile
        .tables
        .block_states
        .get(canonical_stone)
        .unwrap();
    block(client, position, peer_stone_state).await;
    send(
        client,
        "minecraft:player_action",
        &ServerboundPlayerAction {
            action: PlayerActionKind::StartDestroyBlock,
            position: pack_block_pos(position[0], position[1], position[2]),
            direction: Direction::Up,
            sequence: 2,
        },
    )
    .await;
    let air = client
        .profile
        .tables
        .block_states
        .get(core.registry().states.air as i32)
        .unwrap();
    block(client, position, air).await;
    console(
        core,
        format!(
            "setblock {} {} {} minecraft:chest",
            position[0], position[1], position[2]
        ),
    )
    .await;
    let chest = core
        .registry()
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:chest").unwrap())
        .unwrap()
        .default
        .0 as i32;
    let peer_chest = client.profile.tables.block_states.get(chest).unwrap();
    block(client, position, peer_chest).await;
    // Empty hand is not required for opening a chest, but avoids an accidental
    // placement masking a failed interaction branch.
    let mut empty = Vec::new();
    empty.write_i16(36);
    empty.write_varint(0);
    client
        .send(
            client.id("play", false, "minecraft:set_creative_mode_slot"),
            &empty,
        )
        .await;
    send(client, "minecraft:use_item_on", &use_on(position, 3)).await;
    let opened = client
        .play_until(|client, frame| {
            (frame.id == client.id("play", true, "minecraft:open_screen")).then(|| {
                let mut body = frame.body.as_ref();
                let packet = ClientboundOpenScreen::decode(&mut body).unwrap();
                assert!(body.is_empty());
                packet
            })
        })
        .await;
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(client.profile.data.root().join("reports/registries.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        opened.menu_type,
        report["minecraft:menu"]["entries"]["minecraft:generic_9x3"]["protocol_id"]
            .as_i64()
            .unwrap() as i32
    );
    client
        .play_until(|client, frame| {
            if frame.id != client.id("play", true, "minecraft:container_set_content") {
                return None;
            }
            let mut body = frame.body.as_ref();
            if body.read_varint().unwrap() != opened.container_id {
                return None;
            }
            body.read_varint().unwrap();
            assert_eq!(
                body.read_varint().unwrap(),
                63,
                "27 chest slots and 36 player slots"
            );
            Some(())
        })
        .await;
    send(
        client,
        "minecraft:container_close",
        &ServerboundContainerClose {
            container_id: opened.container_id,
        },
    )
    .await;
}
