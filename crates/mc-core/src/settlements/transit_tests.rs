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
    mth::length2(f64::from(a.x - b[0]), f64::from(a.z - b[2]))
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

fn uuid_of(state: &TickState, id: EntityId) -> uuid::Uuid {
    state.entities.row(state.entities.slot_of(id).unwrap()).uuid
}

/// Places holding `uuid`: world entities plus transit members, every dimension.
fn places(state: &TickState, uuid: uuid::Uuid) -> usize {
    let text = uuid.to_string();
    let bodies = worlds(state)
        .filter(|world| world.entities.slot_of_uuid(uuid).is_some())
        .count();
    let records = worlds(state)
        .flat_map(|world| world.settlements.transit.records.values())
        .filter(|record| record.members.iter().any(|member| member.uuid == text))
        .count();
    bodies + records
}

fn player_to(state: &mut TickState, at: [f64; 3]) {
    let player = state.connections.entity_of(ConnectionId(1)).unwrap();
    let slot = state.entities.slot_of(player).unwrap();
    state.entities.set_pos(slot, at);
    let wanted = state.desired_chunks();
    state.world.ensure_blocking(&wanted);
    let tick = state.tick;
    state.refresh_active_chunks(tick);
}

const FAR: [i32; 3] = [1000, 64, 8];

struct Departure {
    state: TickState,
    source: SettlementId,
    destination: SettlementId,
    family: String,
    uuids: [uuid::Uuid; 2],
    id: TransitId,
    exit: BlockPos,
    /// Each member's complete entity image just before `depart`.
    before: [serde_json::Value; 2],
}

fn image(state: &TickState, uuid: uuid::Uuid) -> serde_json::Value {
    let slot = state.entities.slot_of_uuid(uuid).unwrap();
    serde_json::to_value(capture(state, slot)).unwrap()
}

/// Paths at which two images differ, descending into nested objects.
fn differing(a: &serde_json::Value, b: &serde_json::Value, path: &str, out: &mut Vec<String>) {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for key in a.keys().chain(b.keys().filter(|key| !a.contains_key(*key))) {
                let null = serde_json::Value::Null;
                differing(
                    a.get(key).unwrap_or(&null),
                    b.get(key).unwrap_or(&null),
                    &format!("{path}.{key}"),
                    out,
                );
            }
        }
        _ if a != b => out.push(path.to_owned()),
        _ => {}
    }
}

/// A traded level-2 librarian carrying bread and a child, one household of a
/// radius-200 town, standing at the simulated edge on the bearing to a
/// radius-200 town 992 blocks east; `depart` has run.
fn departed() -> Departure {
    let mut state = world(4);
    let source = town(&mut state, "source", ANCHOR, 200);
    let destination = town(&mut state, "destination", FAR, 200);
    let toward = bearing(ANCHOR, FAR);
    let exit = edge_point(&state, ANCHOR, toward).expect("exit at the simulated edge");
    assert!(
        horizontal(exit, ANCHOR) < 200.0,
        "the exit lies inside the radius"
    );
    let librarian = resident(&mut state, source, [10.5, 64.0, 10.5]);
    let child = resident(&mut state, source, [11.5, 64.0, 10.5]);
    let item = |state: &TickState, name: &str| {
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap()
    };
    let (bread, emerald, book) = (
        item(&state, "minecraft:bread"),
        item(&state, "minecraft:emerald"),
        item(&state, "minecraft:book"),
    );
    let slot = state.entities.slot_of(librarian).unwrap();
    let v = state.entities.row_mut(slot).villager.as_mut().unwrap();
    v.data.profession = crate::villager::VillagerProfession::Librarian;
    v.data.level = 2;
    v.xp = 12;
    v.inventory[0] = Some(mc_data::ItemStack::new(bread, 5));
    v.offers.push(crate::villager::VillagerTradeOffer {
        cost_a: mc_data::ItemStack::new(emerald, 9),
        cost_b: Some(mc_data::ItemStack::new(book, 1)),
        result: mc_data::ItemStack::new(book, 1),
        uses: 3,
        max_uses: 12,
        reward_exp: true,
        xp: 2,
        special_price: -1,
        demand: 4,
        price_multiplier: 0.2,
    });
    crate::villager::sync_appearance(&mut state, slot);
    let slot = state.entities.slot_of(child).unwrap();
    state.entities.row_mut(slot).villager.as_mut().unwrap().age = -12_000;
    crate::villager::sync_appearance(&mut state, slot);
    let family = form(
        &mut state,
        source,
        &[librarian, child],
        HouseholdOrigin::Adoption,
    )
    .unwrap();
    let centre = [
        f64::from(exit.x) + 0.5,
        f64::from(exit.y),
        f64::from(exit.z) + 0.5,
    ];
    for id in [librarian, child] {
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.set_pos(slot, centre);
    }
    let uuids = [uuid_of(&state, librarian), uuid_of(&state, child)];
    let before = uuids.map(|uuid| image(&state, uuid));
    let mut changes = ChangeSet::default();
    let id = depart(
        &mut state,
        &mut changes,
        source,
        Unit::Household(&family),
        destination,
        exit,
        toward,
    )
    .unwrap();
    assert_eq!(changes.entities_removed, vec![librarian, child]);
    Departure {
        state,
        source,
        destination,
        family,
        uuids,
        id,
        exit,
        before,
    }
}

#[test]
fn departure_moves_the_household_bodies_into_one_record_in_one_call() {
    let d = departed();
    let state = &d.state;
    assert_eq!(d.id.0, *d.uuids.iter().min().unwrap());
    for uuid in d.uuids {
        assert!(state.entities.slot_of_uuid(uuid).is_none());
        assert_eq!(places(state, uuid), 1);
    }
    let record = state.settlements.transit.get(d.id).unwrap();
    assert_eq!(record.members.len(), 2);
    assert_eq!(
        record.household.as_ref().map(|family| family.id.as_str()),
        Some(d.family.as_str())
    );
    assert_eq!(record.status, Status::Travelling);
    assert_eq!(record.exit, [d.exit.x, d.exit.y, d.exit.z]);
    let distance = horizontal(d.exit, FAR).ceil() as u64;
    assert_eq!(record.arrives, record.departed + TICKS_PER_BLOCK * distance);
    let source = state.settlements.get(d.source).unwrap();
    assert!(!source.households.contains_key(&d.family));
    assert_eq!(crate::settlements::member_count(state, d.source), 0);
    assert_eq!(inbound(state, d.destination), 2);
    assert_eq!(summary(state, d.source).len(), 1);
    assert_eq!(summary(state, d.destination)[0].members, 2);
    crate::settlements::validate_memberships(state).unwrap();
    validate_custody(state).unwrap();
}

/// Owner decision 21:50 with radius 200 and simulation distance 4: departure
/// and arrival both happen at the simulated edge, inside the civil radius.
#[test]
fn arrival_reenters_the_same_villagers_at_the_simulated_edge_and_joins() {
    let mut d = departed();
    let arrives = d.state.settlements.transit.get(d.id).unwrap().arrives;
    d.state.tick = arrives;
    // The destination is not simulated: the record waits, nobody appears.
    let mut changes = ChangeSet::default();
    assert!(tick(&mut d.state, &mut changes).is_empty());
    let record = d.state.settlements.transit.get(d.id).unwrap();
    assert_eq!(record.status, Status::AwaitingEntry);
    assert!(changes.entities_spawned.is_empty());
    assert_eq!(inbound(&d.state, d.destination), 2);
    // A player reaches the destination: it simulates, the household enters.
    player_to(&mut d.state, [1000.5, 64.0, 8.5]);
    let arrivals = tick(&mut d.state, &mut changes);
    assert_eq!(changes.entities_spawned.len(), 2);
    assert!(d.state.settlements.transit.get(d.id).is_none());
    let mut members = d.uuids.to_vec();
    members.sort();
    assert_eq!(
        arrivals,
        [Arrival {
            source: d.source,
            destination: d.destination,
            household: Some(d.family.clone()),
            members,
        }]
    );
    assert_eq!(inbound(&d.state, d.destination), 0);
    let entry = edge_point(&d.state, FAR, 270.0).unwrap();
    let distance = horizontal(entry, FAR);
    assert!((f64::from(MIN_EDGE_DISTANCE)..200.0).contains(&distance));
    for uuid in d.uuids {
        assert_eq!(places(&d.state, uuid), 1);
        let slot = d.state.entities.slot_of_uuid(uuid).unwrap();
        let pos = d.state.entities.pos(slot);
        assert!(
            mth::length2(
                pos[0] - f64::from(entry.x) - 0.5,
                pos[2] - f64::from(entry.z) - 0.5
            ) < 0.01
        );
        let v = d.state.entities.row(slot).villager.as_ref().unwrap();
        assert_eq!(v.settlement, Some(d.destination));
        assert_eq!(v.household.as_deref(), Some(d.family.as_str()));
    }
    // Each body is the one that left, field by field: only the re-entry
    // motion (position, velocity, ground, image tick) and the settlement.
    let moved = [".pos", ".vel", ".ground", ".tick", ".villager.settlement"];
    for (uuid, before) in d.uuids.into_iter().zip(&d.before) {
        let mut diff = Vec::new();
        differing(before, &image(&d.state, uuid), "", &mut diff);
        diff.retain(|path| !moved.contains(&path.as_str()));
        assert!(diff.is_empty(), "{uuid} changed in transit: {diff:?}");
    }
    let librarian = d.state.entities.slot_of_uuid(d.uuids[0]).unwrap();
    let librarian = d.state.entities.row(librarian).villager.as_ref().unwrap();
    assert_eq!(
        librarian.data.profession,
        crate::villager::VillagerProfession::Librarian
    );
    assert_eq!((librarian.data.level, librarian.xp), (2, 12));
    assert_eq!(librarian.offers.len(), 1);
    assert_eq!(
        librarian.inventory[0].as_ref().map(|stack| stack.count),
        Some(5)
    );
    let child = d.state.entities.slot_of_uuid(d.uuids[1]).unwrap();
    assert_eq!(
        d.state.entities.row(child).villager.as_ref().unwrap().age,
        -12_000
    );
    let destination = d.state.settlements.get(d.destination).unwrap();
    assert!(destination.households.contains_key(&d.family));
    assert_eq!(crate::settlements::member_count(&d.state, d.destination), 2);
    crate::settlements::validate_memberships(&d.state).unwrap();
    validate_custody(&d.state).unwrap();
}

#[test]
fn a_deleted_destination_retargets_the_record_to_its_source() {
    let mut d = departed();
    assert!(crate::settlements::remove(&mut d.state, d.destination));
    d.state.tick = d.state.settlements.transit.get(d.id).unwrap().arrives;
    let mut changes = ChangeSet::default();
    let [arrival] = tick(&mut d.state, &mut changes).try_into().unwrap();
    assert_eq!(changes.entities_spawned.len(), 2);
    assert_eq!((arrival.source, arrival.destination), (d.source, d.source));
    for uuid in d.uuids {
        assert_eq!(places(&d.state, uuid), 1);
        let slot = d.state.entities.slot_of_uuid(uuid).unwrap();
        let v = d.state.entities.row(slot).villager.as_ref().unwrap();
        assert_eq!(v.settlement, Some(d.source));
    }
    let source = d.state.settlements.get(d.source).unwrap();
    assert!(source.households.contains_key(&d.family));
}

/// Negative controls: a member that is also a body refuses recovery, and an
/// arrival never re-inserts a UUID that already lives.
#[test]
fn a_member_that_is_also_a_body_refuses_recovery_and_arrival() {
    let mut d = departed();
    validate_custody(&d.state).unwrap();
    let image = d.state.settlements.transit.get(d.id).unwrap().members[0].clone();
    let entity = d.state.entities.alloc_id();
    crate::core::restore_entity(&mut d.state, entity.0, &image).unwrap();
    let error = validate_custody(&d.state).unwrap_err().to_string();
    assert!(
        error.contains("both a world entity and in transit record"),
        "{error}"
    );
    d.state.tick = d.state.settlements.transit.get(d.id).unwrap().arrives;
    player_to(&mut d.state, [1000.5, 64.0, 8.5]);
    let population = d.state.entities.len();
    let mut changes = ChangeSet::default();
    tick(&mut d.state, &mut changes);
    assert!(changes.entities_spawned.is_empty());
    assert_eq!(d.state.entities.len(), population);
    assert!(d.state.settlements.transit.get(d.id).is_some());
}

#[test]
fn departure_refuses_a_member_with_wages_owed_without_mutation() {
    let mut state = world(4);
    let source = town(&mut state, "source", ANCHOR, 48);
    let destination = town(&mut state, "destination", FAR, 48);
    let exit = BlockPos { x: 30, y: 64, z: 8 };
    let worker = resident(&mut state, source, [30.5, 64.0, 8.5]);
    let payer = resident(&mut state, source, [10.5, 64.0, 10.5]);
    let (worker_uuid, payer_uuid) = (uuid_of(&state, worker), uuid_of(&state, payer));
    state
        .settlements
        .get_mut(source)
        .unwrap()
        .economy
        .commerce
        .employments
        .insert(
            "1".into(),
            crate::settlements::commerce::Employment {
                worker: worker_uuid,
                treasury: crate::settlements::inventory::Endpoint::VillagerCarry {
                    uuid: payer_uuid,
                },
                wage: 1,
                period_ticks: 5,
                next_due: 5,
                legacy_owed: 0,
                budget: 10,
                earned: 1,
                paid: 0,
                active: false,
                attempt: 0,
                settled: None,
            },
        );
    let revision = state.settlements.get(source).unwrap().revision;
    let mut changes = ChangeSet::default();
    assert_eq!(
        depart(
            &mut state,
            &mut changes,
            source,
            Unit::Resident(worker),
            destination,
            exit,
            90.0,
        ),
        Err("a member holds an economic reservation")
    );
    assert!(changes.entities_removed.is_empty());
    assert_eq!(state.settlements.get(source).unwrap().revision, revision);
    assert!(state.settlements.transit.records.is_empty());
    assert_eq!(places(&state, worker_uuid), 1);
}

/// Main 2026-11-16: `leave` would turn a Builder into a mason or a jobless
/// villager, so a unit with construction work is refused before any mutation
/// by departure, emigration and the operator command alike.
#[test]
fn a_member_with_construction_work_is_refused_without_mutation() {
    use crate::villager::VillagerProfession as Profession;
    const REFUSAL: &str = "member has construction work; finish or release it first";
    let mut state = world(4);
    let source = town(&mut state, "source", ANCHOR, 48);
    let destination = town(&mut state, "destination", FAR, 48);
    let exit = BlockPos { x: 30, y: 64, z: 8 };
    let builder = resident(&mut state, source, [30.5, 64.0, 8.5]);
    let partner = resident(&mut state, source, [30.5, 64.0, 9.0]);
    let family = form(
        &mut state,
        source,
        &[builder, partner],
        HouseholdOrigin::Adoption,
    )
    .unwrap();
    let builder_uuid = uuid_of(&state, builder);
    let slot = state.entities.slot_of(builder).unwrap();
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Builder;
    let order = super::super::construction::Project {
        settlement: source,
        blueprint: "house".into(),
        level: 1,
        origin: [20, 64, 20],
        rotation: 0,
        building: None,
        structure: "order".into(),
        ordered: true,
        borrowed_builder: Some((builder_uuid, Profession::Mason)),
    };
    state.settlements.get_mut(source).unwrap().project = Some(order.clone());
    let revision = state.settlements.get(source).unwrap().revision;
    let before = [builder, partner].map(|id| image(&state, uuid_of(&state, id)));
    let mut changes = ChangeSet::default();
    let unit = Unit::Household(&family);
    let refused = |state: &mut TickState, changes: &mut ChangeSet| {
        [
            depart(state, changes, source, unit, destination, exit, 90.0).err(),
            emigrate(state, changes, source, unit, exit).err(),
            command_depart(state, changes, source, &family, destination).err(),
        ]
    };
    assert_eq!(refused(&mut state, &mut changes), [Some(REFUSAL); 3]);
    // The profession alone (the order released) is still construction work.
    state.settlements.get_mut(source).unwrap().project = None;
    assert_eq!(refused(&mut state, &mut changes), [Some(REFUSAL); 3]);
    state.settlements.get_mut(source).unwrap().project = Some(order.clone());
    assert!(changes.entities_removed.is_empty() && changes.entities_spawned.is_empty());
    let record = state.settlements.get(source).unwrap();
    assert_eq!(record.revision, revision);
    assert_eq!(record.project, Some(order));
    assert!(record.households.contains_key(&family));
    assert!(state.settlements.transit.records.is_empty());
    for (id, before) in [builder, partner].into_iter().zip(before) {
        assert_eq!(image(&state, uuid_of(&state, id)), before);
    }
}

#[test]
fn operator_command_departs_from_where_the_household_stands() {
    let mut state = world(4);
    let source = town(&mut state, "source", ANCHOR, 48);
    let destination = town(&mut state, "destination", FAR, 48);
    let a = resident(&mut state, source, [30.5, 64.0, 8.5]);
    let b = resident(&mut state, source, [31.0, 64.0, 8.5]);
    let family = form(&mut state, source, &[a, b], HouseholdOrigin::Adoption).unwrap();
    let mut changes = ChangeSet::default();
    let line = command_depart(&mut state, &mut changes, source, &family, destination).unwrap();
    assert!(line.contains(&family), "{line}");
    assert_eq!(changes.entities_removed.len(), 2);
    let list = command_list(&state);
    assert_eq!(list.len(), 1);
    assert!(
        list[0].contains("travelling") && list[0].contains(&family),
        "{list:?}"
    );
}

#[test]
fn migration_roster_partial_arrival_rollback_keeps_live_brains_and_sensors() {
    let mut d = departed();
    d.state.tick = d.state.settlements.transit.get(d.id).unwrap().arrives;
    player_to(&mut d.state, [1000.5, 64.0, 8.5]);
    let stay = resident(&mut d.state, d.destination, [1000.5, 64.0, 10.5]);
    let item =
        crate::villager::brain::roster_tests::dropped_item(&mut d.state, [1001.5, 64.0, 10.5]);
    let mut changes = ChangeSet::default();
    crate::settlements::allocator::tick(&mut d.state, &mut changes);
    // The first member restores; the second refuses before it can spawn.
    // Corrupt only this test's live record, not a production recovery check.
    let record = d
        .state
        .settlements
        .transit
        .records
        .get_mut(&d.id.0)
        .unwrap();
    record.members[1].type_id = u32::MAX;
    let first = record.members[0].uuid.parse::<uuid::Uuid>().unwrap();
    let before = d.state.entities.len();
    let revision = d.state.entities.brain_revision();
    assert!(tick(&mut d.state, &mut changes).is_empty());
    assert!(d.state.entities.brain_revision() > revision);
    assert_eq!(d.state.entities.len(), before);
    assert!(d.state.entities.slot_of_uuid(first).is_none());
    assert!(d.state.settlements.transit.get(d.id).is_some());
    assert!(changes.entities_spawned.is_empty());
    let executed = crate::villager::brain::roster_tests::check_tick(&mut d.state, &mut changes);
    assert_eq!(executed, [stay]);
    let slot = d.state.entities.slot_of(item).unwrap();
    assert!(
        d.state
            .villager_pois
            .items_near([1001.5, 64.0, 10.5])
            .contains(&slot)
    );
}

fn review_island_transit() -> (TickState, TransitId, SettlementId, uuid::Uuid) {
    let mut state = world(0);
    let source = town(&mut state, "source", ANCHOR, 200);
    let destination = town(&mut state, "destination", FAR, 200);
    let member = resident(&mut state, source, [8.5, 64.0, 8.5]);
    let uuid = uuid_of(&state, member);
    // The real operator departure accepts the unit's current standing cell.
    let id = depart(
        &mut state,
        &mut ChangeSet::default(),
        source,
        Unit::Resident(member),
        destination,
        BlockPos { x: 8, y: 64, z: 8 },
        90.0,
    )
    .unwrap();
    state.tick = state.settlements.transit.get(id).unwrap().arrives;
    player_to(&mut state, [1000.5, 64.0, 8.5]);
    assert!(edge_point(&state, FAR, 270.0).is_none());
    (state, id, destination, uuid)
}

#[test]
fn review_transit_island_arrives_at_nearest_anchor_cell_without_return_bearing() {
    let (mut state, id, destination, uuid) = review_island_transit();
    let mut changes = ChangeSet::default();
    let arrivals = tick(&mut state, &mut changes);
    assert_eq!(arrivals.len(), 1);
    assert_eq!(arrivals[0].destination, destination);
    assert!(state.settlements.transit.get(id).is_none());
    let slot = state.entities.slot_of_uuid(uuid).unwrap();
    assert_eq!(state.entities.pos(slot), [1000.5, 64.0, 8.5]);
    assert_eq!(state.settlements.transit.entry_attempts, 1);
    validate_custody(&state).unwrap();
}

#[test]
fn review_transit_missing_ground_backs_off_without_persisting_or_rescanning() {
    let (mut state, id, _, uuid) = review_island_transit();
    for x in FAR[0] - 5..=FAR[0] + 5 {
        for z in FAR[2] - 5..=FAR[2] + 5 {
            for y in 58..=67 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    state.world.fold_overlay();
    let before = serde_json::to_value(state.settlements.transit.get(id).unwrap()).unwrap();
    let mut entries = Vec::new();
    let at = state.tick;
    journal_entries(&mut state, at, &mut entries);
    let mut changes = ChangeSet::default();
    assert!(tick(&mut state, &mut changes).is_empty());
    let deadline = state.settlements.transit.get(id).unwrap().retry_at;
    assert_eq!(deadline, game_time(&state) + ARRIVAL_RETRY_TICKS);
    assert_eq!(state.settlements.transit.entry_attempts, 1);
    assert_eq!(
        before,
        serde_json::to_value(state.settlements.transit.get(id).unwrap()).unwrap()
    );
    entries.clear();
    journal_entries(&mut state, at, &mut entries);
    assert!(entries.is_empty());
    let stone = state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap()
        .default
        .0;
    state.world.set_block(FAR[0], 63, FAR[2], stone);
    state.world.fold_overlay();
    for _ in 1..ARRIVAL_RETRY_TICKS {
        state.tick += 1;
        assert!(tick(&mut state, &mut changes).is_empty());
        assert_eq!(state.settlements.transit.entry_attempts, 1);
        assert!(state.entities.slot_of_uuid(uuid).is_none());
    }
    state.tick += 1;
    assert_eq!(tick(&mut state, &mut changes).len(), 1);
    assert_eq!(state.settlements.transit.entry_attempts, 2);
    assert!(state.settlements.transit.get(id).is_none());
}

#[test]
fn review_transit_restore_refusal_waits_before_retrying_partial_arrival() {
    let mut d = departed();
    d.state.tick = d.state.settlements.transit.get(d.id).unwrap().arrives;
    player_to(&mut d.state, [1000.5, 64.0, 8.5]);
    let original_type = d.state.settlements.transit.records[&d.id.0].members[1].type_id;
    d.state
        .settlements
        .transit
        .records
        .get_mut(&d.id.0)
        .unwrap()
        .members[1]
        .type_id = u32::MAX;
    let mut changes = ChangeSet::default();
    assert!(tick(&mut d.state, &mut changes).is_empty());
    assert!(changes.entities_spawned.is_empty());
    let revision = d.state.entities.projection_revision();
    let deadline = d.state.settlements.transit.get(d.id).unwrap().retry_at;
    for _ in 1..ARRIVAL_RETRY_TICKS {
        d.state.tick += 1;
        assert!(tick(&mut d.state, &mut changes).is_empty());
        assert_eq!(d.state.entities.projection_revision(), revision);
        assert_eq!(d.state.settlements.transit.entry_attempts, 1);
    }
    // Repair only the live test record; recovery rejects this corruption.
    d.state
        .settlements
        .transit
        .records
        .get_mut(&d.id.0)
        .unwrap()
        .members[1]
        .type_id = original_type;
    d.state.tick += 1;
    assert_eq!(game_time(&d.state), deadline);
    assert_eq!(tick(&mut d.state, &mut changes).len(), 1);
    assert_eq!(d.state.settlements.transit.entry_attempts, 2);
    for uuid in d.uuids {
        assert_eq!(places(&d.state, uuid), 1);
    }
}

#[test]
fn review_transit_refusal_budget_does_not_starve_older_waiting_arrivals() {
    let mut state = world(0);
    let source = town(&mut state, "source", ANCHOR, 200);
    let destination = town(&mut state, "destination", FAR, 200);
    let mut ids = Vec::new();
    for _ in 0..MAX_ARRIVALS_PER_TICK + 1 {
        let member = resident(&mut state, source, [8.5, 64.0, 8.5]);
        let id = depart(
            &mut state,
            &mut ChangeSet::default(),
            source,
            Unit::Resident(member),
            destination,
            BlockPos { x: 8, y: 64, z: 8 },
            90.0,
        )
        .unwrap();
        ids.push(id.0);
    }
    ids.sort_unstable();
    let healthy = *ids.last().unwrap();
    for id in ids.iter().take(MAX_ARRIVALS_PER_TICK) {
        state
            .settlements
            .transit
            .records
            .get_mut(id)
            .unwrap()
            .members[0]
            .type_id = u32::MAX;
    }
    state.tick = state.settlements.transit.records[&healthy].arrives;
    player_to(&mut state, [1000.5, 64.0, 8.5]);
    let mut changes = ChangeSet::default();
    assert!(tick(&mut state, &mut changes).is_empty());
    assert_eq!(
        state.settlements.transit.entry_attempts,
        MAX_ARRIVALS_PER_TICK
    );
    assert_eq!(state.settlements.transit.records[&healthy].retry_at, 0);
    state.tick += ARRIVAL_RETRY_TICKS;
    assert_eq!(tick(&mut state, &mut changes).len(), 1);
    assert!(!state.settlements.transit.records.contains_key(&healthy));
    assert_eq!(
        state.settlements.transit.entry_attempts,
        MAX_ARRIVALS_PER_TICK * 2
    );
}
