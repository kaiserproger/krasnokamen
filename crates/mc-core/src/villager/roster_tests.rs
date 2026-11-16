//! Same-frame entity projections: the oracle walks live dense rows, not the cache indexes.
use super::*;
use crate::settlements::migration::return_tests::{fixture, resident, town};

pub(crate) fn live_ids(state: &TickState) -> Vec<EntityId> {
    let mut ids = Vec::new();
    for slot in state.entities.slots() {
        let body = state.entities.body(slot);
        let pos = state.entities.pos(slot);
        if body.alive
            && state.entities.row(slot).villager.is_some()
            && state.world.is_active(
                (pos[0].floor() as i32).div_euclid(16),
                (pos[2].floor() as i32).div_euclid(16),
            )
        {
            ids.push(body.id);
        }
    }
    ids.sort_unstable();
    ids
}

pub(crate) fn mob(state: &mut TickState, name: &str, pos: [f64; 3]) -> EntityId {
    let kind = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap();
    state.spawn_mob(kind, pos, 20.0)
}

pub(crate) fn dropped_item(state: &mut TickState, pos: [f64; 3]) -> EntityId {
    let stick = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stick").unwrap())
        .unwrap();
    crate::tick::spawn_item_entity(state, pos, mc_data::ItemStack::new(stick, 1))
}

/// Call after the real allocator and policy. Capture independently before
/// admission, then compare the plan with every actual execution (including
/// order), not merely sets that could hide duplicate or misdirected brains.
pub(crate) fn check_tick(state: &mut TickState, changes: &mut ChangeSet) -> Vec<EntityId> {
    let expected = live_ids(state);
    state.villager_pois.execution_trace = Some(ExecutionTrace {
        expected: Vec::with_capacity(expected.len()),
        actual: Vec::with_capacity(expected.len()),
    });
    tick(state, changes);
    let cache = &state.villager_pois;
    let trace = cache.execution_trace.as_ref().unwrap();
    assert_eq!(
        trace.expected, trace.actual,
        "a cached identity ran another body"
    );
    let mut planned = trace.expected.clone();
    planned.sort_unstable();
    let mut executed = trace.actual.clone();
    executed.sort_unstable();
    assert_eq!(
        planned, expected,
        "admission omitted or duplicated a live brain"
    );
    assert_eq!(
        executed, expected,
        "execution omitted or duplicated a live brain"
    );
    assert_eq!(cache.residents.len(), expected.len());
    for (index, &slot) in cache.residents.iter().enumerate() {
        let id = state.entities.body(slot).id;
        assert_eq!(cache.resident_data[index].id, id);
        assert_eq!(cache.resident_index.get(&id), Some(&index));
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        assert_eq!(
            cache.resident_data[index].claims,
            [v.home, v.job_site, v.meeting_point]
        );
    }
    let mut actors: Vec<_> = cache
        .actors()
        .iter()
        .map(|&slot| state.entities.body(slot).id)
        .collect();
    actors.sort_unstable();
    assert_eq!(
        actors, expected,
        "social actors must use the same live admission"
    );
    trace.actual.clone()
}

fn reconcile(state: &mut TickState) {
    let mut cache = std::mem::take(&mut state.villager_pois);
    cache.reconcile_entities(state);
    state.villager_pois = cache;
}

#[test]
fn nonresident_item_swap_tail_reprojects_without_a_resident_revision() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 32);
    resident(&mut state, source, 8.5);
    let cow = mob(&mut state, "minecraft:cow", [10.5, 64.0, 8.5]);
    let item = dropped_item(&mut state, [11.5, 64.0, 8.5]);
    let mut changes = ChangeSet::default();
    crate::settlements::allocator::tick(&mut state, &mut changes);
    let revision = state.entities.brain_revision();
    let old_slot = state.entities.slot_of(item).unwrap();
    state.remove_entity(state.entities.slot_of(cow).unwrap());
    assert_eq!(state.entities.brain_revision(), revision);
    assert_ne!(state.entities.slot_of(item).unwrap(), old_slot);
    reconcile(&mut state);
    let slots = state.villager_pois.items_near([11.5, 64.0, 8.5]);
    assert_eq!(slots, &[state.entities.slot_of(item).unwrap()]);
    check_tick(&mut state, &mut changes);
}

#[test]
fn row_only_policy_changes_refresh_profiles_claims_and_preserve_cadence() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 32);
    let id = resident(&mut state, source, 8.5);
    let mut changes = ChangeSet::default();
    crate::settlements::allocator::tick(&mut state, &mut changes);
    let before = state.villager_pois.resident_data[0];
    let revision = state.entities.brain_revision();
    let slot = state.entities.slot_of(id).unwrap();
    let home = BlockPos { x: 12, y: 64, z: 8 };
    let v = state.entities.row_mut(slot).villager.as_mut().unwrap();
    v.data.profession = Profession::Librarian;
    v.activity = Activity::Panic;
    v.panic_until = 100;
    v.home = Some(home);
    assert_eq!(state.entities.brain_revision(), revision);
    reconcile(&mut state);
    let after = state.villager_pois.resident_data[0];
    assert_eq!(after.profession, Profession::Librarian);
    assert_eq!(after.activity, Activity::Panic);
    assert_eq!(after.claims[0], Some(home));
    assert!(after.cadence == before.cadence);
    assert_eq!(after.last_full, before.last_full);
    assert!(state.villager_pois.homes[&home].contains(&id));
    check_tick(&mut state, &mut changes);
}

#[test]
fn new_resident_chunk_rebuilds_item_and_threat_sensors_in_the_same_frame() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 32);
    let first = resident(&mut state, source, 8.5);
    let remote = town(&mut state, "remote", 160, 32);
    let item = dropped_item(&mut state, [162.5, 64.0, 8.5]);
    mob(&mut state, "minecraft:zombie", [164.5, 64.0, 8.5]);
    let mut changes = ChangeSet::default();
    crate::settlements::allocator::tick(&mut state, &mut changes);
    assert!(
        state
            .villager_pois
            .items_near([162.5, 64.0, 8.5])
            .is_empty()
    );
    assert!(state.villager_pois.threats.is_empty());
    let before = state.villager_pois.resident_data[state.villager_pois.resident_index[&first]];
    let arrival = resident(&mut state, remote, 160.5);
    reconcile(&mut state);
    let cache = &mut state.villager_pois;
    assert_eq!(
        cache.items_near([162.5, 64.0, 8.5]),
        &[state.entities.slot_of(item).unwrap()]
    );
    assert!(
        cache
            .threats
            .iter()
            .any(|(pos, _)| *pos == [164.5, 64.0, 8.5])
    );
    assert!(cache.resident_index.contains_key(&arrival));
    let after = cache.resident_data[cache.resident_index[&first]];
    assert!(after.cadence == before.cadence);
    assert_eq!(after.last_full, before.last_full);
    check_tick(&mut state, &mut changes);
}

#[test]
fn projection_idle_and_edit_heavy_costs() {
    let mut state = fixture();
    let source = town(&mut state, "source", 8, 32);
    for i in 0..128 {
        resident(&mut state, source, 8.0 + f64::from(i % 16) / 8.0);
    }
    crate::settlements::allocator::tick(&mut state, &mut ChangeSet::default());
    let mut cache = std::mem::take(&mut state.villager_pois);
    let capacity = cache.resident_data.capacity();
    let start = std::time::Instant::now();
    for _ in 0..100_000 {
        cache.reconcile_entities(std::hint::black_box(&state));
    }
    let idle = start.elapsed();
    assert_eq!(cache.resident_data.capacity(), capacity);
    let slot = cache.residents[0];
    let mut edits = std::time::Duration::ZERO;
    for i in 0..128 {
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .panic_until = i;
        let start = std::time::Instant::now();
        cache.reconcile_entities(&state);
        edits += start.elapsed();
    }
    let mut membership = std::time::Duration::ZERO;
    for _ in 0..32 {
        let arrival = resident(&mut state, source, 12.5);
        let start = std::time::Instant::now();
        cache.reconcile_entities(&state);
        membership += start.elapsed();
        state.remove_entity(state.entities.slot_of(arrival).unwrap());
        let start = std::time::Instant::now();
        cache.reconcile_entities(&state);
        membership += start.elapsed();
    }
    eprintln!(
        "projection_cost residents=128 idle_calls=100000 idle_ns={} row_edit_calls=128 row_edit_ns={} membership_calls=64 membership_ns={}",
        idle.as_nanos(),
        edits.as_nanos(),
        membership.as_nanos()
    );
    assert_eq!(
        cache.entity_revision,
        Some(state.entities.projection_revision())
    );
}
