//! Break-it regressions on settlement claims (BreakIssue1x11): a workstation
//! moved inside its certified building keeps the job (F2), and a resident
//! never holds an open-air bed it will not sleep in (F4).
use super::tests::{bed, block, test_state, villager};
use crate::boundary::{BlockPos, ChangeSet};
use crate::entities::EntitySlot;
use crate::settlements::SettlementId;
use crate::settlements::buildings::{BuildingKind, certify};
use crate::tick::TickState;
use crate::villager::VillagerProfession as Profession;

fn step(state: &mut TickState) {
    state.tick_budget.reset(state.tick);
    let mut changes = ChangeSet::default();
    super::tick(state, &mut changes);
    crate::villager::brain::tick(state, &mut changes);
    state.tick += 1;
}

fn settlement(state: &mut TickState) -> SettlementId {
    state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "qa".into(),
            "owner".into(),
            [8, 201, 8],
            48,
        )
        .unwrap()
}

fn bound_villager(state: &mut TickState, id: SettlementId, pos: [f64; 3]) -> EntitySlot {
    let slot = villager(state, pos);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(state, id, entity).unwrap();
    slot
}

#[test]
fn workstation_moved_inside_its_certified_workshop_keeps_the_job() {
    let mut state = test_state();
    state.day_time = 3000;
    let composter = block(&state, "minecraft:composter", &[]);
    state.world.set_block(4, 201, 4, composter);
    state.world.fold_overlay();
    let id = settlement(&mut state);
    let workshop = certify(
        &mut state,
        id,
        BuildingKind::Workshop(Profession::Farmer),
        mc_script::ScriptWorldBounds::new([3, 200, 3], [7, 204, 7]).unwrap(),
    )
    .unwrap();
    let slot = bound_villager(&mut state, id, [2.5, 201.0, 2.5]);
    for _ in 0..400 {
        step(&mut state);
    }
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!(v.job_site, Some(BlockPos { x: 4, y: 201, z: 4 }));
    assert_eq!(v.data.profession, Profession::Farmer);

    // The player moves the composter two blocks within the same workshop.
    let air = state.registry.states.air;
    state.world.set_block(4, 201, 4, air);
    state.world.set_block(4, 201, 6, composter);
    state.world.fold_overlay();
    let moved = BlockPos { x: 4, y: 201, z: 6 };
    let mut lost = false;
    for _ in 0..800 {
        step(&mut state);
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        lost |= v.data.profession == Profession::None;
        if v.job_site == Some(moved) && v.data.profession == Profession::Farmer {
            break;
        }
    }
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    assert!(lost, "breaking the old composter ends the novice's job");
    assert_eq!(
        v.job_site,
        Some(moved),
        "the jobless resident claims the re-placed composter in its work period"
    );
    assert_eq!(v.data.profession, Profession::Farmer);
    assert_eq!(
        state.settlements.get(id).unwrap().buildings[&workshop].work_points,
        vec![[4, 201, 6]],
        "the certified slot follows its station"
    );
}

#[test]
fn bound_resident_leaves_an_open_air_bed_to_a_villager_that_sleeps_in_it() {
    let mut state = test_state();
    state.day_time = 18_000;
    let open_air = bed(&mut state, 12, 12);
    state.world.fold_overlay();
    let id = settlement(&mut state);
    // A homeless resident searches first: it shelters at night, so an
    // open-air bed would stay its unused claim forever.
    let resident = bound_villager(&mut state, id, [2.5, 201.0, 2.5]);
    for _ in 0..60 {
        step(&mut state);
    }
    assert_eq!(
        state.entities.row(resident).villager.as_ref().unwrap().home,
        None,
        "a resident claims only a bed it sleeps in"
    );
    let vanilla = villager(&mut state, [10.5, 201.0, 14.5]);
    let mut slept = false;
    for _ in 0..3000 {
        step(&mut state);
        if state
            .entities
            .row(vanilla)
            .villager
            .as_ref()
            .unwrap()
            .sleeping
        {
            slept = true;
            break;
        }
    }
    let v = state.entities.row(vanilla).villager.as_ref().unwrap();
    assert_eq!(
        v.home,
        Some(open_air),
        "the unbound villager claims the free bed"
    );
    assert!(slept, "and sleeps in it at night");
    assert_eq!(
        state.entities.row(resident).villager.as_ref().unwrap().home,
        None
    );
}
