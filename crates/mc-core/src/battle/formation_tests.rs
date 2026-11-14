use super::super::steering;
use super::*;

fn spec(id: u32, faction: u16, z: f64, files: u16, order: Order) -> RegimentSpec {
    RegimentSpec {
        frontage: 1.0,
        id,
        faction,
        origin: [0.0, 64.0, z],
        facing: [0.0, if faction == 1 { 1.0 } else { -1.0 }],
        files,
        order,
    }
}

fn add(runtime: &mut FormationRuntime, spec: RegimentSpec, first: i32, count: i32) {
    runtime
        .add_regiment(
            spec,
            &(first..first + count).map(EntityId).collect::<Vec<_>>(),
        )
        .unwrap();
}

fn simulate(runtime: &mut FormationRuntime, tick: u64) {
    let snapshot: FxHashMap<_, _> = runtime
        .members
        .iter()
        .filter(|m| m.alive)
        .map(|m| {
            (
                m.id,
                SoldierSample {
                    slot: None,
                    pos: m.pos,
                    health: 20.0,
                    faction: runtime.regiments[m.regiment].spec.faction,
                },
            )
        })
        .collect();
    runtime.prepare(tick, |id| snapshot.get(&id).copied());
    runtime.neighbours.rebuild(&runtime.members);
    let drives: Vec<_> = (0..runtime.members.len())
        .map(|i| steering::movement(runtime, i))
        .collect();
    for (member, drive) in runtime.members.iter_mut().zip(drives) {
        for (position, velocity) in member.pos.iter_mut().zip(drive) {
            *position += velocity * mc_physics::TICK_SECONDS;
        }
    }
    runtime.update_morale(tick);
}

#[test]
fn advancing_keeps_one_block_files_and_two_block_ranks() {
    let mut runtime = FormationRuntime::default();
    add(&mut runtime, spec(0, 1, 0.0, 10, Order::Advance), 1, 100);
    add(&mut runtime, spec(1, 2, 100.0, 10, Order::Hold), 101, 100);
    for tick in 0..300 {
        simulate(&mut runtime, tick);
    }
    assert!(runtime.regiments[0].spec.origin[2] > 15.0);
    for m in &runtime.members[..100] {
        let expected = slot_position(&runtime.regiments[0].spec, (m.id.0 - 1) as usize);
        assert!((m.pos[0] - expected[0]).abs() < 1e-8);
        assert!(
            (m.pos[2] - expected[2]).abs() < 0.08,
            "{} {:?} {:?}",
            m.id.0,
            m.pos,
            expected
        );
    }
    assert_eq!(runtime.regiments[0].sections.len(), 5);
}

#[test]
fn opposing_fronts_keep_three_to_five_block_standoff_without_blob_collapse() {
    let mut runtime = FormationRuntime::default();
    add(&mut runtime, spec(0, 1, 0.0, 20, Order::Advance), 1, 200);
    add(&mut runtime, spec(1, 2, 30.0, 20, Order::Advance), 201, 200);
    for tick in 0..650 {
        simulate(&mut runtime, tick);
        // Suppress pulses only in this geometry oracle. Their scheduling and
        // contact displacement have separate oracles below.
        for r in &mut runtime.regiments {
            for s in &mut r.sections {
                s.pulse_until = 0;
                s.cooldown_until = u64::MAX;
            }
        }
    }
    let gap = runtime.regiments[1].spec.origin[2] - runtime.regiments[0].spec.origin[2];
    assert!((3.0..=5.0).contains(&gap), "gap={gap}");
    for r in &runtime.regiments {
        let front = &runtime.members[r.start..r.start + 20];
        assert!((front[0].pos[0] - front[19].pos[0]).abs() > 18.9);
        for pair in front.windows(2) {
            assert!((pair[0].pos[0] - pair[1].pos[0]).abs() >= steering::MIN_SPACING);
        }
    }
}

#[test]
fn wounded_front_man_crawls_back_and_next_rank_takes_his_slot() {
    let mut runtime = FormationRuntime::default();
    add(&mut runtime, spec(0, 1, 0.0, 5, Order::Hold), 1, 100);
    runtime.mark_wounded(EntityId(1));
    assert!(!runtime.can_engage(EntityId(1)));
    for tick in 0..80 {
        simulate(&mut runtime, tick);
    }
    assert_eq!(runtime.context(EntityId(6)).unwrap().rank, 0);
    assert!(runtime.members[0].pos[2] < -2.0);
    assert!(runtime.members[5].pos[2].abs() < 0.08);
    assert!(!runtime.can_attack(EntityId(1)));
    assert!(!runtime.can_engage(EntityId(11)));
}

#[test]
fn seeded_surge_gate_matches_configured_two_percent_rate() {
    let starts = (0..100_000)
        .filter(|&decision| roll(41, 0, decision * 10) < SURGE_PROBABILITY)
        .count();
    assert!((1850..2150).contains(&starts), "starts={starts}");
    let mut runtime = FormationRuntime::default();
    add(&mut runtime, spec(0, 1, 0.0, 20, Order::Hold), 1, 20);
    add(&mut runtime, spec(1, 2, 4.0, 20, Order::Hold), 21, 20);
    let mut starts = 0;
    let mut decisions = 0;
    // Constant morale/stamina isolates scheduling from the independently tested
    // fatigue/morale feedback. Observe actual section transitions, not RNG only.
    for tick in 0..100_000 {
        runtime.tick = tick;
        let r = &mut runtime.regiments[0];
        r.gap = 4.0;
        r.enemy = Some([0.0, 64.0, 4.0]);
        r.stamina = 100.0;
        r.morale.value = 80.0;
        if tick % 10 == 0
            && r.sections[0].pulse_until <= tick
            && r.sections[0].cooldown_until <= tick
        {
            decisions += 1;
        }
        let before = r.sections[0].starts;
        runtime.update_sections(0);
        starts += runtime.regiments[0].sections[0].starts - before;
    }
    let rate = starts as f64 / f64::from(decisions);
    assert!(
        (0.015..0.025).contains(&rate),
        "{starts}/{decisions}={rate}"
    );
}

#[test]
fn front_only_closes_during_charge_pulse_and_withdraw_holds_files() {
    let mut runtime = FormationRuntime::default();
    add(&mut runtime, spec(0, 1, 0.0, 5, Order::Charge), 1, 100);
    add(&mut runtime, spec(1, 2, 4.0, 5, Order::Hold), 101, 100);
    for tick in 0..30 {
        simulate(&mut runtime, tick);
    }
    assert!(runtime.can_attack(EntityId(1)));
    assert!(!runtime.can_attack(EntityId(6)));
    assert!(runtime.members[0].pos[2] > 0.8);
    assert!((runtime.members[5].pos[2] + 2.0).abs() < 0.1);
    runtime.set_order(0, Order::Withdraw);
    let origin = runtime.regiments[0].spec.origin;
    for tick in 30..80 {
        simulate(&mut runtime, tick);
    }
    assert!(runtime.regiments[0].spec.origin[2] < origin[2] - 2.0);
}

#[test]
fn twenty_thousand_have_bounded_staggered_decision_and_neighbour_work() {
    let mut runtime = FormationRuntime::default();
    for regiment in 0..20 {
        let faction = if regiment < 10 { 1 } else { 2 };
        let mut s = spec(
            regiment,
            faction,
            if faction == 1 { 0.0 } else { 100.0 },
            50,
            Order::Hold,
        );
        s.origin[0] = f64::from(regiment % 10) * 60.0;
        add(&mut runtime, s, (regiment * 1000 + 1) as i32, 1000);
    }
    let mut total_sections = 0;
    for tick in 0..10 {
        simulate(&mut runtime, tick);
        let w = runtime.work;
        assert_eq!(w.soldiers, 20_000);
        assert_eq!(w.regiments, 2);
        assert_eq!(w.sections, 100);
        assert_eq!(w.distance_samples, 20);
        assert!(w.separation_candidates <= 20_000 * 25 * 8);
        assert_eq!(
            w.separation_candidates, 263_040,
            "pre-CSR frozen oracle work"
        );
        assert!(w.push_calls <= 20_000 * 5);
        assert_eq!(runtime.neighbours.overflow, 0);
        total_sections += w.sections;
        eprintln!("tick={tick} work={w:?}");
    }
    assert_eq!(total_sections, 1000);
}

#[test]
fn admission_is_atomic_and_partial_sections_remain_valid() {
    let mut runtime = FormationRuntime::default();
    let s = spec(0, 1, 0.0, 3, Order::Hold);
    assert!(
        runtime
            .add_regiment(s, &[EntityId(1), EntityId(1), EntityId(2)])
            .is_err()
    );
    assert!(runtime.members.is_empty());
    add(&mut runtime, s, 1, 23);
    assert_eq!(runtime.regiments[0].sections.len(), 2);
    assert!(runtime.set_fighting_interval(0, 2.0).is_ok());
    assert!(runtime.set_fighting_interval(0, 0.1).is_err());
    runtime.clear();
    assert!(runtime.members.is_empty());
    assert!(runtime.context(EntityId(1)).is_none());
}

#[test]
fn cavalry_frontage_is_shared_by_spawn_march_and_contact_slots() {
    let mut runtime = FormationRuntime::default();
    let mut cavalry = spec(0, 1, 0.0, 10, Order::Hold);
    cavalry.frontage = 2.0;
    add(&mut runtime, cavalry, 1, 100);
    assert_eq!(
        (runtime.members[0].pos[0] - runtime.members[1].pos[0]).abs(),
        2.0
    );
    for engaged in [false, true] {
        runtime.regiments[0].gap = if engaged { 4.0 } else { 100.0 };
        for bi in 0..100 {
            assert_eq!(runtime.destination(bi).0, slot_position(&cavalry, bi));
        }
    }
    assert!(runtime.set_fighting_interval(0, 1.0).is_err());
    assert!(runtime.set_fighting_interval(0, 2.0).is_ok());
}

#[test]
fn native_slot_cache_refreshes_and_archers_choose_current_nearest_live_roster() {
    let mut runtime = FormationRuntime::default();
    add(&mut runtime, spec(0, 1, 0.0, 5, Order::Hold), 1, 20);
    add(&mut runtime, spec(1, 2, 100.0, 5, Order::Hold), 101, 20);
    add(&mut runtime, spec(2, 2, 10.0, 5, Order::Hold), 201, 20);
    runtime.mark_wounded(EntityId(201));
    let mut samples: FxHashMap<_, _> = runtime
        .members
        .iter()
        .map(|m| {
            (
                m.id,
                SoldierSample {
                    slot: Some(EntitySlot(m.id.0 as u32)),
                    pos: m.pos,
                    health: 20.0,
                    faction: runtime.regiments[m.regiment].spec.faction,
                },
            )
        })
        .collect();
    samples.get_mut(&EntityId(202)).unwrap().health = 0.0;
    samples.get_mut(&EntityId(203)).unwrap().faction = 1;
    runtime.prepare(0, |id| samples.get(&id).copied());
    assert_eq!(runtime.members[0].slot, Some(EntitySlot(1)));
    assert_eq!(runtime.regiments[0].enemy_regiment, Some(2));
    for roll in 0..100 {
        let target = runtime.archer_target(0, roll).unwrap();
        assert!((204..=220).contains(&target.0));
    }
    assert_eq!(
        runtime.members[41].slot, None,
        "dead slot cannot feed dense columns"
    );
    assert_eq!(
        runtime.members[42].slot, None,
        "faction change cannot feed old regiment columns"
    );
    samples.get_mut(&EntityId(1)).unwrap().slot = Some(EntitySlot(999));
    samples.remove(&EntityId(2));
    for id in 204..=220 {
        samples.remove(&EntityId(id));
    }
    runtime.prepare(1, |id| samples.get(&id).copied());
    assert_eq!(
        runtime.members[0].slot,
        Some(EntitySlot(999)),
        "dense compaction refresh"
    );
    assert_eq!(
        runtime.members[1].slot, None,
        "removed row clears cached slot"
    );
    assert_eq!(
        runtime.archer_target(0, 17),
        None,
        "empty cached enemy roster must not yield stale IDs"
    );
    for bi in 0..runtime.members.len() {
        assert_eq!(
            runtime.can_attack_at(bi),
            runtime.can_attack(runtime.members[bi].id)
        );
    }
}
