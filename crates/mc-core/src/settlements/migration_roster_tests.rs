//! Exercise the actual allocator → policy → brain boundary, not a second policy.
use super::return_tests::{fixture, house, resident, town};
use super::*;
use crate::villager::brain::roster_tests::{check_tick, dropped_item, live_ids, mob};

fn prime_destination(state: &mut TickState, destination: SettlementId) {
    // Drain the real bounded POI census before introducing any test brains.
    // The admission under test is still each resident's first brain pass.
    for _ in 0..10 {
        super::return_tests::step(state);
    }
    let now = state.tick_control.game_ticks(state.tick);
    let observed = facts(state, destination, now);
    assert_eq!(observed.beds, 2);
    state
        .settlements
        .migration
        .observe(destination, now, observed);
}

fn queue_walk(
    state: &mut TickState,
    source: SettlementId,
    members: &[EntityId],
    family: Option<&str>,
    destination: Option<SettlementId>,
) {
    let exit = BlockPos { x: 10, y: 64, z: 8 };
    let uuids = members
        .iter()
        .map(|&id| {
            let slot = state.entities.slot_of(id).unwrap();
            state.entities.set_pos(slot, allocator::centre(exit));
            state.entities.row(slot).uuid
        })
        .collect();
    state.settlements.migration.walking.insert(
        source,
        Walk {
            unit: family.map_or(Unit::Resident(members[0]), |key| {
                Unit::Household(key.into())
            }),
            members: uuids,
            destination,
            exit,
            bearing: 90.0,
            deadline: state.tick_control.game_ticks(state.tick) + WALK_TIMEOUT,
        },
    );
}

#[derive(Clone, Copy, Debug)]
enum Tail {
    Villager,
    Item,
    Cow,
    LastResident,
}

fn departure_case(transit: bool, size: usize, tail: Tail) -> Vec<EntityId> {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 32);
    let destination = town(&mut state, "destination", 208, 32);
    house(&mut state, destination, 208);
    prime_destination(&mut state, destination);
    let members: Vec<_> = (0..size)
        .map(|i| resident(&mut state, source, 10.5 + i as f64))
        .collect();
    let family = (size > 1).then(|| {
        households::form(
            &mut state,
            source,
            &members,
            households::HouseholdOrigin::Adoption,
        )
        .unwrap()
    });
    if !matches!(tail, Tail::LastResident) {
        resident(&mut state, source, 8.5);
    }
    match tail {
        Tail::Villager => {
            resident(&mut state, source, 9.5);
        }
        Tail::Item => {
            dropped_item(&mut state, [12.5, 64.0, 8.5]);
        }
        Tail::Cow => {
            mob(&mut state, "minecraft:cow", [12.5, 64.0, 8.5]);
        }
        Tail::LastResident => {}
    }
    queue_walk(
        &mut state,
        source,
        &members,
        family.as_deref(),
        transit.then_some(destination),
    );
    let mut changes = ChangeSet::default();
    allocator::tick(&mut state, &mut changes);
    let before = live_ids(&state);
    assert!(members.iter().all(|id| before.contains(id)));
    tick(&mut state, &mut changes, &[]);
    assert_eq!(
        changes.entities_removed.len(),
        size,
        "{tail:?}, transit={transit}"
    );
    for id in &members {
        assert!(state.entities.slot_of(*id).is_none());
    }
    assert_eq!(transit::summary(&state, source).len(), usize::from(transit));
    check_tick(&mut state, &mut changes)
}

#[test]
fn migration_roster_departures_cover_each_live_brain_once_for_every_swap_tail() {
    for transit in [false, true] {
        for size in [1, 2] {
            for tail in [Tail::Villager, Tail::Item, Tail::Cow, Tail::LastResident] {
                departure_case(transit, size, tail);
            }
        }
    }
}

#[test]
fn migration_roster_identical_policy_scenarios_keep_execution_order() {
    for tail in [Tail::Villager, Tail::Item, Tail::Cow] {
        assert_eq!(
            departure_case(false, 2, tail),
            departure_case(false, 2, tail)
        );
    }
}

#[test]
fn migration_roster_multiple_sources_compact_before_one_brain_pass() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 32);
    let other = town(&mut state, "other", 40, 32);
    let a = resident(&mut state, source, 10.5);
    let b = resident(&mut state, other, 10.5);
    resident(&mut state, source, 9.5);
    dropped_item(&mut state, [12.5, 64.0, 8.5]);
    queue_walk(&mut state, source, &[a], None, None);
    queue_walk(&mut state, other, &[b], None, None);
    let mut changes = ChangeSet::default();
    allocator::tick(&mut state, &mut changes);
    tick(&mut state, &mut changes, &[]);
    assert_eq!(changes.entities_removed, [a, b]);
    check_tick(&mut state, &mut changes);
}

#[test]
fn migration_roster_transit_arrival_and_departure_share_the_live_projection() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 32);
    let destination = town(&mut state, "destination", 208, 32);
    house(&mut state, destination, 208);
    let a = resident(&mut state, source, 10.5);
    let b = resident(&mut state, source, 11.5);
    let stay = resident(&mut state, source, 9.5);
    let family = households::form(
        &mut state,
        source,
        &[a, b],
        households::HouseholdOrigin::Adoption,
    )
    .unwrap();
    let record = transit::depart(
        &mut state,
        &mut ChangeSet::default(),
        source,
        households::Unit::Household(&family),
        destination,
        BlockPos { x: 10, y: 64, z: 8 },
        90.0,
    )
    .unwrap();
    state.tick = state.settlements.transit.get(record).unwrap().arrives;
    queue_walk(&mut state, source, &[stay], None, None);
    let item = dropped_item(&mut state, [1.5, 64.0, 8.5]);
    let mut changes = ChangeSet::default();
    allocator::tick(&mut state, &mut changes);
    let arrivals = transit::tick(&mut state, &mut changes);
    assert_eq!(arrivals.len(), 1);
    assert_eq!(changes.entities_spawned.len(), 2);
    assert!(changes.entities_spawned.iter().all(|id| {
        let slot = state.entities.slot_of(*id).unwrap();
        !state
            .villager_pois
            .active_chunks
            .contains(&state.entities.chunk_of(slot))
    }));
    tick(&mut state, &mut changes, &arrivals);
    assert_eq!(changes.entities_removed, [stay]);
    check_tick(&mut state, &mut changes);
    let slot = state.entities.slot_of(item).unwrap();
    assert!(
        state
            .villager_pois
            .items_near([1.5, 64.0, 8.5])
            .contains(&slot)
    );
}

#[test]
fn migration_roster_outside_arrival_adds_new_chunk_actors_in_the_same_frame() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 32);
    let destination = town(&mut state, "destination", 160, 32);
    house(&mut state, destination, 160);
    prime_destination(&mut state, destination);
    let leave = resident(&mut state, source, 10.5);
    queue_walk(&mut state, source, &[leave], None, None);
    let mut changes = ChangeSet::default();
    allocator::tick(&mut state, &mut changes);
    let now = state.tick_control.game_ticks(state.tick);
    let observed = facts(&state, destination, now);
    state
        .settlements
        .migration
        .observe(destination, now, observed);
    state.test_acceleration.migration_rate = 3.0;
    outside_arrivals(&mut state, &mut changes, destination, &mut Rng(123));
    assert_eq!(changes.entities_spawned.len(), 2);
    assert!(changes.entities_spawned.iter().all(|id| {
        !state.villager_pois.active_chunks.contains(
            &state
                .entities
                .chunk_of(state.entities.slot_of(*id).unwrap()),
        )
    }));
    tick(&mut state, &mut changes, &[]);
    assert_eq!(changes.entities_removed, [leave]);
    check_tick(&mut state, &mut changes);
}

#[test]
fn migration_roster_certified_role_holders_wait_for_release_before_selection_and_departure() {
    use crate::settlements::buildings::{self, BuildingKind};
    use crate::villager::VillagerProfession as Profession;

    for (kind, profession, station) in [
        (
            BuildingKind::Mine,
            Profession::Miner,
            "minecraft:crafting_table",
        ),
        (
            BuildingKind::LumberCamp,
            Profession::Lumberjack,
            "minecraft:crafting_table",
        ),
        (
            BuildingKind::Workshop(Profession::Librarian),
            Profession::Librarian,
            "minecraft:lectern",
        ),
    ] {
        for size in [1, 2] {
            let mut state = fixture();
            let source = town(&mut state, "source", 8, 32);
            let block = allocator::tests::block(&state, station, &[]);
            let job = BlockPos { x: 8, y: 64, z: 8 };
            state.world.set_block(job.x, job.y, job.z, block);
            state.world.fold_overlay();
            buildings::certify(
                &mut state,
                source,
                kind,
                mc_script::ScriptWorldBounds::new([7, 63, 7], [9, 66, 9]).unwrap(),
            )
            .unwrap();
            let members: Vec<_> = (0..size)
                .map(|i| resident(&mut state, source, 10.5 + i as f64))
                .collect();
            let family = (size > 1).then(|| {
                households::form(
                    &mut state,
                    source,
                    &members,
                    households::HouseholdOrigin::Adoption,
                )
                .unwrap()
            });
            let slot = state.entities.slot_of(members[0]).unwrap();
            let worker = state.entities.row_mut(slot).villager.as_mut().unwrap();
            worker.data.profession = profession;
            worker.job_site = Some(job);
            let mut changes = ChangeSet::default();
            allocator::tick(&mut state, &mut changes);
            assert_eq!(
                buildings::building_for_job(
                    &state,
                    state.entities.row(slot).villager.as_ref().unwrap(),
                )
                .map(|building| building.kind),
                Some(kind),
            );
            assert!(empty(&state, slot));
            let now = state.tick_control.game_ticks(state.tick);
            let observed = facts(&state, source, now);
            state.settlements.migration.observe(source, now, observed);
            depart_decision(&mut state, source, &mut Rng(123));
            assert!(!state.settlements.migration.walking.contains_key(&source));
            assert!(info(&state, source).contains("reservation"));

            // Revalidate a walk whose member acquired a role after selection.
            queue_walk(&mut state, source, &members, family.as_deref(), None);
            tick(&mut state, &mut changes, &[]);
            assert!(changes.entities_removed.is_empty());
            assert!(!state.settlements.migration.walking.contains_key(&source));
            assert!(info(&state, source).contains("reservation"));
            check_tick(&mut state, &mut changes);

            let worker = state.entities.row_mut(slot).villager.as_mut().unwrap();
            worker.job_site = None;
            worker.data.profession = Profession::None;
            let unit = family.map_or(Unit::Resident(members[0]), Unit::Household);
            let mut released = eligible(&state, source, &unit, false).unwrap();
            released.sort_unstable();
            assert_eq!(released, members);
        }
    }
}
