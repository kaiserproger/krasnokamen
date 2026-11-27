use super::return_tests::{fixture, house, resident, shelter, step, town};
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

/// A generated piece may certify a House and its own workshop on one footprint
/// (review wave2-10): development counts distinct footprints, not records.
#[test]
fn a_house_and_its_workshop_on_one_footprint_count_as_one_building() {
    let mut state = fixture();
    let destination = town(&mut state, "destination", 8, 32);
    house(&mut state, destination, 8);
    let bounds = state
        .settlements
        .get(destination)
        .unwrap()
        .buildings
        .values()
        .next()
        .expect("the town's house")
        .bounds;
    let record = state.settlements.get_mut(destination).unwrap();
    let next = record.building_sequence + 1;
    record.building_sequence = next;
    let id = crate::settlements::buildings::BuildingId((destination.0 << 32) | next);
    record.buildings.insert(
        id,
        crate::settlements::buildings::Building {
            id,
            settlement: destination,
            kind: crate::settlements::buildings::BuildingKind::Workshop(
                crate::villager::VillagerProfession::Mason,
            ),
            level: 1,
            bounds,
            work_points: vec![],
            revision: 1,
            natural_zone: false,
        },
    );
    assert_eq!(
        state.settlements.get(destination).unwrap().buildings.len(),
        2,
        "fixture: two records share one footprint"
    );
    assert_eq!(
        facts(&state, destination, state.tick).completed_buildings,
        1,
        "one footprint is one completed building"
    );
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

/// Spain seed 100 (trace pin fd53e9146612-586c7780): the adopted savanna
/// village's members sleep in generated houses that fail House certification,
/// so `certified beds - members` stayed 0 while two built house beds stood
/// empty for 60k ticks and no migrant came.
#[test]
fn members_in_uncertified_houses_leave_certified_beds_to_migrants() {
    let mut state = fixture();
    let destination = town(&mut state, "destination", 8, 32);
    house(&mut state, destination, 8);
    shelter(&mut state, 20);
    let members = [
        resident(&mut state, destination, 20.5),
        resident(&mut state, destination, 19.5),
    ];
    let home = |state: &TickState, id: EntityId| {
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.row(slot).villager.as_ref().unwrap().home
    };
    for _ in 0..200 {
        if members.iter().all(|&id| home(&state, id).is_some()) {
            break;
        }
        step(&mut state);
    }
    for id in members {
        assert_eq!(
            home(&state, id).map(|bed| bed.x),
            Some(20),
            "fixture: members sleep in the uncertified shelter"
        );
    }
    observe(&mut state, destination);
    assert_eq!(capacity(&state, destination), 2);
}

/// Review P2 (wt/SpainLand): an unbound resident sleeping in a certified
/// house holds that bed in the real claim index; counting only members
/// offered it to a newcomer as well.
#[test]
fn a_certified_bed_an_unbound_resident_claims_is_not_offered() {
    let mut state = fixture();
    let destination = town(&mut state, "destination", 8, 32);
    house(&mut state, destination, 8);
    shelter(&mut state, 20);
    let member = resident(&mut state, destination, 20.5);
    let lodger = allocator::tests::villager(&mut state, [8.5, 64.0, 8.5]);
    let lodger = state.entities.body(lodger).id;
    let home = |state: &TickState, id: EntityId| {
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.row(slot).villager.as_ref().unwrap().home
    };
    for _ in 0..200 {
        if [member, lodger]
            .iter()
            .all(|&id| home(&state, id).is_some())
        {
            break;
        }
        step(&mut state);
    }
    assert_eq!(
        [member, lodger].map(|id| home(&state, id).map(|bed| bed.x)),
        [Some(20), Some(8)],
        "fixture: the member sleeps in the shelter, the lodger in the house"
    );
    observe(&mut state, destination);
    assert_eq!(capacity(&state, destination), 1);
}

/// Review P2 (wt/SpainLand): members who arrive after the observation of a
/// destination that is no longer simulated took its free beds; they are not
/// inbound any more, so the cached free beds alone offered those beds twice.
#[test]
fn an_unsimulated_destination_counts_members_who_arrived_since_its_observation() {
    let mut state = fixture();
    let destination = town(&mut state, "destination", 8, 32);
    house(&mut state, destination, 8);
    observe(&mut state, destination);
    assert_eq!(capacity(&state, destination), 2, "fixture: two free beds");
    resident(&mut state, destination, 7.5);
    resident(&mut state, destination, 8.5);
    state.tickets.clear();
    let tick = state.tick;
    state.refresh_active_chunks(tick);
    assert!(
        !simulated(&state, [8, 64, 8]),
        "fixture: the centre is unsimulated"
    );
    assert_eq!(capacity(&state, destination), 0);
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
    // Home claims can precede physical arrival by 48 blocks. Ordinary Rest
    // brings both immigrant parents to their neighbouring beds before the
    // ordinary first-birth courtship is started.
    for _ in 0..4000 {
        state.day_time = 18_000;
        step(&mut state);
        if parents.iter().all(|id| {
            state
                .entities
                .row(state.entities.slot_of(*id).unwrap())
                .villager
                .as_ref()
                .unwrap()
                .sleeping
        }) {
            break;
        }
    }
    for id in parents {
        assert!(
            state
                .entities
                .row(state.entities.slot_of(id).unwrap())
                .villager
                .as_ref()
                .unwrap()
                .sleeping,
            "the immigrant physically sleeps at its claimed home before courtship"
        );
    }
    state.day_time = 2000;
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

/// Owner rule: the outside world scores `OUTSIDE_ATTRACTIVENESS` and migration
/// needs a `MIGRATION_THRESHOLD` advantage, so outside arrivals admit only
/// settlements strictly above that production floor.
#[test]
fn outside_arrival_flips_at_the_production_attractiveness_floor() {
    let mut state = fixture();
    let destination = town(&mut state, "destination", 8, 32);
    house(&mut state, destination, 8);
    observe(&mut state, destination);
    assert!(
        capacity(&state, destination) > 0,
        "the destination must have live bed capacity before testing the floor"
    );
    // members 10, beds 15 (housing 0.5), no jobs, no deaths (safety 1): food
    // moves the score either side of the floor without naming 35.0.
    let facts = |food_points| Facts {
        members: 10,
        beds: 15,
        free_beds: 5,
        inbound: 0,
        food_points,
        open_jobs: 0,
        recent_deaths: 0,
        completed_buildings: 0,
    };
    let floor = OUTSIDE_ATTRACTIVENESS + MIGRATION_THRESHOLD;
    assert!(Attractiveness::from_facts(&facts(58)).total() <= floor);
    assert!(Attractiveness::from_facts(&facts(62)).total() > floor);
    state.test_acceleration.migration_rate = 3.0;
    state.tick_budget.reset(state.tick);
    let mut changes = ChangeSet::default();
    state
        .settlements
        .migration
        .observe(destination, state.tick, facts(58));
    outside_arrivals(&mut state, &mut changes, destination, &mut Rng(123));
    assert!(
        changes.entities_spawned.is_empty(),
        "no advantage at the outside floor: {:?}",
        changes.entities_spawned
    );
    state
        .settlements
        .migration
        .observe(destination, state.tick, facts(62));
    outside_arrivals(&mut state, &mut changes, destination, &mut Rng(123));
    assert!(
        !changes.entities_spawned.is_empty(),
        "above the floor the outside world supplies arrivals"
    );
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

#[test]
fn remove_command_clears_source_destination_and_return_runtime_references() {
    let mut state = fixture();
    let removed = town(&mut state, "removed", 8, 32);
    let surviving = town(&mut state, "surviving", 208, 32);
    let a = resident(&mut state, removed, 9.5);
    let b = resident(&mut state, surviving, 209.5);
    observe(&mut state, removed);
    observe(&mut state, surviving);
    let a_slot = state.entities.slot_of(a).unwrap();
    state.entities.set_pos(a_slot, [100.5, 64.0, 8.5]);
    // Populate the runtime memory through the real rule and allocator census;
    // native work priorities are exercised separately by the return-rule tests.
    let mut v = state.entities.row_mut(a_slot).villager.take().unwrap();
    let mut cache = std::mem::take(&mut state.villager_pois);
    assert!(cache.home_search[&a].no_candidates);
    crate::villager::return_home::step(&mut state, &mut cache, a_slot, &mut v, false);
    state.entities.row_mut(a_slot).villager = Some(v);
    state.villager_pois = cache;
    assert!(
        state
            .settlements
            .migration
            .returning
            .values()
            .any(|memory| memory.settlement() == removed)
    );
    // Seed both directions of the runtime dependency graph. The command must
    // cancel the surviving source's walk too, not just the removed town's map key.
    for (source, destination, entity, x) in
        [(removed, surviving, a, 32), (surviving, removed, b, 224)]
    {
        let uuid = state
            .entities
            .row(state.entities.slot_of(entity).unwrap())
            .uuid;
        let exit = BlockPos { x, y: 64, z: 8 };
        let migration = &mut state.settlements.migration;
        migration.targets.insert(uuid, exit);
        migration.walking.insert(
            source,
            Walk {
                unit: Unit::Resident(entity),
                members: vec![uuid],
                destination: Some(destination),
                exit,
                bearing: 90.0,
                deadline: state.tick + WALK_TIMEOUT,
            },
        );
        migration.walking_sources.push(source);
        migration.decisions.insert(source, "outbound walk".into());
        migration
            .deaths
            .insert(source, VecDeque::from([state.tick]));
    }
    let command = crate::settlements::commands::parse(&["remove", "removed"]).unwrap();
    crate::command_execution::execute(
        &mut state,
        crate::command_console::CONNECTION,
        a,
        &crate::commands::Command::Settlement(command),
        &mut ChangeSet::default(),
    );
    assert!(state.settlements.get(removed).is_none());
    let migration = &state.settlements.migration;
    assert!(migration.observation(removed).is_none());
    assert!(migration.observation(surviving).is_some());
    assert!(!migration.deaths.contains_key(&removed));
    assert!(!migration.decisions.contains_key(&removed));
    assert!(!migration.decisions.contains_key(&surviving));
    assert!(migration.walking.is_empty());
    assert!(migration.walking_sources.is_empty());
    assert!(migration.targets.is_empty());
    assert!(
        migration
            .returning
            .values()
            .all(|memory| memory.settlement() != removed)
    );
    assert_eq!(
        state
            .entities
            .row(a_slot)
            .villager
            .as_ref()
            .unwrap()
            .settlement,
        None
    );
    assert_eq!(
        state
            .entities
            .row(state.entities.slot_of(b).unwrap())
            .villager
            .as_ref()
            .unwrap()
            .settlement,
        Some(surviving)
    );
    // Migration State/Memory have no persistence image: reopening derives them
    // from the surviving native memberships, not from these cancelled intents.
}
#[test]
fn outbound_walk_uses_game_time_before_and_after_a_day_long_freeze() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 200);
    let resident = resident(&mut state, source, 10.5);
    observe(&mut state, source);
    super::return_tests::freeze_ticks(&mut state, 3);
    state.tick_budget.reset(state.tick);
    depart_decision(&mut state, source, &mut Rng(123));
    let deadline = state.settlements.migration.walking[&source].deadline;
    assert_eq!(deadline, state.tick_control.game_ticks(state.tick) + 24_000);
    let slot = state.entities.slot_of(resident).unwrap();
    let uuid = state.entities.row(slot).uuid;
    let target = target(&state, uuid).unwrap();
    super::return_tests::freeze_ticks(&mut state, 24_001);
    let mut changes = ChangeSet::default();
    advance_walks(&mut state, &mut changes);
    assert_eq!(
        state.settlements.migration.walking[&source].deadline,
        deadline
    );
    assert_eq!(self::target(&state, uuid), Some(target));
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
    state.tick += deadline - state.tick_control.game_ticks(state.tick);
    advance_walks(&mut state, &mut changes);
    assert!(!state.settlements.migration.walking.contains_key(&source));
    assert!(self::target(&state, uuid).is_none());
    assert!(info(&state, source).contains("timed out"));
}

fn review_resident_hit(
    state: &mut TickState,
    victim: EntityId,
    amount: f32,
    source: Option<crate::intent::DamageSource>,
) -> Vec<crate::boundary::EntityEvent> {
    let slot = state.entities.slot_of(victim).unwrap();
    state.merge_damage.push((
        slot,
        crate::tick::DamagePiece {
            physical: None,
            custom_armor: false,
            amount,
            kind: if source.is_some() {
                crate::survival::DamageKind::Melee
            } else {
                crate::survival::DamageKind::GenericKill
            },
            source,
        },
    ));
    state.commit_merge_effects().1
}

#[test]
fn review_resident_death_reaches_daily_safety_once_and_expires() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 32);
    let victim = resident(&mut state, source, 10.5);
    resident(&mut state, source, 11.5);
    state.tick = phase(source);
    let events = review_resident_hit(&mut state, victim, 100.0, None);
    assert!(events.iter().any(|event| matches!(event,
        crate::boundary::EntityEvent::Death { entity } if *entity == victim)));
    assert!(state.entities.slot_of(victim).is_none());
    state.test_acceleration.migration_rate = 0.0;
    let mut changes = ChangeSet::default();
    tick(&mut state, &mut changes, &[]);
    let observed = state.settlements.migration.observation(source).unwrap();
    assert_eq!(observed.facts.recent_deaths, 1);
    // Native death animation retains the body until its later removal.
    assert_eq!(observed.attractiveness.safety, 0.5);
    changes.entity_events.extend(events);
    state.commit_merge_effects();
    state.tick += DAY_TICKS;
    tick(&mut state, &mut changes, &[]);
    assert_eq!(
        state
            .settlements
            .migration
            .observation(source)
            .unwrap()
            .facts
            .recent_deaths,
        1
    );
    state.tick += (SAFETY_DAYS - 1) * DAY_TICKS;
    tick(&mut state, &mut changes, &[]);
    let observed = state.settlements.migration.observation(source).unwrap();
    assert_eq!(observed.facts.recent_deaths, 0);
    assert_eq!(observed.attractiveness.safety, 1.0);
}

#[test]
fn review_hard_infection_is_not_a_resident_death() {
    let mut state = fixture();
    state.difficulty = crate::survival::Difficulty::Hard;
    let source = town(&mut state, "source", 8, 32);
    let victim = resident(&mut state, source, 10.5);
    let zombie_type = state.registry.entity_type_id("minecraft:zombie").unwrap();
    let zombie = state.spawn_mob(zombie_type, [11.5, 64.0, 8.5], 20.0);
    state.tick = phase(source);
    let events = review_resident_hit(
        &mut state,
        victim,
        100.0,
        Some(crate::intent::DamageSource::Melee {
            attacker: zombie,
            position: [11.5, 64.0, 8.5],
            yaw: 0.0,
            bonus_knockback: 0.0,
            sweep: false,
            fire_ticks: 0,
            bane_ticks: 0,
        }),
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, crate::boundary::EntityEvent::Death { .. }))
    );
    let slot = state.entities.slot_of(victim).unwrap();
    assert!(state.entities.body(slot).alive);
    assert!(matches!(state.entities.body(slot).kind,
        crate::entities::EntityKind::Mob { type_id }
        if type_id == state.registry.entity_type_id("minecraft:zombie_villager").unwrap()));
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .conversion_time,
        Some(-1)
    );
    state.test_acceleration.migration_rate = 0.0;
    tick(&mut state, &mut ChangeSet::default(), &[]);
    let observed = state.settlements.migration.observation(source).unwrap();
    assert_eq!(observed.facts.recent_deaths, 0);
    assert_eq!(observed.attractiveness.safety, 1.0);
}
