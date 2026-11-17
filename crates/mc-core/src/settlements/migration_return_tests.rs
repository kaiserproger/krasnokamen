//! Daytime controls deliberately offer no certified job: neither existing
//! remote-job routing nor night shelter may accidentally prove the return rule.
use super::*;
use crate::boundary::{ChangeSet, ChunkPos, EntityId};
use crate::settlements::{allocator, buildings, households};
use crate::tick::TickState;
use std::sync::Arc;

pub(crate) fn fixture() -> TickState {
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let world = crate::horde_bench::flat_world(&registry);
    let mut state = TickState::new(registry, world, 16, 512);
    let wanted = rustc_hash::FxHashMap::from_iter(
        (-1..=14).flat_map(|x| (-1..=1).map(move |z| (ChunkPos { x, z }, 0))),
    );
    state.world.ensure_blocking(&wanted);
    state
        .tickets
        .insert("migration-fixture".into(), (ChunkPos { x: 7, z: 0 }, 8));
    state.refresh_active_chunks(0);
    state.day_time = 2000;
    state
}

pub(crate) fn town(state: &mut TickState, name: &str, x: i32, radius: u32) -> SettlementId {
    state
        .settlements
        .create(
            &mut state.settlement_sequence,
            name.into(),
            "owner".into(),
            [x, 64, 8],
            radius,
        )
        .unwrap()
}

pub(crate) fn resident(state: &mut TickState, town: SettlementId, x: f64) -> EntityId {
    let slot = allocator::tests::villager(state, [x, 64.0, 8.5]);
    let id = state.entities.body(slot).id;
    crate::settlements::bind(state, town, id).unwrap();
    id
}

pub(crate) fn house(state: &mut TickState, town: SettlementId, x: i32) {
    let stone = state.registry.states.stone;
    for a in x - 3..=x + 3 {
        for z in 5..=11 {
            state.world.set_block(a, 67, z, stone);
            if a == x - 3 || a == x + 3 || z == 5 || z == 11 {
                for y in 64..67 {
                    state.world.set_block(a, y, z, stone);
                }
            }
        }
    }
    let lower = allocator::tests::block(
        state,
        "minecraft:oak_door",
        &[("half", "lower"), ("facing", "east")],
    );
    let upper = allocator::tests::block(
        state,
        "minecraft:oak_door",
        &[("half", "upper"), ("facing", "east")],
    );
    state.world.set_block(x + 3, 64, 8, lower);
    state.world.set_block(x + 3, 65, 8, upper);
    for z in [7, 9] {
        let foot = allocator::tests::block(
            state,
            "minecraft:red_bed",
            &[("part", "foot"), ("facing", "east")],
        );
        let head = allocator::tests::block(
            state,
            "minecraft:red_bed",
            &[("part", "head"), ("facing", "east")],
        );
        state.world.set_block(x - 1, 64, z, foot);
        state.world.set_block(x, 64, z, head);
    }
    state.world.fold_overlay();
    buildings::certify(
        state,
        town,
        buildings::BuildingKind::House,
        mc_script::ScriptWorldBounds::new([x - 3, 63, 5], [x + 3, 67, 11]).unwrap(),
    )
    .unwrap();
}

pub(crate) fn step(state: &mut TickState) {
    state.tick_budget.reset(state.tick);
    let mut changes = ChangeSet::default();
    allocator::tick(state, &mut changes);
    crate::villager::brain::tick(state, &mut changes);
    // Match the real tick's commit barrier: POIs observe committed revisions.
    state.world.fold_overlay();
    state.tick += 1;
    state.day_time = 2000;
}

pub(crate) fn freeze_ticks(state: &mut TickState, ticks: u64) {
    let game_time = state.tick_control.game_ticks(state.tick);
    crate::command_console::execute(
        state,
        crate::command_console::CONNECTION,
        "tick freeze",
        4,
        &mut ChangeSet::default(),
    );
    assert!(state.tick_control.frozen());
    for _ in 0..ticks {
        state.tick += 1;
        crate::tick_control::begin_tick(state);
        assert!(!state.tick_control.runs_normally());
    }
    assert_eq!(state.tick_control.game_ticks(state.tick), game_time);
    crate::command_console::execute(
        state,
        crate::command_console::CONNECTION,
        "tick unfreeze",
        4,
        &mut ChangeSet::default(),
    );
    state.tick += 1;
    crate::tick_control::begin_tick(state);
    assert!(state.tick_control.runs_normally());
}

#[test]
fn relocated_family_walks_two_hundred_blocks_to_native_roofed_beds_in_daylight() {
    let mut state = fixture();
    let source = town(&mut state, "source", 208, 32);
    let destination = town(&mut state, "destination", 8, 32);
    house(&mut state, destination, 8);
    let a = resident(&mut state, source, 207.5);
    let b = resident(&mut state, source, 208.5);
    let key = households::form(
        &mut state,
        source,
        &[a, b],
        households::HouseholdOrigin::Adoption,
    )
    .unwrap();
    let uuids = [a, b].map(|id| state.entities.row(state.entities.slot_of(id).unwrap()).uuid);
    households::relocate(&mut state, source, &key, destination).unwrap();
    for _ in 0..7000 {
        step(&mut state);
        if [a, b].iter().all(|id| {
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
    for (id, uuid) in [a, b].into_iter().zip(uuids) {
        let slot = state.entities.slot_of(id).unwrap();
        assert_eq!(state.entities.row(slot).uuid, uuid);
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        assert!(
            v.home.is_some(),
            "relocated member {id:?} remains unhoused at {:?}",
            state.entities.pos(slot)
        );
        assert_eq!(v.settlement, Some(destination));
        let cell = allocator::at(state.entities.pos(slot));
        assert!(
            state
                .settlements
                .get(destination)
                .unwrap()
                .contains([cell.x, cell.y, cell.z])
        );
    }
}

#[test]
fn homeless_edge_entry_inside_large_sphere_still_approaches_remote_bed() {
    let mut state = fixture();
    let destination = town(&mut state, "wide", 8, 200);
    house(&mut state, destination, 8);
    let id = resident(&mut state, destination, 120.5);
    for _ in 0..5000 {
        step(&mut state);
        if state
            .entities
            .row(state.entities.slot_of(id).unwrap())
            .villager
            .as_ref()
            .unwrap()
            .home
            .is_some()
        {
            break;
        }
    }
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
        "edge member remains unhoused at {:?}",
        state.entities.pos(slot)
    );
    assert!(allocator::distance(state.entities.pos(slot), [8.5, 64.0, 8.5]) < 48.0 * 48.0);
}

#[test]
fn real_daughter_handover_and_outside_builder_preserve_native_work_and_people() {
    use crate::settlements::construction::tests::{complete, fixture as builders, fund, start};
    use crate::settlements::construction::{self, FoundingExpedition};
    use crate::villager::{Activity, VillagerProfession};
    let (mut state, parent, builder) = builders();
    let wanted = (-1..=14)
        .flat_map(|x| (-1..=3).map(move |z| (ChunkPos { x, z }, 0)))
        .collect();
    state.world.ensure_blocking(&wanted);
    for x in -16..240 {
        for z in -16..64 {
            state
                .world
                .set_block(x, 200, z, state.registry.states.stone);
        }
    }
    state.world.fold_overlay();
    state
        .tickets
        .insert("daughter-path".into(), (ChunkPos { x: 7, z: 1 }, 8));
    let tick = state.tick;
    state.refresh_active_chunks(tick);
    start(&mut state, parent, "spanish_plaza", 1, None);
    fund(&mut state, "spanish_plaza", 1);
    complete(&mut state, parent, builder);
    let mut migrants = Vec::new();
    for i in 0..23 {
        let slot = allocator::tests::villager(&mut state, [2.5, 201.0, 2.5]);
        let id = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, parent, id).unwrap();
        state.entities.row_mut(slot).controls.no_ai = true;
        if i < 3 {
            migrants.push(id);
        }
    }
    let key = households::form(
        &mut state,
        parent,
        &migrants,
        households::HouseholdOrigin::Migration,
    )
    .unwrap();
    let expedition = FoundingExpedition {
        destination: [200, 200, 0],
        household: key,
    };
    state.settlements.get_mut(parent).unwrap().founding = Some(expedition.clone());
    state.test_acceleration.construction_speed = 100.0;
    let mut outside_builder = false;
    for (index, (name, _)) in expedition.sites().into_iter().enumerate() {
        fund(&mut state, name, 1);
        state.tick = state.tick.div_ceil(20) * 20;
        construction::tick(&mut state);
        assert!(
            state
                .settlements
                .get(parent)
                .unwrap()
                .projects
                .first()
                .is_some()
        );
        let v = state.entities.row_mut(builder).villager.as_mut().unwrap();
        v.data.profession = VillagerProfession::Builder;
        v.activity = Activity::Work;
        if index == 0 {
            for _ in 0..20_000 {
                state.day_time = 4000;
                state.tick_budget.reset(state.tick);
                let mut changes = ChangeSet::default();
                allocator::tick(&mut state, &mut changes);
                construction::tick(&mut state);
                crate::villager::brain::tick(&mut state, &mut changes);
                state.world.fold_overlay();
                let pos = allocator::at(state.entities.pos(builder));
                let id = state.entities.body(builder).id;
                outside_builder |= !state
                    .settlements
                    .get(parent)
                    .unwrap()
                    .contains([pos.x, pos.y, pos.z])
                    && state
                        .villager_pois
                        .home_search
                        .get(&id)
                        .is_some_and(|r| r.no_candidates);
                state.tick += 1;
                if state.settlements.get(parent).unwrap().projects.is_empty() {
                    break;
                }
            }
            assert!(
                state.settlements.get(parent).unwrap().projects.is_empty(),
                "native outside builder must finish the outpost house: status={:?} pos={:?} villager={:?}",
                construction::project_status(&state, parent),
                state.entities.pos(builder),
                state.entities.row(builder).villager,
            );
        } else {
            complete(&mut state, parent, builder);
        }
    }
    assert!(
        outside_builder,
        "builder actually worked outside the civil sphere with the fallback trigger present"
    );
    state.entities.row_mut(builder).controls.no_ai = true;
    state.tick = state.tick.div_ceil(20) * 20;
    construction::tick(&mut state);
    let child = state
        .settlements
        .find("colony_2")
        .expect("real paid handover")
        .id;
    let identity: Vec<_> = migrants
        .iter()
        .map(|id| {
            let slot = state.entities.slot_of(*id).unwrap();
            assert_eq!(
                state
                    .entities
                    .row(slot)
                    .villager
                    .as_ref()
                    .unwrap()
                    .settlement,
                Some(child)
            );
            state.entities.row_mut(slot).controls.no_ai = false;
            (
                state.entities.row(slot).uuid,
                state
                    .entities
                    .row(slot)
                    .villager
                    .as_ref()
                    .unwrap()
                    .inventory
                    .clone(),
            )
        })
        .collect();
    for _ in 0..9000 {
        step(&mut state);
        if migrants.iter().all(|id| {
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
    for (id, (uuid, inventory)) in migrants.iter().zip(identity) {
        let slot = state.entities.slot_of(*id).unwrap();
        let row = state.entities.row(slot);
        assert_eq!(row.uuid, uuid);
        assert_eq!(row.villager.as_ref().unwrap().inventory, inventory);
        assert!(
            row.villager.as_ref().unwrap().home.is_some(),
            "daughter member did not reach a roofed bed: pos={:?}",
            state.entities.pos(slot),
        );
    }
}
