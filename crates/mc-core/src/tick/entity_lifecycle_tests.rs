//! The spawn clearance diagnostic: a body placed inside a block is created (the
//! spawn is never refused) and the writer is named in one warn line.

use std::sync::{Arc, Mutex};

const TARGET: &str = "tick::entity_lifecycle";

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl tracing::Subscriber for Warnings {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::WARN
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if !event.metadata().target().ends_with(TARGET) {
            return;
        }
        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write as _;
                write!(&mut self.0, "{}={value:?} ", field.name()).unwrap();
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                use std::fmt::Write as _;
                write!(&mut self.0, "{}={value} ", field.name()).unwrap();
            }
        }
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// The platform top is `centre[1]`; its stone occupies `centre[1] - 1`.
#[test]
fn a_body_spawned_inside_a_block_is_created_and_named() {
    let (mut state, _player, centre) = crate::hostile_tests::setup(0.0);
    let cow = state.registry.cow_type_id;
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        let in_air = state.spawn_mob(cow, [centre[0], centre[1] + 5.0, centre[2]], 10.0);
        let inside = state.spawn_mob(cow, [centre[0], centre[1] - 0.5, centre[2]], 10.0);
        assert!(state.entities.slot_of(in_air).is_some());
        assert!(
            state.entities.slot_of(inside).is_some(),
            "the clearance diagnostic must never refuse a spawn"
        );
    });
    let lines = warnings.0.lock().unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    for field in [
        "tick=",
        "slot=",
        "type_id=",
        "entity_type=minecraft:cow",
        "position=",
        "half_width=",
        "height=",
        "writer=TickState::spawn_mob",
    ] {
        assert!(line.contains(field), "{field} missing from {line}");
    }
}

#[test]
fn a_body_standing_on_the_collision_box_is_not_reported() {
    let (mut state, _player, centre) = crate::hostile_tests::setup(0.0);
    let cow = state.registry.cow_type_id;
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        // Feet exactly on the platform's top face: touching, not overlapping.
        let standing = state.spawn_mob(cow, [centre[0], centre[1], centre[2]], 10.0);
        assert!(state.entities.slot_of(standing).is_some());
    });
    assert!(
        warnings.0.lock().unwrap().is_empty(),
        "{:?}",
        warnings.0.lock().unwrap()
    );
}

#[test]
fn a_body_spawned_outside_the_resident_world_is_not_reported() {
    let (mut state, _player, _centre) = crate::hostile_tests::setup(0.0);
    let cow = state.registry.cow_type_id;
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        // An unloaded cell is not evidence: the motion path treats it as solid,
        // but the spawn diagnostic must not invent an overlap there.
        let unloaded = state.spawn_mob(cow, [12_000.0, 70.0, 12_000.0], 10.0);
        assert!(state.entities.slot_of(unloaded).is_some());
    });
    assert!(
        warnings.0.lock().unwrap().is_empty(),
        "{:?}",
        warnings.0.lock().unwrap()
    );
}
