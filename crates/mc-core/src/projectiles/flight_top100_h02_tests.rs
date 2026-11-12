use super::*;
use mc_data::{Identifier, ItemStack};

fn item(state: &TickState, name: &str) -> ItemStack {
    ItemStack::new(
        state
            .registry
            .items
            .id_of(&Identifier::parse(name).unwrap())
            .unwrap(),
        1,
    )
}

fn mob(state: &mut TickState, name: &str, pos: [f64; 3]) -> EntityId {
    let kind = state
        .registry
        .entity_types
        .id_of(&Identifier::parse(name).unwrap())
        .unwrap();
    state.spawn_mob(kind, pos, 20.0)
}

fn scene() -> (TickState, EntityId) {
    let (mut state, player, _) = crate::projectiles::flight_tests::setup();
    let chunks: Vec<_> = (-5..=5)
        .flat_map(|x| (-2..=2).map(move |z| crate::boundary::ChunkPos { x, z }))
        .collect();
    state
        .world
        .ensure_blocking(&chunks.iter().map(|&chunk| (chunk, 0)).collect());
    for chunk in chunks {
        state.tickets.insert(format!("h02:{chunk:?}"), (chunk, 0));
    }
    state.refresh_active_chunks(state.tick);
    let slot = state.entities.slot_of(player).unwrap();
    state.entities.set_pos(slot, [-40.0, 201.0, 20.0]);
    (state, player)
}

fn arrow(
    state: &mut TickState,
    player: EntityId,
    from: [f64; 3],
    velocity: [f64; 3],
    piercing: u32,
) -> EntityId {
    let owner = state
        .entities
        .row(state.entities.slot_of(player).unwrap())
        .uuid;
    let mut record = ProjectileState::new(
        ProjectileKind::Arrow,
        Some(owner),
        item(state, "minecraft:arrow"),
    );
    record.pickup = true;
    record.piercing = piercing;
    record.punch = 2;
    super::super::launch(state, record, from, velocity).unwrap()
}

fn exhaustive(state: &mut TickState, intents: &mut Vec<Intent>, ids: &[EntityId]) {
    if ids.is_empty() {
        return;
    }
    let mut bodies = Vec::new();
    hittables(state, &mut bodies);
    let candidates: Vec<_> = (0..bodies.len()).collect();
    run_shots(
        state,
        intents,
        ids.iter().map(|&id| (id, candidates.as_slice())),
        &bodies,
        &mut Vec::new(),
    );
}

fn identities(state: &TickState) -> Vec<(String, String)> {
    state
        .entities
        .slots()
        .map(|slot| {
            let body = state.entities.body(slot);
            (
                state.entities.row(slot).uuid.to_string(),
                Uuid::from_u128(body.id.0 as u128).to_string(),
            )
        })
        .collect()
}

fn normalize(mut text: String, identities: &[(String, String)]) -> String {
    for (uuid, stable) in identities {
        text = text.replace(uuid, stable);
    }
    text
}

/// Includes ordered damage/source/sequence, hit UUID order and ownership, all
/// saved projectile fields, motion, surviving bodies, dropped stacks and custody.
fn digest(state: &TickState, intents: &[Intent], names: &[(String, String)]) -> String {
    let mut result = normalize(format!("{intents:?}"), names);
    for slot in state.entities.slots() {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        result.push_str(&format!(
            "{:?}:{:?}:{:?}:{:?}:{:?}:{:?};",
            body.id,
            body.alive,
            row.health,
            state.entities.pos(slot),
            state.entities.vel(slot),
            row.item
        ));
        if let Some(record) = row.projectile.as_deref() {
            let stable = normalize(serde_json::to_string(record).unwrap(), names);
            let stable: ProjectileState = serde_json::from_str(&stable).unwrap();
            result.push_str(&format!(
                "{:?}",
                crate::projectiles::persistence::to_fields(&stable, &state.registry.items)
            ));
            result.push_str(&format!("{stable:?}"));
        }
    }
    let mut players: Vec<_> = state.players.iter().collect();
    players.sort_unstable_by_key(|(id, _)| **id);
    for (id, player) in players {
        result.push_str(&format!("{id:?}:{:?};", player.inventory.slots));
    }
    result
}

#[test]
fn exhaustive_flight_oracle_covers_negative_seams_long_sweeps_ties_and_swap_removals() {
    for (origin, speed, piercing, shots) in
        [(-16.7, 2.0, 0, 1), (-48.0, 96.0, 7, 1), (-32.0, 64.0, 1, 3)]
    {
        let simulate = |indexed| {
            let (mut state, player) = scene();
            for (x, z) in [(-15.8, -0.1), (-15.8, 0.1), (0.2, 0.0), (31.9, 0.0)] {
                mob(&mut state, "minecraft:cow", [x, 201.0, z]);
            }
            // An out-of-range body must not affect either the hit or its order.
            mob(&mut state, "minecraft:cow", [70.0, 201.0, 25.0]);
            let ids: Vec<_> = (0..shots)
                .map(|_| {
                    arrow(
                        &mut state,
                        player,
                        [origin, 201.7, -0.1],
                        [speed, 0.0, 0.0],
                        piercing,
                    )
                })
                .collect();
            let names = identities(&state);
            let mut intents = Vec::new();
            if indexed {
                run(&mut state, &mut intents, ids.into_iter());
            } else {
                exhaustive(&mut state, &mut intents, &ids);
            }
            assert!(!intents.is_empty(), "fixture must actually hit a body");
            digest(&state, &intents, &names)
        };
        assert_eq!(
            simulate(true),
            simulate(false),
            "origin={origin}, speed={speed}, piercing={piercing}, shots={shots}"
        );
    }
}

#[test]
fn neighbouring_chunk_omission_changes_boundary_hit_oracle() {
    let (mut state, player) = scene();
    let victim = mob(&mut state, "minecraft:cow", [-15.8, 201.0, -0.1]);
    let id = arrow(&mut state, player, [-16.7, 201.7, -0.1], [2.0, 0.0, 0.0], 0);
    let mut bodies = Vec::new();
    hittables(&state, &mut bodies);
    let mut candidates = Vec::new();
    shot_candidates(&state, id, &bodies, &[], &mut Vec::new(), &mut candidates);
    assert!(candidates.iter().any(|&i| bodies[i].id == victim));
    // Negative-control mutant: retain only the projectile's starting chunk.
    let bad: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|&i| {
            bodies[i]
                .slot
                .is_some_and(|s| state.entities.chunk_of(s).x == -2)
        })
        .collect();
    let hit = |indices: &[usize]| {
        indices
            .iter()
            .filter_map(|&i| {
                let body = &bodies[i];
                segment_enters(
                    [-16.7, 201.7, -0.1],
                    [2.0, 0.0, 0.0],
                    body.min.map(|v| v - 0.3),
                    body.max.map(|v| v + 0.3),
                )
                .map(|t| (t, body.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, id)| id)
    };
    assert_eq!(hit(&candidates), Some(victim));
    assert_ne!(
        hit(&bad),
        hit(&candidates),
        "omitting the neighbouring chunk must break the oracle"
    );
}

#[test]
fn wide_moved_body_and_duplicate_memberships_remain_conservative() {
    let (mut state, player) = scene();
    let victim = mob(&mut state, "minecraft:cow", [50.0, 201.0, 0.0]);
    let slot = state.entities.slot_of(victim).unwrap();
    state.entities.set_half_width(slot, 36.0);
    state.entities.set_pos(slot, [18.0, 201.0, -0.1]);
    let id = arrow(&mut state, player, [-20.0, 201.7, -0.1], [4.0, 0.0, 0.0], 0);
    let mut bodies = Vec::new();
    hittables(&state, &mut bodies);
    let mut candidates = Vec::new();
    shot_candidates(&state, id, &bodies, &[], &mut Vec::new(), &mut candidates);
    assert!(candidates.iter().any(|&i| bodies[i].id == victim));
}

#[test]
fn stuck_pickup_keeps_dense_slot_priority_and_trident_ownership() {
    for kind in [ProjectileKind::Arrow, ProjectileKind::Trident] {
        let (mut state, owner) = scene();
        let pos = [-16.1, 201.0, -0.1];
        let slot = state.entities.slot_of(owner).unwrap();
        state.entities.set_pos(slot, [-15.9, 201.0, -0.1]);
        let uuid = state.entities.row(slot).uuid;
        let mut record = ProjectileState::new(
            kind,
            Some(uuid),
            item(
                &state,
                if kind == ProjectileKind::Arrow {
                    "minecraft:arrow"
                } else {
                    "minecraft:trident"
                },
            ),
        );
        record.pickup = true;
        record.stuck = true;
        assert!(pickup(&mut state, &record, pos, &mut Vec::new()));
        assert_eq!(
            state.players[&owner]
                .inventory
                .slots
                .iter()
                .map(|s| s.count.max(0))
                .sum::<i32>(),
            1
        );
    }
}

#[test]
fn riptide_candidates_match_exhaustive_swept_targets_in_dense_order() {
    let (mut state, actor) = scene();
    for x in [-48.0, -16.1, -15.9, 0.0, 31.9, 65.0] {
        mob(&mut state, "minecraft:cow", [x, 201.0, -0.1]);
    }
    let start = [-50.0, 201.0, -0.1];
    let end = [48.0, 201.0, -0.1];
    let slot = state.entities.slot_of(actor).unwrap();
    state.entities.set_pos(slot, end);
    let expected: Vec<_> = state
        .entities
        .slots()
        .filter_map(|slot| {
            let body = state.entities.body(slot);
            let at = state.entities.pos(slot);
            let min = [
                at[0] - body.half_width - 0.5,
                at[1] - 1.8,
                at[2] - body.half_width - 0.5,
            ];
            let max = [
                at[0] + body.half_width + 0.5,
                at[1] + body.height,
                at[2] + body.half_width + 0.5,
            ];
            (body.id != actor && segment_enters(start, [98.0, 0.0, 0.0], min, max).is_some())
                .then_some(body.id)
        })
        .collect();
    let stack = item(&state, "minecraft:trident");
    state.projectiles.riptides.insert(
        actor,
        super::super::weapons::Riptide {
            until: state.tick + 20,
            previous: start,
            hit: Vec::new(),
            stack,
        },
    );
    let mut intents = Vec::new();
    super::super::weapons::tick(&mut state, &mut intents);
    let actual: Vec<_> = intents
        .iter()
        .filter_map(|i| match i.kind {
            IntentKind::DamageEntity { target, .. } => Some(target),
            _ => None,
        })
        .collect();
    assert_eq!(actual, expected);
    assert!(!actual.is_empty());
    let hit = &state.projectiles.riptides[&actor].hit;
    assert_eq!(hit.len(), expected.len());
    super::super::weapons::tick(&mut state, &mut intents);
    assert_eq!(
        intents.len(),
        expected.len(),
        "riptide must not damage a UUID twice"
    );
}

#[test]
fn rejected_fast_or_nonfinite_flight_does_not_expand_world_query() {
    let (mut state, player) = scene();
    let id = arrow(&mut state, player, [-16.7, 201.7, -0.1], [2.0, 0.0, 0.0], 0);
    let slot = state.entities.slot_of(id).unwrap();
    let mut bodies = Vec::new();
    hittables(&state, &mut bodies);
    for speed in [f64::INFINITY, f64::NAN, 1e20] {
        state.entities.set_vel(slot, [speed; 3]);
        let mut candidates = Vec::new();
        shot_candidates(&state, id, &bodies, &[], &mut Vec::new(), &mut candidates);
        assert!(candidates.is_empty());
    }
}

#[test]
fn world_spanning_sweep_visits_only_existing_chunk_memberships() {
    let (mut state, _) = scene();
    mob(&mut state, "minecraft:cow", [30.0, 201.0, 30.0]);
    let mut slots = Vec::new();
    swept_slots(
        &state,
        [-30_000_000.0, 201.0, -30_000_000.0],
        [30_000_000.0, 201.0, 30_000_000.0],
        0.5,
        &mut slots,
    );
    assert_eq!(slots, state.entities.slots().collect::<Vec<_>>());
}

/// Diagnostic M1/M3-shaped owner micro, not the native-100 capacity gate.
/// Both paths include snapshot construction, collision, ordered intents and
/// retained projectile writeback. Scene creation is outside each timed window.
#[test]
#[ignore = "run in mem-run -x with --ignored --nocapture; diagnostic only"]
fn projectile_candidate_cpu_micro() {
    let count: usize = std::env::var("H02_RESIDENTS")
        .unwrap_or_else(|_| "100000".into())
        .parse()
        .unwrap();
    let (mut state, player) = scene();
    for index in 0..count {
        let x = 256.0 + (index % 1024) as f64 * 3.0;
        let z = 256.0 + (index / 1024) as f64 * 3.0;
        mob(&mut state, "minecraft:cow", [x, 201.0, z]);
    }
    // Real local collision and piercing work, not a Q=0-only comparison.
    mob(&mut state, "minecraft:cow", [-15.8, 201.0, -0.1]);
    mob(&mut state, "minecraft:cow", [-14.8, 201.0, -0.1]);
    let cpu_ticks = || {
        let stat = std::fs::read_to_string("/proc/thread-self/stat").unwrap();
        let fields: Vec<_> = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .collect();
        fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
    };
    for shots in [0usize, 100, 1000] {
        for indexed in [false, true, false, true] {
            let ids: Vec<_> = (0..shots)
                .map(|_| arrow(&mut state, player, [-16.7, 201.7, -0.1], [3.0, 0.0, 0.0], 4))
                .collect();
            let mut bodies = Vec::new();
            hittables(&state, &mut bodies);
            let mut h = 0;
            let mut slots = Vec::new();
            let mut candidates = Vec::new();
            for &id in &ids {
                candidates.clear();
                shot_candidates(&state, id, &bodies, &[], &mut slots, &mut candidates);
                h += candidates.len();
            }
            assert!(
                shots == 0 || h <= 2 * shots,
                "distant population must not enter per-shot narrow phase"
            );
            let mut intents = Vec::new();
            let cpu = cpu_ticks();
            let start = std::time::Instant::now();
            if indexed {
                run(&mut state, &mut intents, ids.iter().copied());
            } else {
                exhaustive(&mut state, &mut intents, &ids);
            }
            let wall_us = start.elapsed().as_micros();
            let cpu = cpu_ticks() - cpu;
            assert_eq!(intents.len(), 2 * shots);
            eprintln!(
                "H02_MICRO residents={count} shots={shots} indexed={indexed} candidates={} exhaustive_candidates={} wall_us={wall_us} thread_cpu_ticks={cpu} cpu_tick_hz=100",
                h,
                bodies.len() * shots
            );
            for id in ids {
                if state.entities.slot_of_any(id).is_some() {
                    super::super::remove(&mut state, id);
                }
            }
        }
    }
}

#[test]
fn fishing_item_and_arrow_saved_state_match_exhaustive_snapshot() {
    let simulate = |indexed| {
        let (mut state, owner) = scene();
        let owner_slot = state.entities.slot_of(owner).unwrap();
        state.entities.set_pos(owner_slot, [-18.0, 201.0, -0.1]);
        let rod = item(&state, "minecraft:fishing_rod");
        state.players.get_mut(&owner).unwrap().inventory.slots[0] = rod.clone();
        let uuid = state.entities.row(owner_slot).uuid;
        let stack = item(&state, "minecraft:diamond");
        let dropped = crate::tick::spawn_item_entity(&mut state, [-15.8, 201.0, -0.1], stack);
        let arrow = arrow(&mut state, owner, [-16.7, 201.3, -0.1], [2.0, 0.0, 0.0], 0);
        let record = ProjectileState::new(ProjectileKind::FishingHook, Some(uuid), rod);
        let hook = super::super::launch(&mut state, record, [-16.7, 201.3, -0.1], [2.0, 0.0, 0.0])
            .unwrap();
        let names = identities(&state);
        let mut intents = Vec::new();
        if indexed {
            run(&mut state, &mut intents, [arrow, hook].into_iter());
        } else {
            exhaustive(&mut state, &mut intents, &[arrow, hook]);
        }
        let slot = state.entities.slot_of(hook).unwrap();
        let super::super::ProjectileData::Fishing(fishing) =
            &state.entities.row(slot).projectile.as_deref().unwrap().data
        else {
            panic!("hook retained fishing state")
        };
        assert_eq!(
            fishing.hooked,
            Some(
                state
                    .entities
                    .row(state.entities.slot_of(dropped).unwrap())
                    .uuid
            )
        );
        assert!(intents.is_empty(), "arrow must pass the dropped item");
        digest(&state, &intents, &names)
    };
    assert_eq!(simulate(true), simulate(false));
}

#[test]
fn dragon_multipart_hit_and_retained_uuid_match_exhaustive_snapshot() {
    let simulate = |indexed| {
        let (mut state, owner) = scene();
        let dragon = mob(&mut state, "minecraft:ender_dragon", [32.0, 210.0, -0.1]);
        let slot = state.entities.slot_of(dragon).unwrap();
        state.end_fight.dragon_uuid = Some(state.entities.row(slot).uuid);
        let (min, max) = crate::end_fight::part_bounds(&state, EntityId(dragon.0 + 1)).unwrap();
        let at: [f64; 3] = std::array::from_fn(|axis| (min[axis] + max[axis]) * 0.5);
        let id = arrow(
            &mut state,
            owner,
            [at[0] - 8.0, at[1], at[2]],
            [16.0, 0.0, 0.0],
            4,
        );
        let names = identities(&state);
        let mut intents = Vec::new();
        if indexed {
            run(&mut state, &mut intents, [id].into_iter());
        } else {
            exhaustive(&mut state, &mut intents, &[id]);
        }
        assert!(!intents.is_empty());
        let record = state
            .entities
            .row(state.entities.slot_of(id).unwrap())
            .projectile
            .as_deref()
            .unwrap();
        assert_eq!(record.hit, vec![state.end_fight.dragon_uuid.unwrap()]);
        digest(&state, &intents, &names)
    };
    assert_eq!(simulate(true), simulate(false));
}

#[test]
fn deployed_candidate_outside_body_chunks_is_not_pruned() {
    let (mut state, owner) = scene();
    let stand = mob(&mut state, "minecraft:armor_stand", [70.0, 201.0, 25.0]);
    let id = arrow(&mut state, owner, [-16.7, 201.7, -0.1], [2.0, 0.0, 0.0], 0);
    let mut bodies = Vec::new();
    hittables(&state, &mut bodies);
    let exceptional: Vec<_> = bodies
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.deployed_candidate.then_some(i))
        .collect();
    let mut candidates = Vec::new();
    shot_candidates(
        &state,
        id,
        &bodies,
        &exceptional,
        &mut Vec::new(),
        &mut candidates,
    );
    assert!(candidates.iter().any(|&i| bodies[i].id == stand));
}

#[test]
fn shooter_grid_preserves_linear_target_ties_and_projectile_ownership() {
    let simulate = |indexed| {
        let (mut state, _) = scene();
        let shooter = mob(&mut state, "minecraft:shulker", [-16.0, 205.0, -0.1]);
        let mut targets = Vec::new();
        for pos in [[-14.0, 205.0, -0.1], [-18.0, 205.0, -0.1]] {
            let id = mob(&mut state, "minecraft:cow", pos);
            targets.push((id, pos));
        }
        for index in 0..64 {
            let at = [70.0, 205.0, index as f64 * 3.0];
            let id = mob(&mut state, "minecraft:cow", at);
            targets.push((id, at));
        }
        state.hostile.native.attacks.insert(
            shooter,
            crate::special_mobs::Attack {
                deadline: state.tick,
                ..Default::default()
            },
        );
        let grid = if indexed {
            crate::target_grid::TargetGrid::indexed(&targets)
        } else {
            crate::target_grid::TargetGrid::linear(&targets)
        };
        crate::special_mobs::run(
            &mut state,
            &mut Vec::new(),
            shooter,
            crate::blueprint::GoalKind::HomingShot,
            &grid,
        );
        let record = state
            .entities
            .slots()
            .find_map(|slot| state.entities.row(slot).projectile.as_deref())
            .expect("shooter must actually launch");
        assert_eq!(
            record.owner,
            Some(
                state
                    .entities
                    .row(state.entities.slot_of(shooter).unwrap())
                    .uuid
            )
        );
        assert_eq!(
            record.data,
            super::super::ProjectileData::Homing {
                target: state
                    .entities
                    .row(state.entities.slot_of(targets[0].0).unwrap())
                    .uuid
            }
        );
        normalize(serde_json::to_string(record).unwrap(), &identities(&state))
    };
    assert_eq!(simulate(true), simulate(false));
}

#[test]
fn pickup_matches_exhaustive_player_priority_after_swap_remove() {
    let simulate = |indexed, kind| {
        let (mut state, first) = scene();
        let removed = mob(&mut state, "minecraft:cow", [40.0, 201.0, 20.0]);
        crate::tick::run_tick(
            &mut state,
            &[crate::boundary::PlayerInput::Join {
                connection: crate::boundary::ConnectionId(2),
                uuid: Uuid::from_u128(200),
                name: "h02-second".into(),
                permission_level: 0,
            }],
        );
        let second = state
            .connections
            .entity_of(crate::boundary::ConnectionId(2))
            .unwrap();
        state
            .entities
            .remove(state.entities.slot_of(removed).unwrap());
        for (id, x) in [(first, -15.9), (second, -16.2)] {
            let slot = state.entities.slot_of(id).unwrap();
            state.entities.set_pos(slot, [x, 201.0, -0.1]);
        }
        let second_uuid = state
            .entities
            .row(state.entities.slot_of(second).unwrap())
            .uuid;
        let mut record =
            ProjectileState::new(kind, Some(second_uuid), item(&state, "minecraft:trident"));
        record.pickup = true;
        let pos = [-16.1, 201.0, -0.1];
        let names = identities(&state);
        let picked = if indexed {
            pickup(&mut state, &record, pos, &mut Vec::new())
        } else {
            let mut picked = false;
            for index in 0..state.entities.len() as u32 {
                let slot = EntitySlot(index);
                let body = state.entities.body(slot);
                let id = body.id;
                if body.alive
                    && state.players.get(&id).is_some_and(|p| !p.spectator())
                    && state
                        .entities
                        .pos(slot)
                        .iter()
                        .zip(pos)
                        .map(|(a, b)| (a - b).powi(2))
                        .sum::<f64>()
                        < 2.25
                    && (kind != ProjectileKind::Trident
                        || super::super::owner_id(&state, &record) == Some(id))
                    && give(&mut state, id, record.stack.clone())
                {
                    picked = true;
                    break;
                }
            }
            picked
        };
        assert!(picked);
        let recipient = if kind == ProjectileKind::Trident {
            second
        } else {
            first
        };
        assert_eq!(
            state.players[&recipient]
                .inventory
                .slots
                .iter()
                .map(|s| s.count.max(0))
                .sum::<i32>(),
            1
        );
        digest(&state, &[], &names)
    };
    for kind in [ProjectileKind::Arrow, ProjectileKind::Trident] {
        assert_eq!(simulate(true, kind), simulate(false, kind));
    }
}
