use super::*;
use crate::boundary::{ChangeSet, EntityId};

fn npc() -> (TickState, crate::entities::EntitySlot, String) {
    let (mut state, _, position) = crate::hostile_tests::setup(8.0);
    let request = ScriptOperationRequest::try_new(
        "spawn-command-test",
        ScriptOperation::Npc {
            operation: ScriptNpcOperation::Spawn {
                operation_id: "spawn-command-test".into(),
                generation_id: "command-test".into(),
                position: ScriptPosition::try_new(position[0], position[1], position[2]).unwrap(),
                entity_type: "minecraft:villager".into(),
            },
        },
    )
    .unwrap();
    let result = crate::plugin_npcs::execute(&mut state, "owner", &request);
    let ScriptOperationPayload::Npc { result } = result.payload() else {
        panic!("{result:?}");
    };
    let ScriptNpcResult::Snapshot { npc } = &**result else {
        panic!("{result:?}");
    };
    let handle = npc.handle.clone();
    let slot = crate::plugin_npcs::owned_slot(&state, "owner", &handle).unwrap();
    synchronize_entity_custody(&mut state, slot).unwrap();
    (state, slot, handle)
}

fn reserve_request(
    endpoint: Endpoint,
    fence: ScriptInventoryFence,
    count: u64,
) -> ScriptOperationRequest {
    ScriptOperationRequest::try_new(
        "reserve-command-test",
        ScriptOperation::Inventory {
            operation: ScriptOwnedInventoryOperation::Reserve {
                operation_id: "reserve-command-test".into(),
                endpoint,
                resource_plan: ScriptInventoryResourcePlan::new(vec![
                    ScriptInventoryWorkPortion::new(
                        1,
                        vec![ScriptInventoryMaterial::new(
                            "minecraft:emerald".into(),
                            count,
                        )],
                    ),
                ]),
                expected_revision: fence,
            },
        },
    )
    .unwrap()
}

#[test]
fn command_custody_npc_reserve_is_rejected_by_real_ingress_without_state_changes() {
    let (mut state, slot, handle) = npc();
    for endpoint in [
        Endpoint::NpcEquipment {
            handle: handle.clone(),
        },
        Endpoint::NpcCarry { handle },
    ] {
        let image = load(&state, "owner", &endpoint).unwrap();
        let request = reserve_request(endpoint, image.fence, 1);
        let before = state.plugins.clone();
        let row = state.entities.row(slot).clone();
        let result = execute(&mut state, "owner", &request);
        assert_eq!(result.failure(), Some(Failure::InvalidRequest));
        assert_eq!(state.plugins, before);
        assert_eq!(state.entities.row(slot), &row);
    }
}

#[test]
fn command_custody_data_preflights_exact_revision_capacity_before_health_or_equipment() {
    for exhausted_equipment in [true, false] {
        let (mut state, slot, handle) = npc();
        let endpoint = if exhausted_equipment {
            Endpoint::NpcEquipment { handle }
        } else {
            Endpoint::NpcCarry { handle }
        };
        let key = endpoint_key(&state, &endpoint);
        let mut version = image_version(&state, &key).unwrap().unwrap();
        version.revision = MAX_SCRIPT_WORLD_TIME;
        state.plugins.set_domain(
            OWNER,
            &format!("image/{key}"),
            serde_json::to_value(version).unwrap(),
        );
        let id = state.entities.body(slot).id;
        let context = crate::command_context::Context::source(
            &state,
            crate::command_console::CONNECTION,
            id,
            2,
        );
        let row = state.entities.row(slot).clone();
        let body = *state.entities.body(slot);
        let plugins = state.plugins.clone();
        let journal = state.appearance_changed.clone();
        let result = crate::command_data::command(
            &mut state,
            context.connection,
            id,
            "data merge entity @s {Health:7.0f,equipment:{mainhand:{id:'minecraft:emerald',count:3}}}",
        );
        if exhausted_equipment {
            assert!(
                matches!(result, crate::command_execution::CommandResult::Failed),
                "changed image must not exceed the script revision range"
            );
            assert_eq!(state.entities.row(slot), &row);
            assert_eq!(state.entities.body(slot), &body);
            assert_eq!(
                state.plugins, plugins,
                "no durable metadata or dirty-key changes on refusal"
            );
            assert_eq!(state.appearance_changed, journal);
        } else {
            assert!(
                matches!(result, crate::command_execution::CommandResult::Success(_)),
                "unchanged carry image at capacity must not reject equipment: {result:?}"
            );
            assert_eq!(state.entities.row(slot).health, 7.0);
            assert_eq!(
                state.entities.row(slot).appearance.equipment[0]
                    .as_ref()
                    .unwrap()
                    .count,
                3
            );
            assert_eq!(
                image_version(&state, &key).unwrap().unwrap().revision,
                MAX_SCRIPT_WORLD_TIME
            );
            assert!(state.appearance_changed.contains(&id));
        }
    }
}

#[test]
fn command_custody_removing_equipment_clears_the_live_image_and_advances_fence() {
    let (mut state, slot, handle) = npc();
    let stack = item(&state, "minecraft:iron_helmet", 1);
    std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[5] =
        Some(stack);
    synchronize_entity_custody(&mut state, slot).unwrap();
    let before = native_npc_inventory_revision(&state, "owner", &handle).unwrap();
    let id = state.entities.body(slot).id;
    assert!(matches!(
        crate::command_data::command(
            &mut state,
            crate::command_console::CONNECTION,
            id,
            "data remove entity @s equipment",
        ),
        crate::command_execution::CommandResult::Success(_)
    ));
    assert!(state.entities.row(slot).appearance.equipment.iter().all(Option::is_none));
    assert!(native_npc_inventory_revision(&state, "owner", &handle).unwrap() > before);
}

fn item(state: &TickState, name: &str, count: i32) -> ItemStack {
    ItemStack::new(
        state
            .registry
            .items
            .id_of(&Identifier::parse(name).unwrap())
            .unwrap(),
        count,
    )
}
fn command(state: &mut TickState, actor: EntityId, text: &str) {
    let context = crate::command_context::Context::source(
        state,
        crate::command_console::CONNECTION,
        actor,
        2,
    );
    crate::command_context::run(state, text, context, &mut ChangeSet::default());
}
fn loot(state: &mut TickState, actor: EntityId, slot: usize, table: &str) -> Result<i32, String> {
    let text = format!("replace block 4 201 4 container.{slot} 2 loot {table}");
    let parsed = crate::command_loot::parse(&text.split_whitespace().collect::<Vec<_>>()).unwrap();
    crate::command_loot::execute(state, ConnectionId(1), actor, &parsed)
}
fn table(state: &mut TickState, id: &str, second: &str, count: i32) {
    let json = format!(
        r#"{{"type":"minecraft:chest","pools":[
        {{"rolls":1,"entries":[{{"type":"minecraft:item","name":"minecraft:diamond"}}]}},
        {{"rolls":1,"entries":[{{"type":"minecraft:item","name":"{second}","functions":[{{"function":"minecraft:set_count","count":{count}}}]}}]}}
    ]}}"#
    );
    let table =
        mc_data::loot::chest_26_1_2::ChestLootTable::compile(Identifier::parse(id).unwrap(), &json)
            .unwrap();
    std::sync::Arc::make_mut(&mut std::sync::Arc::make_mut(&mut state.registry).chest_loot)
        .insert(table);
}

#[test]
fn command_custody_loot_replacement_admits_whole_reserved_range_before_writing() {
    let (mut state, actor, _) = crate::hostile_tests::setup(8.0);
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    command(&mut state, actor, "setblock 4 201 4 minecraft:chest");
    command(
        &mut state,
        actor,
        "data merge block 4 201 4 {Items:[{Slot:0b,id:'minecraft:gold_ingot',count:1},{Slot:1b,id:'minecraft:emerald',count:8}]}",
    );
    let endpoint = Endpoint::Container {
        dimension: "minecraft:overworld".into(),
        position: ScriptBlockPosition::new(4, 201, 4),
    };
    let image = load(&state, "owner", &endpoint).unwrap();
    let reserved = execute(
        &mut state,
        "owner",
        &reserve_request(endpoint.clone(), image.fence, 8),
    );
    assert_eq!(reserved.failure(), None, "{reserved:?}");
    table(&mut state, "test:refused", "minecraft:stone", 1);
    table(&mut state, "test:held", "minecraft:emerald", 8);
    let slots = state.block_entities[&(4, 201, 4)].slots.clone();
    let opaque = state.opaque_changes.clone();
    let plugins = state.plugins.clone();
    assert!(loot(&mut state, actor, 0, "test:refused").is_err());
    assert_eq!(
        state.block_entities[&(4, 201, 4)].slots,
        slots,
        "later reserved slot cannot leave an earlier write"
    );
    assert_eq!(state.opaque_changes, opaque);
    assert_eq!(state.plugins, plugins);
    assert!(loot(&mut state, actor, 26, "test:held").is_err());
    assert_eq!(
        state.block_entities[&(4, 201, 4)].slots,
        slots,
        "missing final slot cannot leave a valid prefix"
    );
    // A final-image-preserving move must not be rejected by an intermediate floor.
    let emerald = item(&state, "minecraft:emerald", 8);
    let gold = item(&state, "minecraft:gold_ingot", 1);
    state.block_entities.get_mut(&(4, 201, 4)).unwrap().slots[..2]
        .clone_from_slice(&[emerald, gold]);
    assert_eq!(loot(&mut state, actor, 0, "test:held"), Ok(2));
    assert_eq!(
        state.block_entities[&(4, 201, 4)].slots[0],
        item(&state, "minecraft:diamond", 1)
    );
    assert_eq!(
        state.block_entities[&(4, 201, 4)].slots[1],
        item(&state, "minecraft:emerald", 8)
    );
    assert!(!below_block_floors(
        &state,
        BlockPos { x: 4, y: 201, z: 4 },
        &state.block_entities[&(4, 201, 4)].slots
    ));
    let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
    let entries = crate::core::journal_entries_for_tick(&mut state, &output);
    assert!(
        format!("{entries:?}").contains("OpaqueBlockEntity"),
        "changed final inventory needs a durable postimage"
    );
}
