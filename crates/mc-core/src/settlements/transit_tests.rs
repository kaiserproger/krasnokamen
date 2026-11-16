//! Migration transit and emigration on real flat terrain with a real player's
//! simulation area: edge geometry, the leave/join halves, outside removal.
use super::*;
use crate::boundary::{ConnectionId, PlayerInput};
use crate::settlements::households::{HouseholdOrigin, form};
use std::sync::Arc;

const ANCHOR: [i32; 3] = [8, 64, 8];

/// Flat grass at y 63 (standable cells at y 64), one player standing at the
/// anchor with `simulation` chunks of entity simulation.
fn world(simulation: i32) -> TickState {
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let world = crate::horde_bench::flat_world(&registry);
    let mut state = TickState::new(registry, world, simulation, 512);
    crate::tick::run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(1),
            uuid: uuid::Uuid::from_u128(1),
            name: "Mayor".into(),
            permission_level: 0,
        }],
    );
    let player = state.connections.entity_of(ConnectionId(1)).unwrap();
    let slot = state.entities.slot_of(player).unwrap();
    state.entities.set_pos(slot, [8.5, 64.0, 8.5]);
    let wanted = state.desired_chunks();
    state.world.ensure_blocking(&wanted);
    let tick = state.tick + 1;
    state.refresh_active_chunks(tick);
    state
}

fn town(state: &mut TickState, name: &str, anchor: [i32; 3], radius: u32) -> SettlementId {
    state
        .settlements
        .create(
            &mut state.settlement_sequence,
            name.into(),
            "owner".into(),
            anchor,
            radius,
        )
        .unwrap()
}

fn resident(state: &mut TickState, settlement: SettlementId, pos: [f64; 3]) -> EntityId {
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:villager").unwrap())
        .unwrap();
    let id = state.spawn_mob(type_id, pos, 20.0);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.row_mut(slot).villager =
        Some(Box::new(crate::villager::VillagerState::default()));
    crate::settlements::bind(state, settlement, id).unwrap();
    id
}

fn horizontal(a: BlockPos, b: [i32; 3]) -> f64 {
    f64::from(a.x - b[0]).hypot(f64::from(a.z - b[2]))
}

#[test]
fn bearing_is_clockwise_from_north() {
    assert_eq!(bearing([0, 0, 0], [0, 0, -10]), 0.0);
    assert_eq!(bearing([0, 0, 0], [10, 0, 0]), 90.0);
    assert_eq!(bearing([0, 0, 0], [0, 0, 10]), 180.0);
    assert_eq!(bearing([0, 0, 0], [-10, 0, 0]), 270.0);
}

/// Owner decision 21:50: the exit lies on the simulated edge even when that is
/// well inside a large civil radius. Control: no bearing reaches a point
/// outside radius 200 with one player at simulation distance 4.
#[test]
fn edge_point_is_the_simulated_edge_inside_a_large_radius() {
    let mut state = world(4);
    let town = town(&mut state, "wide", ANCHOR, 200);
    assert!(state.settlements.get(town).is_some());
    // Simulation distance 4 around chunk (0, 0): blocks −64..=79 per axis.
    let east = edge_point(&state, ANCHOR, 90.0).expect("east edge");
    assert_eq!((east.x, east.y, east.z), (79, 64, 8));
    assert!(state.world.is_active(east.x.div_euclid(16), 0));
    assert!(!state.world.is_active((east.x + 1).div_euclid(16), 0));
    let north = edge_point(&state, ANCHOR, 0.0).expect("north edge");
    assert_eq!((north.x, north.y, north.z), (8, 64, -64));
    for step in 0..24 {
        let point = edge_point(&state, ANCHOR, step as f32 * 15.0).expect("edge on every bearing");
        let distance = horizontal(point, ANCHOR);
        assert!(distance >= f64::from(MIN_EDGE_DISTANCE));
        assert!(
            distance <= 200.0,
            "bearing {}: {distance} blocks is outside the simulated area",
            step * 15
        );
    }
}

#[test]
fn edge_point_needs_simulated_ground_beyond_the_minimum_distance() {
    let state = world(0);
    // Only the anchor chunk simulates: nothing 16 blocks out on any bearing.
    assert!((0..8).all(|step| edge_point(&state, ANCHOR, step as f32 * 45.0).is_none()));
}

#[test]
fn lone_resident_leaves_and_joins_with_counts_and_claims_released() {
    let mut state = world(4);
    let source = town(&mut state, "source", ANCHOR, 48);
    let destination = town(&mut state, "destination", [40, 64, 40], 48);
    let lone = resident(&mut state, source, [12.5, 64.0, 12.5]);
    let slot = state.entities.slot_of(lone).unwrap();
    state.entities.row_mut(slot).villager.as_mut().unwrap().home =
        Some(BlockPos { x: 1, y: 64, z: 1 });
    assert_eq!(crate::settlements::member_count(&state, source), 1);
    let departed = households::leave(&mut state, source, Unit::Resident(lone)).unwrap();
    assert_eq!(departed.household, None);
    assert_eq!(departed.members, vec![lone]);
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!((v.settlement, v.home), (None, None));
    assert_eq!(crate::settlements::member_count(&state, source), 0);
    households::join(&mut state, destination, departed).unwrap();
    let slot = state.entities.slot_of(lone).unwrap();
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .settlement,
        Some(destination)
    );
    assert_eq!(crate::settlements::member_count(&state, destination), 1);
    crate::settlements::validate_memberships(&state).unwrap();
}

#[test]
fn a_household_member_cannot_leave_alone() {
    let mut state = world(4);
    let source = town(&mut state, "source", ANCHOR, 48);
    let a = resident(&mut state, source, [12.5, 64.0, 12.5]);
    let b = resident(&mut state, source, [13.5, 64.0, 12.5]);
    form(&mut state, source, &[a, b], HouseholdOrigin::Adoption).unwrap();
    assert_eq!(
        households::leave(&mut state, source, Unit::Resident(a)),
        Err("resident belongs to a household; move the household")
    );
}

#[test]
fn emigration_removes_the_household_bodies_and_record_in_one_call() {
    let mut state = world(4);
    let source = town(&mut state, "source", ANCHOR, 48);
    let exit = BlockPos { x: 30, y: 64, z: 8 };
    let a = resident(&mut state, source, [30.5, 64.0, 8.5]);
    let b = resident(&mut state, source, [31.5, 64.0, 8.5]);
    let stay = resident(&mut state, source, [10.5, 64.0, 10.5]);
    let family = form(&mut state, source, &[a, b], HouseholdOrigin::Adoption).unwrap();
    let uuids: Vec<_> = [a, b]
        .map(|id| state.entities.row(state.entities.slot_of(id).unwrap()).uuid)
        .into();
    let mut changes = ChangeSet::default();
    emigrate(
        &mut state,
        &mut changes,
        source,
        Unit::Household(&family),
        exit,
    )
    .unwrap();
    assert_eq!(changes.entities_removed, vec![a, b]);
    for uuid in uuids {
        assert!(state.entities.slot_of_uuid(uuid).is_none());
    }
    assert!(state.entities.slot_of(stay).is_some());
    let record = state.settlements.get(source).unwrap();
    assert!(!record.households.contains_key(&family));
    assert_eq!(crate::settlements::member_count(&state, source), 1);
    crate::settlements::validate_memberships(&state).unwrap();
}

/// Negative controls: stock never leaves the world, and a household is never
/// half-taken (a member away from the exit refuses the whole unit).
#[test]
fn emigration_refuses_carried_items_and_absent_members_without_mutation() {
    let mut state = world(4);
    let source = town(&mut state, "source", ANCHOR, 48);
    let exit = BlockPos { x: 30, y: 64, z: 8 };
    let a = resident(&mut state, source, [30.5, 64.0, 8.5]);
    let b = resident(&mut state, source, [31.5, 64.0, 8.5]);
    let family = form(&mut state, source, &[a, b], HouseholdOrigin::Adoption).unwrap();
    let bread = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:bread").unwrap())
        .unwrap();
    let slot = state.entities.slot_of(b).unwrap();
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(mc_data::ItemStack::new(bread, 3));
    let revision = state.settlements.get(source).unwrap().revision;
    let mut changes = ChangeSet::default();
    assert_eq!(
        emigrate(
            &mut state,
            &mut changes,
            source,
            Unit::Household(&family),
            exit
        ),
        Err("an emigrant still carries items")
    );
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = None;
    state.entities.set_pos(slot, [36.5, 64.0, 8.5]);
    assert_eq!(
        emigrate(
            &mut state,
            &mut changes,
            source,
            Unit::Household(&family),
            exit
        ),
        Err("a member is not at the exit point")
    );
    assert!(changes.entities_removed.is_empty());
    let record = state.settlements.get(source).unwrap();
    assert_eq!(record.revision, revision);
    assert!(record.households.contains_key(&family));
    assert_eq!(crate::settlements::member_count(&state, source), 2);
}
