//! Review regressions for scheduling, transition authority and persisted bounds.
use super::*;

#[test]
fn fresh_unpaused_job_starts_at_due_without_deferred_work() {
    let mut f = fixture(state());
    let body = f
        .world
        .entities
        .body(f.world.entities.slot_of_uuid(f.worker).unwrap())
        .id;
    give(
        vec![item(&f.world, "minecraft:wheat", 3)],
        &mut f.world,
        body,
    );
    let due = job(&f).next_due;
    visit(&mut f, due - 1, true);
    assert_eq!(job(&f).done, 0);
    visit(&mut f, due, true);
    assert_eq!(job(&f).done, 1);
}

#[test]
fn simulated_fetch_wait_keeps_one_opportunity_until_inputs_arrive() {
    let mut f = fixture(state());
    fill(&mut f, vec![ItemStack::EMPTY; 27]);
    let due = job(&f).next_due;
    let now = due + 5 * job(&f).duration;
    visit(&mut f, due, true);
    visit(&mut f, now, true);
    assert_eq!(job(&f).next_due, due);
    assert!(job(&f).waiting_simulated);
    let body = f
        .world
        .entities
        .body(f.world.entities.slot_of_uuid(f.worker).unwrap())
        .id;
    give(
        vec![item(&f.world, "minecraft:wheat", 3)],
        &mut f.world,
        body,
    );
    visit(&mut f, now + RETRY_TICKS, true);
    assert_eq!(job(&f).done, 1);
    assert!(job(&f).next_due > now + RETRY_TICKS);
    assert!(!job(&f).waiting_simulated);
}

#[test]
fn deferred_jobs_sleep_until_retry_but_new_periods_still_accrue() {
    let mut f = fixture(state());
    let due = job(&f).next_due;
    visit(&mut f, due, false);
    assert_eq!(job(&f).retry_at, due + RETRY_TICKS);
    assert!(!offscreen::ready(job(&f), due + 1));
    assert!(offscreen::ready(job(&f), due + RETRY_TICKS));
    let before = serde_json::to_value(job(&f)).unwrap();
    visit(&mut f, due + RETRY_TICKS, false);
    assert_eq!(serde_json::to_value(job(&f)).unwrap(), before);
    assert_eq!(job(&f).retry_at, due + 2 * RETRY_TICKS);
    let next = job(&f).next_due;
    visit(&mut f, next, false);
    assert_eq!(job(&f).deferred, 2);
}

#[test]
fn overdue_parked_job_does_not_enter_cursor_before_retry() {
    let mut f = fixture(state());
    fill(&mut f, vec![ItemStack::EMPTY; 27]);
    let due = job(&f).next_due;
    visit(&mut f, due, true);
    assert!(job(&f).paused.is_some());
    assert_eq!(job(&f).next_due, due);
    assert!(!offscreen::ready(job(&f), due + 1));
    assert!(offscreen::ready(job(&f), due + RETRY_TICKS));
    let policy = &mut f
        .world
        .settlements
        .get_mut(f.town)
        .unwrap()
        .economy
        .production;
    let mut cursor = crate::settlements::economy::WorkCursor::default();
    cursor.run(
        &mut policy.jobs,
        |job| offscreen::ready(job, due + 1),
        |_, _| panic!("parked job consumed cursor budget before retry"),
    );
}

#[test]
fn consumption_waits_for_resident_storage_and_does_not_retry_each_tick() {
    let mut f = fixture(state());
    f.world
        .settlements
        .get_mut(f.town)
        .unwrap()
        .economy
        .production
        .jobs
        .clear();
    let mut slots = vec![ItemStack::EMPTY; 27];
    slots[0] = item(&f.world, "minecraft:bread", 10);
    fill(&mut f, slots);
    visit(&mut f, 1, true);
    let due = f
        .world
        .settlements
        .get(f.town)
        .unwrap()
        .economy
        .production
        .consumption
        .next_due
        .unwrap();
    f.world
        .settlements
        .get_mut(f.town)
        .unwrap()
        .bootstrap_storage = Some([1600, 201, 8]);
    visit(&mut f, due, true);
    assert_eq!(
        f.world
            .settlements
            .get(f.town)
            .unwrap()
            .economy
            .production
            .consumption
            .debit,
        1
    );
    f.world
        .settlements
        .get_mut(f.town)
        .unwrap()
        .bootstrap_storage = Some(f.chest);
    visit(&mut f, due + 1, true);
    assert_eq!(
        total(&f, "minecraft:bread"),
        10,
        "waiting retry remains armed"
    );
    visit(&mut f, due + RETRY_TICKS, true);
    assert_eq!(total(&f, "minecraft:bread"), 9);
    assert_eq!(
        f.world
            .settlements
            .get(f.town)
            .unwrap()
            .economy
            .production
            .consumption
            .credit,
        3
    );
}

#[test]
fn persisted_consumption_credit_bound_fails_closed() {
    let mut policy = State::default();
    policy.consumption.credit = 4;
    assert!(validate(&policy).is_err());
}

#[test]
fn persisted_consumption_debit_bound_fails_closed() {
    let mut policy = State::default();
    policy.consumption.debit = mc_script::MAX_SCRIPT_WORLD_TIME + 1;
    assert!(validate(&policy).is_err());
}

#[test]
fn persisted_consumption_future_bound_fails_closed() {
    let mut f = fixture(state());
    f.world
        .settlements
        .get_mut(f.town)
        .unwrap()
        .economy
        .production
        .consumption
        .next_due = Some(48_000);
    assert!(offscreen::restore_clocks(&mut f.world).is_err());
}

#[test]
fn persisted_legacy_clock_bound_fails_closed() {
    let mut f = fixture(state());
    let policy = &mut f
        .world
        .settlements
        .get_mut(f.town)
        .unwrap()
        .economy
        .production;
    policy.clock = offscreen::Clock::WorldTicks;
    let job = policy.jobs.get_mut("supply").unwrap();
    job.next_due = job.duration + 1;
    assert!(offscreen::restore_clocks(&mut f.world).is_err());
}

#[test]
#[ignore = "manual native economy timing receipt"]
fn offscreen_deferred_settlement_tick_cost() {
    let mut f = fixture(state());
    let template = job(&f).clone();
    for index in 1..1000 {
        // Admission performs real path queries; give each town a normal tick budget.
        f.world.tick += 1;
        f.world.tick_budget.reset(f.world.tick);
        let (survey, revision) = survey_bounds(&mut f.world, template.stand_at, template.station);
        let id = town(&mut f.world, &format!("deferred-{index}"), [8, 201, 8], 32);
        let site = shop(
            &mut f.world,
            &survey,
            revision,
            &format!("deferred-site-{index}"),
            template.stand_at,
            template.station,
        );
        certify(&mut f.world, id, &site);
        let body = resident(&mut f.world, id, [7.5, 201.0, 8.5]);
        let worker = uuid_of(&f.world, body);
        crate::settlements::economy::execute(
            &mut f.world,
            id,
            crate::settlements::economy::Request::Production(Request::Start {
                job: "supply".into(),
                worker,
                recipe: template.recipe.clone(),
                count: 20,
                station: template.station,
                stand_at: template.stand_at,
                building: worksite_ref(&site),
            }),
        )
        .unwrap_or_else(|error| panic!("town {index} admission: {error:?}"));
    }
    let now = f.world.tick + 2 * template.duration;
    visit(&mut f, now, false);
    assert!(
        f.world
            .settlements
            .records()
            .all(|r| r.economy.production.jobs["supply"].deferred > 0)
    );
    let started = std::time::Instant::now();
    for _ in 0..1000 {
        f.world.tick += 1;
        crate::settlements::economy::tick(&mut f.world);
    }
    eprintln!(
        "offscreen deferred economy: 1000 settlements x 1000 ticks: {:?}, {:?}/tick",
        started.elapsed(),
        started.elapsed() / 1000
    );
    assert!(
        f.world
            .settlements
            .records()
            .all(|r| r.economy.production.jobs["supply"].done == 0)
    );
    assert_eq!(total(&f, "minecraft:wheat"), 60);
}
