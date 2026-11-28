use super::*;
use crate::entities::EntitySlot;
use crate::settlements::allocator::tests::{block, villager};
use crate::villager::Activity;
use mc_data::ItemStack;
use std::collections::BTreeMap;

pub(crate) fn item(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap()
}

/// R5's remaining storehouse work: 26 planks, then four fences already carried
/// by the released Weaponsmith. Every bank slot is occupied; crafting another
/// fence needs a new slot. The 26-plank variant exercises the earlier
/// `worksite_unreached` release and the existing bounded return-duty path too.
pub(crate) fn retained_release(
    state: TickState,
    with_planks: bool,
) -> (TickState, SettlementId, EntitySlot, EntitySlot, String) {
    let (mut state, id, donor) = tests::fixture_in(state);
    let planks = item(&state, "minecraft:acacia_planks");
    let fence = item(&state, "minecraft:acacia_fence");
    let log = item(&state, "minecraft:acacia_log");
    let stick = item(&state, "minecraft:stick");
    let filler = item(&state, "minecraft:cobblestone");
    let food = item(&state, "minecraft:bread");
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    slots.fill(ItemStack::new(filler, 64));
    slots[0] = ItemStack::new(planks, if with_planks { 1 } else { 27 });
    slots[1] = ItemStack::new(log, 64);
    slots[2] = ItemStack::new(stick, 1);
    let cells = (0..30)
        .map(|n| {
            let name = if n < 26 {
                "minecraft:acacia_planks"
            } else {
                "minecraft:acacia_fence"
            };
            executor::PlannedCell {
                at: [3 + n % 8, 201, 3 + n / 8],
                state: block(&state, name, &[]),
                resource: Some(name.into()),
                quantity: 1,
                portion: executor::PortionKind::Build,
            }
        })
        .collect();
    let site = ScriptWorldBounds::new([3, 201, 3], [10, 205, 10]).unwrap();
    let handle = executor::prepare(&mut state, id, "storage".into(), site, site, cells).unwrap();
    let uuid = state.entities.row(donor).uuid;
    access::push_project(
        state.settlements.get_mut(id).unwrap(),
        Project {
            settlement: id,
            blueprint: "storage".into(),
            level: 1,
            origin: site.min,
            rotation: 0,
            building: None,
            structure: handle.clone(),
            ordered: true,
            builder: Some((uuid, Some(Profession::Weaponsmith))),
            order_seq: None,
        },
    );
    let v = state.entities.row_mut(donor).villager.as_mut().unwrap();
    v.inventory[0] = Some(ItemStack::new(fence, 4));
    v.inventory[1] = Some(ItemStack::new(food, 3));
    if with_planks {
        v.inventory[2] = Some(ItemStack::new(planks, 26));
    }
    assert!(release_incumbent(&mut state, id, &handle, donor));
    assert_eq!(
        state
            .entities
            .row(donor)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::Weaponsmith
    );
    drain_carry_duties(&mut state, id);
    assert_eq!(
        state.settlements.get(id).unwrap().carry_pending[&uuid],
        vec![(fence, 4)]
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots[0].count, 27);
    assert!(
        state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .all(|stack| !stack.is_empty())
    );
    state.entities.set_pos(donor, [14.5, 201.0, 14.5]);
    let builder = villager(&mut state, [1.5, 201.0, 2.5]);
    let entity = state.entities.body(builder).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    let v = state.entities.row_mut(builder).villager.as_mut().unwrap();
    v.data.profession = Profession::Builder;
    v.activity = Activity::Work;
    tests::bind_project(&mut state, id, &handle, builder);
    (state, id, donor, builder, handle)
}

/// Every physical stack plus every consumed material, not just wood. A handoff
/// must leave this map unchanged; each placement moves exactly one unit into
/// the canonical structure receipt.
pub(crate) fn accounted(state: &TickState, id: SettlementId, handle: &str) -> BTreeMap<u32, i64> {
    let mut counts = BTreeMap::new();
    for stack in state.block_entities.values().flat_map(|bank| &bank.slots) {
        if !stack.is_empty() {
            *counts.entry(stack.item_id).or_default() += i64::from(stack.count);
        }
    }
    for index in 0..state.entities.len() {
        let slot = EntitySlot(index as u32);
        let Some(v) = state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .filter(|v| v.settlement == Some(id))
        else {
            continue;
        };
        for stack in v.inventory.iter().flatten() {
            *counts.entry(stack.item_id).or_default() += i64::from(stack.count);
        }
    }
    for material in executor::status(state, id, handle).unwrap().consumed {
        *counts.entry(item(state, &material.resource)).or_default() += material.quantity as i64;
    }
    counts
}

fn advance_one(state: &mut TickState, id: SettlementId, handle: &str, builder: EntitySlot) {
    let cell = executor::next_cell(state, id, handle).unwrap().unwrap();
    state.entities.set_pos(
        builder,
        [
            f64::from(cell[0]) + 1.5,
            f64::from(cell[1]),
            f64::from(cell[2]) + 0.5,
        ],
    );
    let uuid = state.entities.row(builder).uuid;
    executor::advance(state, id, handle, uuid).unwrap();
}

fn carried_fences(state: &TickState, slot: EntitySlot, fence: u32) -> i32 {
    state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .iter()
        .flatten()
        .filter(|stack| stack.item_id == fence)
        .map(|stack| stack.count)
        .sum()
}

#[test]
fn released_fences_unblock_full_bank_storehouse_next_cell() {
    let (mut state, id, _, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), false);
    let before = accounted(&state, id, &handle);
    let uuid = state.entities.row(builder).uuid;
    let planks = item(&state, "minecraft:acacia_planks");
    executor::fetch(
        &mut state,
        uuid,
        &handle,
        executor::Pickup::bank([1, 201, 1]),
        planks,
    )
    .unwrap();
    assert_eq!(accounted(&state, id, &handle), before);
    let next = executor::next_cell(&state, id, &handle).unwrap().unwrap();
    advance_one(&mut state, id, &handle, builder);
    assert_eq!(
        state.world.block_at(next[0], next[1], next[2]).unwrap(),
        block(&state, "minecraft:acacia_planks", &[])
    );
    assert_eq!(
        executor::status(&state, id, &handle)
            .unwrap()
            .completed_work_units,
        1
    );
    assert_eq!(accounted(&state, id, &handle), before);
}

#[test]
fn worksite_release_returns_26_planks_and_the_next_builder_collects_all_four_fences() {
    let (mut state, id, donor, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), true);
    let before = accounted(&state, id, &handle);
    let uuid = state.entities.row(builder).uuid;
    let donor_uuid = state.entities.row(donor).uuid;
    let planks = item(&state, "minecraft:acacia_planks");
    let fence = item(&state, "minecraft:acacia_fence");
    executor::fetch(
        &mut state,
        uuid,
        &handle,
        executor::Pickup::bank([1, 201, 1]),
        planks,
    )
    .unwrap();
    for _ in 0..26 {
        advance_one(&mut state, id, &handle, builder);
    }
    let pickup = executor::pickup_point(
        &state,
        id,
        uuid,
        &handle,
        fence,
        state.entities.pos(builder),
    )
    .unwrap();
    assert_eq!(pickup.at, [14, 201, 14]);
    assert_eq!(pickup.source, executor::PickupSource::Debtor(donor_uuid));
    assert_eq!(
        executor::fetch(&mut state, uuid, &handle, pickup, fence),
        Err(mc_script::ScriptOperationFailure::Blocked)
    );
    assert_eq!(accounted(&state, id, &handle), before);
    assert_eq!(
        state.settlements.get(id).unwrap().carry_pending[&donor_uuid],
        vec![(fence, 4)]
    );
    state.entities.set_pos(builder, [13.5, 201.0, 14.5]);
    executor::fetch(&mut state, uuid, &handle, pickup, fence).unwrap();
    assert!(
        !state
            .settlements
            .get(id)
            .unwrap()
            .carry_pending
            .contains_key(&donor_uuid)
    );
    assert_eq!(accounted(&state, id, &handle), before);
    for _ in 0..4 {
        advance_one(&mut state, id, &handle, builder);
    }
    assert_eq!(
        executor::status(&state, id, &handle).unwrap().state,
        ScriptStructureState::Committed
    );
    assert_eq!(accounted(&state, id, &handle), before);
    let v = state.entities.row(donor).villager.as_ref().unwrap();
    assert_eq!(
        v.inventory
            .iter()
            .flatten()
            .map(|stack| stack.count)
            .sum::<i32>(),
        3,
        "only the resident's bread remains"
    );
}

#[test]
fn physical_handoff_is_bounded_by_the_duty_and_preserves_stack_components() {
    let (mut state, id, donor, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), false);
    let fence = item(&state, "minecraft:acacia_fence");
    let donor_uuid = state.entities.row(donor).uuid;
    let v = state.entities.row_mut(donor).villager.as_mut().unwrap();
    let stack = v.inventory[0].as_mut().unwrap();
    stack.count = 6; // A later personal load cannot enlarge the recorded duty.
    stack.custom_name = Some("retained-native-fence".into());
    let before = accounted(&state, id, &handle);
    state.entities.set_pos(builder, [13.5, 201.0, 14.5]);
    let uuid = state.entities.row(builder).uuid;
    let pickup = executor::Pickup::debtor(donor_uuid, [14, 201, 14]);
    executor::fetch(&mut state, uuid, &handle, pickup, fence).unwrap();
    let donated = state
        .entities
        .row(builder)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .iter()
        .flatten()
        .find(|stack| stack.item_id == fence)
        .unwrap();
    assert_eq!(donated.count, 4);
    assert_eq!(
        donated.custom_name.as_deref(),
        Some("retained-native-fence")
    );
    let retained = state
        .entities
        .row(donor)
        .villager
        .as_ref()
        .unwrap()
        .inventory[0]
        .as_ref()
        .unwrap();
    assert_eq!(retained.count, 2);
    assert_eq!(retained.custom_name, donated.custom_name);
    assert_eq!(accounted(&state, id, &handle), before);
    assert_eq!(
        executor::pickup_point(
            &state,
            id,
            uuid,
            &handle,
            fence,
            state.entities.pos(builder)
        ),
        None,
        "unowed personal items are not construction sources"
    );
}

#[test]
fn a_return_duty_reserves_only_its_addressable_physical_stock() {
    let (mut state, id, donor, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), false);
    let uuid = state.entities.row(builder).uuid;
    let donor_uuid = state.entities.row(donor).uuid;
    let fence = item(&state, "minecraft:acacia_fence");
    let planks = item(&state, "minecraft:acacia_planks");
    for condition in 0..3 {
        let v = state.entities.row_mut(donor).villager.as_mut().unwrap();
        v.conversion_time = (condition == 0).then_some(20);
        v.inventory[0].as_mut().unwrap().count = if condition == 1 { 3 } else { 4 };
        state.settlements.get_mut(id).unwrap().carry_pending.insert(
            donor_uuid,
            vec![(fence, if condition == 2 { 1 } else { 4 })],
        );
        let before = accounted(&state, id, &handle);
        assert_eq!(
            executor::fetch(
                &mut state,
                uuid,
                &handle,
                executor::Pickup::bank([1, 201, 1]),
                planks
            ),
            Err(mc_script::ScriptOperationFailure::InsufficientItems),
            "unaddressable, missing, or unowed fences cannot fund future work"
        );
        assert_eq!(accounted(&state, id, &handle), before);
    }
    // The same duty, now addressable, does fund the haul: the three refusals
    // above pin the custody screen, not a broken fixture.
    let v = state.entities.row_mut(donor).villager.as_mut().unwrap();
    v.conversion_time = None;
    v.inventory[0].as_mut().unwrap().count = 4;
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .carry_pending
        .insert(donor_uuid, vec![(fence, 4)]);
    executor::fetch(
        &mut state,
        uuid,
        &handle,
        executor::Pickup::bank([1, 201, 1]),
        planks,
    )
    .expect("an addressable duty funds future work");
}

#[test]
fn a_smaller_duty_caps_the_handoff_before_the_remaining_haul() {
    let (mut state, id, donor, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), false);
    let fence = item(&state, "minecraft:acacia_fence");
    let donor_uuid = state.entities.row(donor).uuid;
    state
        .entities
        .row_mut(donor)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0]
        .as_mut()
        .unwrap()
        .count = 6;
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .carry_pending
        .insert(donor_uuid, vec![(fence, 2)]);
    state.entities.set_pos(builder, [13.5, 201.0, 14.5]);
    let before = accounted(&state, id, &handle);
    let uuid = state.entities.row(builder).uuid;
    let pickup = executor::Pickup::debtor(donor_uuid, [14, 201, 14]);
    executor::fetch(&mut state, uuid, &handle, pickup, fence).unwrap();
    assert_eq!(carried_fences(&state, builder, fence), 2);
    assert_eq!(carried_fences(&state, donor, fence), 4);
    assert!(
        !state
            .settlements
            .get(id)
            .unwrap()
            .carry_pending
            .contains_key(&donor_uuid)
    );
    assert_eq!(
        executor::fetch(&mut state, uuid, &handle, pickup, fence),
        Err(mc_script::ScriptOperationFailure::Forbidden)
    );
    assert_eq!(accounted(&state, id, &handle), before);
}

#[test]
fn a_handoff_leaves_the_unpaid_part_of_the_duty_recorded() {
    let (mut state, id, donor, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), false);
    let fence = item(&state, "minecraft:acacia_fence");
    let donor_uuid = state.entities.row(donor).uuid;
    let v = state.entities.row_mut(donor).villager.as_mut().unwrap();
    v.inventory[0] = Some(ItemStack::new(fence, 6));
    // The builder already carries two of the four fence cells, so this trip's
    // remaining haul is two: a full duty (four) must not be erased.
    let v = state.entities.row_mut(builder).villager.as_mut().unwrap();
    v.inventory[0] = Some(ItemStack::new(fence, 2));
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .carry_pending
        .insert(donor_uuid, vec![(fence, 4)]);
    state.entities.set_pos(builder, [13.5, 201.0, 14.5]);
    let uuid = state.entities.row(builder).uuid;
    let before = accounted(&state, id, &handle);
    let pickup = executor::Pickup::debtor(donor_uuid, [14, 201, 14]);
    executor::fetch(&mut state, uuid, &handle, pickup, fence).unwrap();
    assert_eq!(carried_fences(&state, builder, fence), 4);
    assert_eq!(carried_fences(&state, donor, fence), 4);
    assert_eq!(
        state.settlements.get(id).unwrap().carry_pending[&donor_uuid],
        vec![(fence, 2)],
        "an unpaid remainder stays on the released resident"
    );
    assert_eq!(accounted(&state, id, &handle), before);
}

#[test]
fn donor_arrival_and_handoff_share_the_exact_body_distance() {
    let (mut state, id, donor, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), false);
    let uuid = state.entities.row(builder).uuid;
    let donor_uuid = state.entities.row(donor).uuid;
    let planks = item(&state, "minecraft:acacia_planks");
    let fence = item(&state, "minecraft:acacia_fence");
    executor::fetch(
        &mut state,
        uuid,
        &handle,
        executor::Pickup::bank([1, 201, 1]),
        planks,
    )
    .unwrap();
    for _ in 0..26 {
        advance_one(&mut state, id, &handle, builder);
    }
    state.entities.set_pos(donor, [14.99, 201.0, 14.99]);
    state.entities.set_pos(builder, [12.1, 201.0, 14.5]);
    let pickup = executor::pickup_point(
        &state,
        id,
        uuid,
        &handle,
        fence,
        state.entities.pos(builder),
    )
    .unwrap();
    assert_eq!(pickup.source, executor::PickupSource::Debtor(donor_uuid));
    assert!(
        (state.entities.pos(builder)[0] - 14.5).abs() < 2.5,
        "the old floored-cell predicate would stop here"
    );
    assert!(!executor::pickup_reached(
        &state, id, builder, &pickup, fence
    ));
    let mut v = state.entities.row_mut(builder).villager.take().unwrap();
    let target = crate::villager::building::work(
        &mut state,
        builder,
        &mut v,
        &mut crate::boundary::ChangeSet::default(),
    )
    .unwrap();
    assert!(
        target.exact,
        "route closer instead of repeatedly refusing at the cell's 2.5-block boundary"
    );
    state.entities.row_mut(builder).villager = Some(v);
    assert_eq!(
        executor::fetch(&mut state, uuid, &handle, pickup, fence),
        Err(mc_script::ScriptOperationFailure::Blocked)
    );
    // Inside the donor's 2.5-block body radius while outside the old
    // cell-centre predicate: the exact tick gate must still transfer.
    state.entities.set_pos(builder, [16.34, 201.0, 16.34]);
    let centre = (16.34_f64 - 14.5).powi(2) + (16.34_f64 - 14.5).powi(2);
    assert!(
        centre > 2.5_f64.powi(2),
        "old cell-centre check would refuse"
    );
    assert!(executor::pickup_reached(
        &state, id, builder, &pickup, fence
    ));
    crate::villager::building::tick(&mut state, id);
    assert!(
        !state
            .settlements
            .get(id)
            .unwrap()
            .carry_pending
            .contains_key(&donor_uuid)
    );
}

#[test]
fn bank_crafting_is_preferred_to_a_released_resident_pickup() {
    // An admissible bank is selected and actually debited.
    let (mut state, id, donor, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), false);
    let fence = item(&state, "minecraft:acacia_fence");
    let planks = item(&state, "minecraft:acacia_planks");
    let donor_uuid = state.entities.row(donor).uuid;
    let uuid = state.entities.row(builder).uuid;
    {
        let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
        slots[0] = ItemStack::new(planks, 64);
        slots[3] = ItemStack::new(fence, 4);
    }
    let pickup = executor::pickup_point(
        &state,
        id,
        uuid,
        &handle,
        fence,
        state.entities.pos(builder),
    )
    .unwrap();
    assert_eq!(pickup.source, executor::PickupSource::Bank);
    assert_eq!(pickup.at, [1, 201, 1]);
    executor::fetch(&mut state, uuid, &handle, pickup, fence).unwrap();
    assert_eq!(carried_fences(&state, builder, fence), 4);
    assert!(
        state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .all(|stack| stack.item_id != fence || stack.count == 0),
        "the bank paid, not the resident"
    );
    assert_eq!(
        state.settlements.get(id).unwrap().carry_pending[&donor_uuid],
        vec![(fence, 4)]
    );
    assert_eq!(carried_fences(&state, donor, fence), 4);
    assert!(
        state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .filter(|stack| stack.item_id == planks)
            .map(|stack| stack.count)
            .sum::<i32>()
            < 64
    );

    // A craft probe that passes while the real spend is refused by the plan's
    // own wood reservation must route to the resident, and debit the resident.
    let (mut state, id, donor, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), false);
    let donor_uuid = state.entities.row(donor).uuid;
    let uuid = state.entities.row(builder).uuid;
    let filler = item(&state, "minecraft:cobblestone");
    let stick = item(&state, "minecraft:stick");
    {
        let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
        slots.fill(ItemStack::new(filler, 64));
        slots[0] = ItemStack::new(planks, 4);
        slots[1] = ItemStack::new(stick, 2);
    }
    assert!(
        crate::settlements::construction_crafting::can_craft(
            &state,
            fence,
            &executor::source_images(&state, id)
        ),
        "the one-unit probe passes"
    );
    assert!(!executor::bank_admissible(
        &state,
        id,
        uuid,
        &handle,
        [1, 201, 1],
        fence
    ));
    state.entities.set_pos(builder, [13.5, 201.0, 14.5]);
    let pickup = executor::pickup_point(
        &state,
        id,
        uuid,
        &handle,
        fence,
        state.entities.pos(builder),
    )
    .unwrap();
    assert_eq!(pickup.source, executor::PickupSource::Debtor(donor_uuid));
    executor::fetch(&mut state, uuid, &handle, pickup, fence).unwrap();
    assert_eq!(carried_fences(&state, builder, fence), 4);
    assert_eq!(carried_fences(&state, donor, fence), 0);
    assert_eq!(
        state.block_entities[&(1, 201, 1)].slots[0].count,
        4,
        "the refused bank is untouched"
    );
    assert!(
        !state
            .settlements
            .get(id)
            .unwrap()
            .carry_pending
            .contains_key(&donor_uuid)
    );
}

#[test]
fn a_bank_and_a_resident_in_one_cell_debit_the_selected_body() {
    let (mut state, id, donor, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), false);
    let fence = item(&state, "minecraft:acacia_fence");
    let filler = item(&state, "minecraft:cobblestone");
    let donor_uuid = state.entities.row(donor).uuid;
    let uuid = state.entities.row(builder).uuid;
    // A full chest at the released resident's own cell: the cell now names two
    // custody pools.
    let chest = block(&state, "minecraft:chest", &[]);
    state.world.set_block(14, 201, 14, chest);
    state.block_entities.insert(
        (14, 201, 14),
        crate::tick::BlockEntityState {
            slots: vec![ItemStack::new(filler, 64); 27],
            furnace: None,
        },
    );
    state.settlements.get_mut(id).unwrap().bootstrap_storage = Some([14, 201, 14]);
    state.entities.set_pos(donor, [14.2, 201.0, 14.2]);
    let before = accounted(&state, id, &handle);
    let pickup = executor::pickup_point(
        &state,
        id,
        uuid,
        &handle,
        fence,
        state.entities.pos(builder),
    )
    .unwrap();
    assert_eq!(pickup.source, executor::PickupSource::Debtor(donor_uuid));
    assert_eq!(pickup.at, [14, 201, 14]);
    // Asking for the bank at that very cell cannot silently take the resident's
    // fences.
    assert_eq!(
        executor::fetch(
            &mut state,
            uuid,
            &handle,
            executor::Pickup::bank([14, 201, 14]),
            fence
        ),
        Err(mc_script::ScriptOperationFailure::InsufficientItems)
    );
    assert_eq!(carried_fences(&state, donor, fence), 4);
    state.entities.set_pos(builder, [13.5, 201.0, 14.5]);
    executor::fetch(&mut state, uuid, &handle, pickup, fence).unwrap();
    assert_eq!(carried_fences(&state, builder, fence), 4);
    assert_eq!(carried_fences(&state, donor, fence), 0);
    assert!(accounted(&state, id, &handle) == before);
}

#[test]
fn two_debtors_in_one_cell_debit_only_the_selected_body() {
    let (mut state, id, donor, builder, handle) =
        retained_release(crate::settlements::allocator::tests::test_state(), false);
    let fence = item(&state, "minecraft:acacia_fence");
    let donor_uuid = state.entities.row(donor).uuid;
    let uuid = state.entities.row(builder).uuid;
    state.entities.set_pos(donor, [14.5, 201.0, 14.5]);
    let other = villager(&mut state, [14.5, 201.0, 14.5]);
    let entity = state.entities.body(other).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    let other_uuid = state.entities.row(other).uuid;
    state
        .entities
        .row_mut(other)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(ItemStack::new(fence, 4));
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .carry_pending
        .insert(other_uuid, vec![(fence, 4)]);
    state.entities.set_pos(builder, [13.5, 201.0, 14.5]);
    let before = accounted(&state, id, &handle);
    executor::fetch(
        &mut state,
        uuid,
        &handle,
        executor::Pickup::debtor(donor_uuid, [14, 201, 14]),
        fence,
    )
    .unwrap();
    assert_eq!(carried_fences(&state, builder, fence), 4);
    assert_eq!(carried_fences(&state, donor, fence), 0);
    assert_eq!(
        carried_fences(&state, other, fence),
        4,
        "the co-located debtor is untouched"
    );
    assert_eq!(
        state.settlements.get(id).unwrap().carry_pending[&other_uuid],
        vec![(fence, 4)]
    );
    assert!(accounted(&state, id, &handle) == before);
}
