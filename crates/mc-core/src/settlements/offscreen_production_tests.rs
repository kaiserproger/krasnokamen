//! Native terrain, inventory fences and durable settlement authority.
use super::*;
use crate::{CoreConfig, dimensions::Dimension, journal::JournalWriter};

struct Fixture {
    world: TickState,
    town: SettlementId,
    worker: Uuid,
    chest: [i32; 3],
}

fn fixture(world: TickState) -> Fixture {
    let (mut world, town, body, worker, table, stand, site) = setup_in(world);
    let slot = world.entities.slot_of(body).unwrap();
    world.entities.row_mut(slot).controls.no_ai = true;
    let chest = supply_chest(&mut world, town, 60);
    crate::settlements::economy::execute(
        &mut world,
        town,
        crate::settlements::economy::Request::Production(Request::Start {
            job: "supply".into(),
            worker,
            recipe: "krasnokamen-settlements:bread".into(),
            count: 20,
            station: table,
            stand_at: stand,
            building: worksite_ref(&site),
        }),
    )
    .unwrap();
    Fixture {
        world,
        town,
        worker,
        chest,
    }
}

fn job(f: &Fixture) -> &Job {
    &f.world
        .settlements
        .get(f.town)
        .unwrap()
        .economy
        .production
        .jobs["supply"]
}

fn visit(f: &mut Fixture, now: u64, simulated: bool) {
    f.world.tick = now;
    if simulated {
        ensure(&mut f.world);
    } else {
        f.world.tickets.clear();
        f.world.refresh_active_chunks(now);
    }
    super::super::super::economy::tick(&mut f.world);
}

fn total(f: &Fixture, name: &str) -> i32 {
    let id = item(&f.world, name, 1).item_id;
    f.world.block_entities[&(f.chest[0], f.chest[1], f.chest[2])]
        .slots
        .iter()
        .filter(|s| s.item_id == id)
        .map(|s| s.count)
        .sum()
}

fn fill(f: &mut Fixture, slots: Vec<ItemStack>) {
    f.world
        .block_entities
        .get_mut(&(f.chest[0], f.chest[1], f.chest[2]))
        .unwrap()
        .slots = slots;
    f.world.world.mark_block_entity_dirty(mc_world::BlockPos {
        x: f.chest[0],
        y: f.chest[1],
        z: f.chest[2],
    });
}
#[test]
fn offscreen_resident_station_never_writes_storage_or_carry() {
    let mut f = fixture(state());
    let due = job(&f).next_due;
    let period = job(&f).duration;
    let before = f.world.block_entities[&(f.chest[0], f.chest[1], f.chest[2])]
        .slots
        .clone();
    visit(&mut f, due + period * 2, false);
    assert!(f.world.world.has_chunk(0, 0));
    assert!(!f.world.world.is_active(0, 0));
    assert_eq!((job(&f).done, job(&f).deferred), (0, 3));
    assert_eq!(
        f.world.block_entities[&(f.chest[0], f.chest[1], f.chest[2])].slots,
        before
    );
    let now = f.world.tick + RETRY_TICKS;
    visit(&mut f, now, true);
    assert_eq!((job(&f).done, job(&f).deferred), (3, 0));
    assert_eq!(total(&f, "minecraft:bread"), 3);
    assert_eq!(total(&f, "minecraft:wheat"), 51);
    let saved = job(&f).clone();
    visit(&mut f, now, true);
    assert_eq!(job(&f), &saved);
    assert_eq!(total(&f, "minecraft:bread"), 3);
}

#[test]
fn offscreen_reconciliation_applies_affordable_portions_and_drops_rest() {
    for full_output in [false, true] {
        let mut f = fixture(state());
        let mut slots = vec![ItemStack::EMPTY; 27];
        if full_output {
            slots.fill(item(&f.world, "minecraft:stone", 64));
        }
        slots[0] = item(
            &f.world,
            "minecraft:wheat",
            if full_output { 64 } else { 6 },
        );
        if full_output {
            slots[1] = item(&f.world, "minecraft:bread", 63);
        }
        fill(&mut f, slots);
        let now = job(&f).next_due + 3 * job(&f).duration;
        visit(&mut f, now, false);
        assert_eq!(job(&f).deferred, 4);
        let now = now + RETRY_TICKS;
        visit(&mut f, now, true);
        assert_eq!(job(&f).deferred, 0);
        assert_eq!(job(&f).done, if full_output { 1 } else { 2 });
        assert_eq!(
            total(&f, "minecraft:bread"),
            if full_output { 64 } else { 2 }
        );
        let done = job(&f).done;
        let mut refill = vec![ItemStack::EMPTY; 27];
        refill[0] = item(&f.world, "minecraft:wheat", 60);
        fill(&mut f, refill);
        visit(&mut f, now, true);
        assert_eq!(
            job(&f).done,
            done,
            "discarded portions must not return after refill"
        );
        assert_eq!(total(&f, "minecraft:bread"), 0);
    }
}

#[test]
fn offscreen_frozen_owner_does_not_advance_either_watermark() {
    let mut f = fixture(state());
    let now = job(&f).next_due;
    visit(&mut f, now, false);
    let before = f
        .world
        .settlements
        .get(f.town)
        .unwrap()
        .economy
        .production
        .clone();
    crate::command_console::execute(
        &mut f.world,
        crate::command_console::CONNECTION,
        "tick freeze",
        4,
        &mut crate::boundary::ChangeSet::default(),
    );
    for _ in 0..(job(&f).duration * 2) {
        let _ = crate::tick::run_tick(&mut f.world, &[]);
    }
    assert_eq!(
        f.world.settlements.get(f.town).unwrap().economy.production,
        before
    );
    crate::command_console::execute(
        &mut f.world,
        crate::command_console::CONNECTION,
        "tick unfreeze",
        4,
        &mut crate::boundary::ChangeSet::default(),
    );
    let _ = crate::tick::run_tick(&mut f.world, &[]);
    assert_eq!(
        f.world.settlements.get(f.town).unwrap().economy.production,
        before,
        "frozen world ticks cannot become production periods on unfreeze"
    );
}

#[path = "offscreen_production_disk_tests.rs"]
mod disk;

#[test]
fn offscreen_consumption_bread_credit_matches_daily_online_totals() {
    for members in [2, 3] {
        let mut totals = Vec::new();
        for online in [true, false] {
            let mut f = fixture(state());
            f.world
                .settlements
                .get_mut(f.town)
                .unwrap()
                .economy
                .production
                .jobs
                .clear();
            for _ in 1..members {
                resident(&mut f.world, f.town, [7.5, 201.0, 8.5]);
            }
            let mut slots = vec![ItemStack::EMPTY; 27];
            slots[0] = item(&f.world, "minecraft:bread", 10);
            fill(&mut f, slots);
            visit(&mut f, 1, online);
            let first = f
                .world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .consumption
                .next_due
                .unwrap();
            for day in 0..4 {
                visit(&mut f, first + day * 24_000, online);
                if !online {
                    assert_eq!(total(&f, "minecraft:bread"), 10);
                }
            }
            visit(&mut f, first + 3 * 24_000 + RETRY_TICKS, true);
            let consumption = &f
                .world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .consumption;
            assert_eq!((consumption.debit, consumption.credit), (0, 0));
            assert_eq!(total(&f, "minecraft:bread"), 10 - members);
            totals.push(total(&f, "minecraft:bread"));
        }
        assert_eq!(totals[0], totals[1]);
    }
}

#[test]
fn offscreen_empty_larder_debit_is_dropped_not_repaid_after_refill() {
    let mut f = fixture(state());
    f.world
        .settlements
        .get_mut(f.town)
        .unwrap()
        .economy
        .production
        .jobs
        .clear();
    fill(&mut f, vec![ItemStack::EMPTY; 27]);
    visit(&mut f, 1, false);
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
    visit(&mut f, due, false);
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
    visit(&mut f, due + RETRY_TICKS, true);
    assert_eq!(
        f.world
            .settlements
            .get(f.town)
            .unwrap()
            .economy
            .production
            .consumption
            .debit,
        0
    );
    let mut slots = vec![ItemStack::EMPTY; 27];
    slots[0] = item(&f.world, "minecraft:bread", 10);
    fill(&mut f, slots);
    visit(&mut f, due + RETRY_TICKS, true);
    assert_eq!(total(&f, "minecraft:bread"), 10);
    visit(&mut f, due + 24_000, true);
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
fn offscreen_reconciliation_waits_for_declared_storage_residency() {
    let mut f = fixture(state());
    let now = job(&f).next_due;
    visit(&mut f, now, false);
    f.world
        .settlements
        .get_mut(f.town)
        .unwrap()
        .bootstrap_storage = Some([1600, 201, 8]);
    assert!(!f.world.world.has_chunk(100, 0));
    visit(&mut f, now + RETRY_TICKS, true);
    assert_eq!((job(&f).done, job(&f).deferred), (0, 1));
    assert_eq!(total(&f, "minecraft:wheat"), 60);
    f.world
        .settlements
        .get_mut(f.town)
        .unwrap()
        .bootstrap_storage = Some(f.chest);
    visit(&mut f, now + RETRY_TICKS + 1, true);
    assert_eq!((job(&f).done, job(&f).deferred), (0, 1));
    visit(&mut f, now + 2 * RETRY_TICKS, true);
    assert_eq!((job(&f).done, job(&f).deferred), (1, 0));
    assert_eq!(total(&f, "minecraft:bread"), 1);
}

#[test]
#[ignore = "manual native economy timing receipt"]
fn offscreen_idle_settlement_tick_cost() {
    let mut world = state();
    for index in 0..1000 {
        town(&mut world, &format!("idle-{index}"), [0, 201, 0], 16);
    }
    world.tick = 1001;
    super::super::super::economy::tick(&mut world);
    let before = persistence_image(&world);
    let started = std::time::Instant::now();
    for _ in 0..1000 {
        world.tick += 1;
        super::super::super::economy::tick(&mut world);
    }
    eprintln!(
        "offscreen economy: 1000 settlements x 1000 ticks: {:?}, {:?}/tick",
        started.elapsed(),
        started.elapsed() / 1000
    );
    assert_eq!(persistence_image(&world), before);
}

fn persistence_image(world: &TickState) -> Vec<serde_json::Value> {
    world
        .settlements
        .records()
        .map(|record| serde_json::to_value(record).unwrap())
        .collect()
}

#[test]
fn offscreen_new_zero_job_town_consumes_without_command_or_restart() {
    for generated in [false, true] {
        let mut world = state();
        floor(&mut world);
        let id = if generated {
            world
                .settlements
                .adopt_village(&mut world.settlement_sequence, "fresh-village", [8, 201, 8])
                .unwrap()
        } else {
            town(&mut world, "fresh-town", [8, 201, 8], 32)
        };
        let body = resident(&mut world, id, [7.5, 201.0, 8.5]);
        let worker = uuid_of(&world, body);
        let chest = supply_chest(&mut world, id, 0);
        let mut f = Fixture {
            world,
            town: id,
            worker,
            chest,
        };
        let mut slots = vec![ItemStack::EMPTY; 27];
        slots[0] = item(&f.world, "minecraft:bread", 10);
        fill(&mut f, slots);
        visit(&mut f, 1, true);
        let due = f
            .world
            .settlements
            .get(id)
            .unwrap()
            .economy
            .production
            .consumption
            .next_due
            .unwrap();
        visit(&mut f, due, true);
        assert!(
            f.world
                .settlements
                .get(id)
                .unwrap()
                .economy
                .production
                .jobs
                .is_empty()
        );
        assert_eq!(total(&f, "minecraft:bread"), 9, "generated={generated}");
    }
}

#[path = "offscreen_production_review_tests.rs"]
mod review;
