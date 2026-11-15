use super::*;

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "dead non-corpse row reached checkpoint capture")]
fn capture_rejects_dead_rows_outside_the_dying_clock() {
    let (mut state, _, position) = crate::hostile_tests::setup(0.0);
    let cow = state.spawn_mob(state.registry.cow_type_id, position, 20.0);
    let slot = state.entities.slot_of(cow).unwrap();
    state.entities.body_mut(slot).alive = false;
    assert!(!state.entities.is_dying(cow));
    let _ = entity_data_record(&state, slot);
}
