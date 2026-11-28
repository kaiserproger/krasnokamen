//! A released builder's stock crosses two native inventories and one durable
//! return duty in a single tick WAL batch, never a second inventory mirror.
use super::*;
use crate::settlements::construction::carry_release_tests::{accounted, item, retained_release};
use crate::settlements::inventory::{self, Endpoint};

fn handoff_cut(published: bool) {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = fixture(directory.path());
    drill.state.world.ensure_blocking(
        &[(mc_world::ChunkPos { x: 0, z: 0 }, 0)]
            .into_iter()
            .collect(),
    );
    let stone = crate::settlements::allocator::tests::block(&drill.state, "minecraft:stone", &[]);
    for x in 0..16 {
        for z in 0..16 {
            drill.state.world.set_block(x, 200, z, stone);
        }
    }
    let (state, id, donor, builder, handle) = retained_release(drill.state, true);
    drill.state = state;
    let fence = item(&drill.state, "minecraft:acacia_fence");
    let donor_uuid = drill.state.entities.row(donor).uuid;
    let builder_uuid = drill.state.entities.row(builder).uuid;
    // Six carried, six owed, four wanted: a full duty must survive part-paid.
    drill
        .state
        .entities
        .row_mut(donor)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(mc_data::ItemStack::new(fence, 6));
    drill
        .state
        .settlements
        .get_mut(id)
        .unwrap()
        .carry_pending
        .insert(donor_uuid, vec![(fence, 6)]);
    drill.state.entities.set_pos(builder, [13.5, 201.0, 14.5]);
    drill.state.tick_control.set_frozen(true);
    drill.run(&[]).unwrap();
    drill.checkpoint();
    let donor_endpoint = Endpoint::VillagerCarry { uuid: donor_uuid };
    let builder_endpoint = Endpoint::VillagerCarry { uuid: builder_uuid };
    let donor_before = inventory::load(&drill.state, &donor_endpoint)
        .unwrap()
        .slots;
    let builder_before = inventory::load(&drill.state, &builder_endpoint)
        .unwrap()
        .slots;
    let before = accounted(&drill.state, id, &handle);
    crate::plugin_structures::settlement_builds::fetch(
        &mut drill.state,
        builder_uuid,
        &handle,
        crate::plugin_structures::settlement_builds::Pickup::debtor(donor_uuid, [14, 201, 14]),
        fence,
    )
    .unwrap();
    let donor_after = inventory::load(&drill.state, &donor_endpoint)
        .unwrap()
        .slots;
    let builder_after = inventory::load(&drill.state, &builder_endpoint)
        .unwrap()
        .slots;
    assert_ne!(donor_before, donor_after);
    assert_ne!(builder_before, builder_after);
    assert_eq!(accounted(&drill.state, id, &handle), before);
    let batch = drill.capture_tick(&[]);
    if published {
        drill.append(&batch).unwrap();
    }
    let mut reopened = drill.crash(); // No checkpoint can conceal a missing WAL post-image.
    reopened.state.world.ensure_blocking(
        &[(mc_world::ChunkPos { x: 0, z: 0 }, 0)]
            .into_iter()
            .collect(),
    );
    crate::tick::sync_chunk_block_entities(&mut reopened.state);
    assert_eq!(
        inventory::load(&reopened.state, &donor_endpoint)
            .unwrap()
            .slots,
        if published { donor_after } else { donor_before }
    );
    assert_eq!(
        inventory::load(&reopened.state, &builder_endpoint)
            .unwrap()
            .slots,
        if published {
            builder_after
        } else {
            builder_before
        }
    );
    let duty = reopened
        .state
        .settlements
        .get(id)
        .unwrap()
        .carry_pending
        .get(&donor_uuid);
    if published {
        assert_eq!(duty, Some(&vec![(fence, 2)]), "the unpaid duty replays");
    } else {
        assert_eq!(duty, Some(&vec![(fence, 6)]));
    }
    assert_eq!(accounted(&reopened.state, id, &handle), before);
}

#[test]
fn released_construction_handoff_replays_both_carries_and_the_return_duty() {
    handoff_cut(true);
}

#[test]
fn unpublished_construction_handoff_recovers_the_original_custody_and_duty() {
    handoff_cut(false);
}
