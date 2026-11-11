//! ISSUE-1-11 regression: a bound resident that slept in its own bed must stand
//! up when it wakes and walk back to its claimed job site in the morning.
//!
//! The wake stand-up search used to run at `at(pos).y`, but `at` ceils the
//! sleeping position (bed.y + 0.6875) to bed.y + 1, where the neighbour cells
//! have no floor. With a roof close above the bed the reposition always failed,
//! the A* start cell (the bed itself) is not standable for a 1.95-high body, so
//! the woken villager kept `navigation_search_failed` and never returned to work.
use crate::boundary::ChangeSet;
use crate::settlements::allocator::tests::{bed, block, test_state, villager};
use crate::settlements::allocator::{JOB_SITE_REACH_SQUARED, centre, distance};
use crate::tick::TickState;

fn step(state: &mut TickState) {
    state.tick_budget.reset(state.tick);
    let mut changes = ChangeSet::default();
    crate::settlements::allocator::tick(state, &mut changes);
    crate::villager::brain::tick(state, &mut changes);
    state.tick += 1;
}

#[test]
fn woken_bound_resident_stands_up_and_returns_to_its_job_site() {
    use crate::settlements::buildings::{BuildingKind, certify};
    use crate::villager::VillagerProfession as Profession;
    let mut state = test_state();
    state.day_time = 3000;
    let composter = block(&state, "minecraft:composter", &[]);
    state.world.set_block(4, 201, 4, composter);
    bed(&mut state, 12, 12);
    // Roof close above the bed: the bed cell is only standable while lying down.
    let stone = state.registry.states.stone;
    for dx in -1..=2 {
        for dz in -1..=2 {
            state.world.set_block(12 + dx, 203, 12 + dz, stone);
        }
    }
    state.world.fold_overlay();
    let id = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "qa".into(),
            "owner".into(),
            [8, 201, 8],
            48,
        )
        .unwrap();
    certify(
        &mut state,
        id,
        BuildingKind::Workshop(Profession::Farmer),
        mc_script::ScriptWorldBounds::new([3, 200, 3], [7, 204, 7]).unwrap(),
    )
    .unwrap();
    let slot = villager(&mut state, [2.5, 201.0, 2.5]);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    for _ in 0..400 {
        step(&mut state);
    }
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!(
        v.job_site,
        Some(crate::boundary::BlockPos { x: 4, y: 201, z: 4 }),
        "the bound resident claims the certified composter"
    );

    state.day_time = 18_000;
    for _ in 0..3000 {
        step(&mut state);
        if state.entities.row(slot).villager.as_ref().unwrap().sleeping {
            break;
        }
    }
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    assert!(
        v.sleeping,
        "the resident actually sleeps in its own bed at night"
    );
    assert_eq!(
        v.job_site,
        Some(crate::boundary::BlockPos { x: 4, y: 201, z: 4 }),
        "sleeping must not release the job claim"
    );

    state.day_time = 3000;
    let mut returned = false;
    for _ in 0..1500 {
        step(&mut state);
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        let pos = state.entities.pos(slot);
        if !v.sleeping
            && let Some(job) = v.job_site
            && distance(pos, centre(job)) <= JOB_SITE_REACH_SQUARED
        {
            returned = true;
            break;
        }
    }
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    assert!(
        !v.sleeping && v.activity == crate::villager::Activity::Work,
        "the woken resident resumes Work, got {:?}",
        v.activity
    );
    assert_eq!(
        v.job_site,
        Some(crate::boundary::BlockPos { x: 4, y: 201, z: 4 }),
        "the job claim survives the night"
    );
    assert!(
        returned,
        "woken resident must walk back to its job site, pos={:?}",
        state.entities.pos(slot)
    );
}

#[test]
fn claimed_job_site_assigns_its_profession_before_arrival() {
    use crate::settlements::buildings::{BuildingKind, certify};
    use crate::villager::VillagerProfession as Profession;
    let mut state = test_state();
    state.day_time = 3000;
    let composter = block(&state, "minecraft:composter", &[]);
    state.world.set_block(4, 201, 4, composter);
    state.world.fold_overlay();
    let id = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "qa".into(),
            "owner".into(),
            [8, 201, 8],
            48,
        )
        .unwrap();
    certify(
        &mut state,
        id,
        BuildingKind::Workshop(Profession::Farmer),
        mc_script::ScriptWorldBounds::new([3, 200, 3], [7, 204, 7]).unwrap(),
    )
    .unwrap();
    // Far beyond JOB_SITE_REACH of the only certified workstation.
    let slot = villager(&mut state, [12.5, 201.0, 12.5]);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    for _ in 0..400 {
        step(&mut state);
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        if v.job_site.is_some() {
            break;
        }
    }
    let v = state.entities.row(slot).villager.as_ref().unwrap();
    let job = v
        .job_site
        .expect("bound villager claims the certified composter");
    assert!(
        distance(state.entities.pos(slot), centre(job)) > JOB_SITE_REACH_SQUARED,
        "the villager must still be far from the station when the claim lands"
    );
    assert_eq!(
        v.data.profession,
        Profession::Farmer,
        "claiming a job site assigns its profession immediately (vanilla AssignProfessionFromJobSite), not only on arrival"
    );
}
