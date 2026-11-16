use super::return_tests::{fixture, house, resident, step, town};
use super::*;
use crate::boundary::ChunkPos;
use crate::villager::VillagerProfession;

fn observe(state: &mut TickState, id: SettlementId) {
    for _ in 0..10 {
        step(state);
    }
    let observed = facts(state, id, state.tick);
    state
        .settlements
        .migration
        .observe(id, state.tick, observed);
}

#[test]
fn loaded_reachable_family_relocates_without_a_transit_record_or_teleport() {
    let mut state = fixture();
    let source = town(&mut state, "source", 32, 32);
    let destination = town(&mut state, "destination", 8, 32);
    house(&mut state, destination, 8);
    let a = resident(&mut state, source, 31.5);
    let b = resident(&mut state, source, 32.5);
    let key = households::form(
        &mut state,
        source,
        &[a, b],
        households::HouseholdOrigin::Adoption,
    )
    .unwrap();
    observe(&mut state, source);
    observe(&mut state, destination);
    let positions = [a, b].map(|id| state.entities.pos(state.entities.slot_of(id).unwrap()));
    depart_decision(&mut state, source, &mut Rng(123));
    assert!(
        !state
            .settlements
            .get(source)
            .unwrap()
            .households
            .contains_key(&key)
    );
    assert!(
        state
            .settlements
            .get(destination)
            .unwrap()
            .households
            .contains_key(&key)
    );
    assert!(transit::summary(&state, source).is_empty());
    for (id, position) in [a, b].into_iter().zip(positions) {
        let slot = state.entities.slot_of(id).unwrap();
        assert_eq!(state.entities.pos(slot), position);
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
    }
    assert!(info(&state, source).contains("departure"));
    assert!(info(&state, destination).contains("arrival"));
}

#[test]
fn founding_conversion_and_builders_refuse_before_any_membership_mutation() {
    let mut state = fixture();
    let source = town(&mut state, "source", 32, 32);
    let destination = town(&mut state, "destination", 8, 32);
    house(&mut state, destination, 8);
    let a = resident(&mut state, source, 31.5);
    let b = resident(&mut state, source, 32.5);
    let key = households::form(
        &mut state,
        source,
        &[a, b],
        households::HouseholdOrigin::Adoption,
    )
    .unwrap();
    let unit = Unit::Household(key.clone());
    state.settlements.get_mut(source).unwrap().founding =
        Some(super::super::construction::FoundingExpedition {
            destination: [200, 63, 8],
            household: key.clone(),
        });
    assert_eq!(
        eligible(&state, source, &unit, false),
        Err("household leads a founding expedition")
    );
    state.settlements.get_mut(source).unwrap().founding = None;
    let slot = state.entities.slot_of(a).unwrap();
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .conversion_time = Some(100);
    assert!(eligible(&state, source, &unit, false).is_err());
    let v = state.entities.row_mut(slot).villager.as_mut().unwrap();
    v.conversion_time = None;
    v.data.profession = VillagerProfession::Builder;
    assert!(eligible(&state, source, &unit, false).is_err());
    assert!(
        state
            .settlements
            .get(source)
            .unwrap()
            .households
            .contains_key(&key)
    );
    assert!(
        !state
            .settlements
            .get(destination)
            .unwrap()
            .households
            .contains_key(&key)
    );
    assert_eq!(crate::settlements::member_count(&state, source), 2);
    assert!(state.settlements.migration.walking.is_empty());
}

#[test]
fn inactive_destination_walks_to_edge_inside_radius_then_transits_same_people() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 200);
    let destination = town(&mut state, "destination", 208, 32);
    house(&mut state, destination, 208);
    let a = resident(&mut state, source, 10.5);
    let b = resident(&mut state, source, 11.5);
    let key = households::form(
        &mut state,
        source,
        &[a, b],
        households::HouseholdOrigin::Adoption,
    )
    .unwrap();
    observe(&mut state, source);
    observe(&mut state, destination);
    let uuids = [a, b].map(|id| state.entities.row(state.entities.slot_of(id).unwrap()).uuid);
    state.tickets.clear();
    state
        .tickets
        .insert("source-only".into(), (ChunkPos { x: 0, z: 0 }, 2));
    let tick = state.tick;
    state.refresh_active_chunks(tick);
    state.tick_budget.reset(state.tick);
    depart_decision(&mut state, source, &mut Rng(123));
    let exit = state.settlements.migration.walking[&source].exit;
    assert!(
        state
            .settlements
            .get(source)
            .unwrap()
            .contains([exit.x, exit.y, exit.z])
    );
    assert_eq!(
        capacity(&state, destination),
        0,
        "pending household reserves its two beds"
    );
    let mut changes = ChangeSet::default();
    for _ in 0..1500 {
        step(&mut state);
        advance_walks(&mut state, &mut changes);
        if !transit::summary(&state, source).is_empty() {
            break;
        }
    }
    assert_eq!(changes.entities_removed.len(), 2);
    assert!(
        !state
            .settlements
            .get(source)
            .unwrap()
            .households
            .contains_key(&key)
    );
    for uuid in uuids {
        assert!(state.entities.slot_of_uuid(uuid).is_none());
    }
    let summary = transit::summary(&state, destination);
    assert_eq!(summary.len(), 1);
    state.tick = summary[0].arrives + 1;
    let arrivals = transit::tick(&mut state, &mut changes);
    assert!(arrivals.is_empty());
    assert_eq!(
        transit::summary(&state, destination)[0].status,
        transit::Status::AwaitingEntry
    );
    state
        .tickets
        .insert("destination".into(), (ChunkPos { x: 13, z: 0 }, 2));
    let tick = state.tick;
    state.refresh_active_chunks(tick);
    let arrivals = transit::tick(&mut state, &mut changes);
    assert_eq!(arrivals.len(), 1);
    assert_eq!(arrivals[0].members.len(), 2);
    super::tick(&mut state, &mut changes, &arrivals);
    for uuid in uuids {
        let slot = state
            .entities
            .slot_of_uuid(uuid)
            .expect("same UUID re-enters");
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
    }
    assert!(info(&state, destination).contains("arrived from"));
}

#[test]
fn outside_arrivals_are_singleton_free_and_rate_is_capped_by_physical_beds() {
    let mut state = fixture();
    let destination = town(&mut state, "destination", 8, 32);
    house(&mut state, destination, 8);
    observe(&mut state, destination);
    state.test_acceleration.migration_rate = 3.0;
    state.tick_budget.reset(state.tick);
    let mut changes = ChangeSet::default();
    outside_arrivals(&mut state, &mut changes, destination, &mut Rng(123));
    assert_eq!(changes.entities_spawned.len(), 2);
    assert_eq!(capacity(&state, destination), 0);
    assert!(
        state
            .settlements
            .get(destination)
            .unwrap()
            .households
            .is_empty()
    );
    for &id in &changes.entities_spawned {
        let slot = state.entities.slot_of(id).unwrap();
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        assert_eq!(v.settlement, Some(destination));
        assert_eq!(v.household, None);
        assert_eq!(v.age, 0);
    }
    assert!(households::can_breed(
        &state,
        [changes.entities_spawned[0], changes.entities_spawned[1]]
    ));
    assert!(info(&state, destination).contains("outside world"));
    // A third physical bed is required for an actual ordinary birth, not just
    // identity compatibility. Let the immigrants approach and claim homes.
    let parents = [changes.entities_spawned[0], changes.entities_spawned[1]];
    house(&mut state, destination, 18);
    for _ in 0..5000 {
        step(&mut state);
        if parents.iter().all(|id| {
            state
                .entities
                .row(state.entities.slot_of(*id).unwrap())
                .villager
                .as_ref()
                .unwrap()
                .home
                .is_some()
        }) {
            break;
        }
    }
    for id in parents {
        let slot = state.entities.slot_of(id).unwrap();
        assert!(
            state
                .entities
                .row(slot)
                .villager
                .as_ref()
                .unwrap()
                .home
                .is_some(),
            "native edge arrival must claim before breeding: pos={:?} v={:?}",
            state.entities.pos(slot),
            state.entities.row(slot).villager
        );
    }
    let started = state.tick;
    for (index, id) in parents.into_iter().enumerate() {
        let slot = state.entities.slot_of(id).unwrap();
        let v = state.entities.row_mut(slot).villager.as_mut().unwrap();
        v.food_level = 12;
        v.partner = Some(parents[1 - index]);
        v.courtship_started_tick = Some(started);
        v.courtship_until = Some(started + 300);
    }
    for _ in 0..800 {
        step(&mut state);
        if !state
            .settlements
            .get(destination)
            .unwrap()
            .households
            .is_empty()
        {
            break;
        }
    }
    let family = state
        .settlements
        .get(destination)
        .unwrap()
        .households
        .values()
        .next()
        .unwrap_or_else(|| {
            panic!(
                "ordinary first birth adopts immigrant parents: {:?}",
                parents.map(|id| {
                    let s = state.entities.slot_of(id).unwrap();
                    (
                        state.entities.pos(s),
                        state.entities.row(s).villager.as_ref(),
                    )
                })
            )
        });
    let members = households::members(&state, destination, &family.id);
    assert_eq!(members.len(), 3);
    assert!(parents.iter().all(|id| members.contains(id)));
    let baby = members.iter().find(|id| !parents.contains(id)).unwrap();
    let slot = state.entities.slot_of(*baby).unwrap();
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    assert!(v.age < 0 && v.home.is_some() && v.birth_origin.is_some());
    assert_eq!(v.household.as_deref(), Some(family.id.as_str()));
}

#[test]
fn manual_bind_still_refuses_outside_presence_but_route_proven_arrival_admits() {
    let mut state = fixture();
    let destination = town(&mut state, "destination", 8, 32);
    let slot = allocator::tests::villager(&mut state, [120.5, 64.0, 8.5]);
    let id = state.entities.body(slot).id;
    assert_eq!(
        crate::settlements::bind(&mut state, destination, id),
        Err("villager is outside the settlement")
    );
    let goal = approach(&state, [8, 64, 8]).unwrap();
    assert!(
        route(
            &mut state,
            [120.5, 64.0, 8.5],
            goal,
            ADULT_HALF_WIDTH,
            ADULT_HEIGHT
        ) == Proof::Reachable
    );
    crate::settlements::admit_arrival(&mut state, destination, id).unwrap();
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
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .household,
        None
    );
}

#[test]
fn outside_outflow_rechecks_inventory_at_exit_and_cancels_without_loss() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 200);
    let a = resident(&mut state, source, 10.5);
    observe(&mut state, source);
    state.tick_budget.reset(state.tick);
    depart_decision(&mut state, source, &mut Rng(123));
    let exit = state.settlements.migration.walking[&source].exit;
    let slot = state.entities.slot_of(a).unwrap();
    let bread = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:bread").unwrap())
        .unwrap();
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(mc_data::ItemStack::new(bread, 1));
    state.entities.set_pos(slot, allocator::centre(exit));
    let mut changes = ChangeSet::default();
    advance_walks(&mut state, &mut changes);
    assert!(changes.entities_removed.is_empty());
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .settlement,
        Some(source)
    );
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .inventory[0]
            .as_ref()
            .unwrap()
            .count,
        1
    );
    assert!(state.settlements.migration.walking.is_empty());
    assert!(info(&state, source).contains("cancel"));
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = None;
    state.tick_budget.reset(state.tick);
    depart_decision(&mut state, source, &mut Rng(123));
    let exit = state.settlements.migration.walking[&source].exit;
    state.entities.set_pos(slot, allocator::centre(exit));
    advance_walks(&mut state, &mut changes);
    assert_eq!(changes.entities_removed, vec![a]);
    assert!(state.entities.slot_of(a).is_none());
    assert!(transit::summary(&state, source).is_empty());
}

#[test]
fn carried_items_disqualify_outside_not_a_less_attractive_eligible_town() {
    let mut state = fixture();
    let destination = town(&mut state, "destination", 8, 64);
    for x in [8, 18, 28] {
        house(&mut state, destination, x);
    }
    for x in [8.5, 9.5, 18.5, 28.5] {
        resident(&mut state, destination, x);
    }
    let source = town(&mut state, "source", 60, 32);
    let id = resident(&mut state, source, 60.5);
    let slot = state.entities.slot_of(id).unwrap();
    let diamond = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(mc_data::ItemStack::new(diamond, 1));
    observe(&mut state, source);
    observe(&mut state, destination);
    let score = state
        .settlements
        .migration
        .observation(destination)
        .unwrap()
        .attractiveness
        .total();
    assert!(score < OUTSIDE_ATTRACTIVENESS);
    assert!(gain(15.0, score, 52.0) > MIGRATION_THRESHOLD);
    state.tick_budget.reset(state.tick);
    depart_decision(&mut state, source, &mut Rng(123));
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
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .inventory[0]
            .as_ref()
            .unwrap()
            .count,
        1
    );
}

#[test]
fn simulated_capacity_rechecks_roofed_beds_instead_of_trusting_yesterdays_score() {
    let mut state = fixture();
    let destination = town(&mut state, "destination", 8, 32);
    house(&mut state, destination, 8);
    observe(&mut state, destination);
    assert_eq!(capacity(&state, destination), 2);
    for x in 5..=11 {
        for z in 5..=11 {
            state.world.set_block(x, 67, z, state.registry.states.air);
        }
    }
    state.world.fold_overlay();
    assert_eq!(
        state
            .settlements
            .migration
            .observation(destination)
            .unwrap()
            .facts
            .beds,
        2
    );
    assert_eq!(capacity(&state, destination), 0);
}

#[test]
fn edge_proof_uses_farthest_reachable_surface_not_an_isolated_outer_strip() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 200);
    let id = resident(&mut state, source, 8.5);
    state.tickets.clear();
    for x in 0..4 {
        state
            .tickets
            .insert(format!("strip-{x}"), (ChunkPos { x, z: 0 }, 0));
    }
    let tick = state.tick;
    state.refresh_active_chunks(tick);
    for x in 56..=60 {
        for z in 0..16 {
            for y in -64..80 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    state.world.fold_overlay();
    assert_eq!(transit::edge_point(&state, [8, 64, 8], 90.0).unwrap().x, 63);
    // Match the production registration: native allocator/brain passes first
    // announce the only simulated dimension and release unused guarantees.
    step(&mut state);
    step(&mut state);
    state.tick_budget.reset(state.tick);
    let exit = proven_edge(&mut state, [8, 64, 8], 90.0, EdgeTravel::Leaving(&[id]))
        .expect("reachable inner edge");
    assert_eq!(exit, BlockPos { x: 55, y: 64, z: 8 });
    state.tick += 1;
    state.tick_budget.reset(state.tick);
    let entry = proven_edge(
        &mut state,
        [8, 64, 8],
        90.0,
        EdgeTravel::Entering(BlockPos { x: 8, y: 64, z: 8 }),
    )
    .expect("entry route");
    assert_eq!(entry, exit);
}

#[test]
fn native_daily_evaluation_uses_world_age_and_skips_an_inactive_anchor() {
    let mut state = fixture();
    let id = town(&mut state, "town", 8, 32);
    state.tick = phase(id);
    state.day_time = 12345;
    tick(&mut state, &mut ChangeSet::default(), &[]);
    let first = state
        .settlements
        .migration
        .observation(id)
        .unwrap()
        .game_time;
    assert_eq!(first, state.tick_control.game_ticks(state.tick));
    state.day_time = 0;
    state.tick += DAY_TICKS - 1;
    tick(&mut state, &mut ChangeSet::default(), &[]);
    assert_eq!(
        state
            .settlements
            .migration
            .observation(id)
            .unwrap()
            .game_time,
        first
    );
    state.tick += 1;
    state.tickets.clear();
    let now = state.tick;
    state.refresh_active_chunks(now);
    tick(&mut state, &mut ChangeSet::default(), &[]);
    assert_eq!(
        state
            .settlements
            .migration
            .observation(id)
            .unwrap()
            .game_time,
        first
    );
}
