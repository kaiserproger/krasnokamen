use super::*;
use crate::boundary::{ChunkPos, ConnectionId, PlayerChange, PlayerInput};
use crate::registry::CoreRegistry;
use crate::tick::run_tick;
use crate::world::ChunkWorld;
use std::sync::{Arc, LazyLock};
struct Empty(u32);
impl mc_world::ChunkGenerator for Empty {
    fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
        mc_world::Chunk::empty(
            pos,
            mc_world::BlockStateId(self.0),
            mc_data::Identifier::parse("minecraft:plains").unwrap(),
        )
    }
}
fn fixture() -> (TickState, EntityId, EntityId) {
    static REG: LazyLock<Arc<CoreRegistry>> =
        LazyLock::new(|| Arc::new(CoreRegistry::load(None).unwrap()));
    let registry = Arc::clone(&REG);
    let mut world = ChunkWorld::new(
        42,
        Arc::new(Empty(registry.states.air)),
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [8, 200, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    world.ensure_blocking(&[(ChunkPos { x: 0, z: 0 }, 0)].into_iter().collect());
    let mut state = TickState::new(registry, world, 0, 512);
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(1),
            uuid: uuid::Uuid::new_v4(),
            name: "swimmer".into(),
            permission_level: 0,
        }],
    );
    let actor = state.connections.entity_of(ConnectionId(1)).unwrap();
    let slot = state.entities.slot_of(actor).unwrap();
    state.entities.set_pos(slot, [8.5, 200.0, 8.5]);
    let water = state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:water").unwrap())
        .unwrap()
        .default
        .0;
    for x in 3..=13 {
        for y in 198..=203 {
            for z in 3..=13 {
                state.world.set_block(x, y, z, water);
            }
        }
    }
    state.world.fold_overlay();
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:dolphin").unwrap())
        .unwrap();
    let dolphin = state.spawn_mob(type_id, [11.5, 200.0, 8.5], 10.0);
    state.players.get_mut(&actor).unwrap().sprinting = true;
    state.tick = 10;
    (state, actor, dolphin)
}
#[test]
fn dolphin_grace_native_effect_refresh_speed_and_expiry() {
    let (mut state, actor, dolphin) = fixture();
    run(&mut state);
    let effects = &state.players[&actor].active_effects;
    assert_eq!(effects.get(EffectId::DolphinsGrace).unwrap().duration, 100);
    assert_eq!(effects.swim_speed_multiplier(), 5.0);
    assert!(state.outbox.values().flatten().any(|event|matches!(event,PlayerChange::MobEffect{effect,..} if effect.id==EffectId::DolphinsGrace)));
    state
        .players
        .get_mut(&actor)
        .unwrap()
        .active_effects
        .entries[0]
        .duration = 40;
    state.tick = 20;
    run(&mut state);
    assert_eq!(
        state.players[&actor]
            .active_effects
            .get(EffectId::DolphinsGrace)
            .unwrap()
            .duration,
        100
    );
    let slot = state.entities.slot_of(dolphin).unwrap();
    state.entities.set_pos(slot, [30.0, 200.0, 8.5]);
    state.tick = 30;
    run(&mut state);
    let player = state.players.get_mut(&actor).unwrap();
    for tick in 0..100 {
        player.active_effects.tick(&mut player.vitals, tick, 20.0);
    }
    assert!(!player.active_effects.has(EffectId::DolphinsGrace));
    assert_eq!(player.active_effects.swim_speed_multiplier(), 1.0);
}
#[test]
fn dolphin_grace_requires_swimming_water_and_live_companion() {
    let (mut state, actor, dolphin) = fixture();
    state.players.get_mut(&actor).unwrap().sprinting = false;
    run(&mut state);
    assert!(
        !state.players[&actor]
            .active_effects
            .has(EffectId::DolphinsGrace)
    );
    state.players.get_mut(&actor).unwrap().sprinting = true;
    let slot = state.entities.slot_of(dolphin).unwrap();
    state.entities.body_mut(slot).alive = false;
    run(&mut state);
    assert!(
        !state.players[&actor]
            .active_effects
            .has(EffectId::DolphinsGrace)
    );
}
#[test]
fn dolphin_fish_claim_is_atomic_and_native_state_roundtrips() {
    let (mut state, actor, dolphin) = fixture();
    let cod = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:cod").unwrap())
        .unwrap();
    state.players.get_mut(&actor).unwrap().inventory.slots[0] = mc_data::ItemStack::new(cod, 1);
    let mut intents = Vec::new();
    assert!(interact(
        &mut state,
        actor,
        dolphin,
        Hand::Main,
        &mut intents
    ));
    assert!(interact(
        &mut state,
        actor,
        dolphin,
        Hand::Main,
        &mut intents
    ));
    assert_eq!(state.merge(&mut intents), 1);
    assert!(state.players[&actor].inventory.slots[0].is_empty());
    let slot = state.entities.slot_of(dolphin).unwrap();
    let body = state.entities.body(slot);
    let row = state.entities.row(slot);
    assert!(row.entity_life.dolphin.as_ref().unwrap().got_fish);
    let image = crate::journal::EntityStateImage::from_row(body, row);
    let fields = image.to_fields(&state.registry.blocks);
    assert!(fields.contains(&("GotFish".into(), Tag::Byte(1))));
    let native = mc_data::entity_nbt::VanillaEntity {
        id: "minecraft:dolphin".into(),
        extra_fields: fields,
        ..Default::default()
    };
    let recovered =
        crate::journal::EntityStateImage::from_record(&state.registry, &native).unwrap();
    assert_eq!(recovered.entity_life.dolphin, image.entity_life.dolphin);
    assert_eq!(recovered.air_supply, image.air_supply);
    let serde: crate::journal::EntityStateImage =
        serde_json::from_slice(&serde_json::to_vec(&image).unwrap()).unwrap();
    assert_eq!(serde, image);
}
#[test]
fn dolphin_feed_debit_reaches_the_client_inventory() {
    // DOLPHIN-TREASURE gate: the fed fish was debited server-side but the client kept
    // showing it (no inventory sync after the claim), so the cod looked duplicated.
    let (mut state, actor, dolphin) = fixture();
    let cod = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:cod").unwrap())
        .unwrap();
    state.players.get_mut(&actor).unwrap().inventory.slots[0] = mc_data::ItemStack::new(cod, 1);
    let mut intents = Vec::new();
    assert!(interact(
        &mut state,
        actor,
        dolphin,
        Hand::Main,
        &mut intents
    ));
    state.outbox.clear();
    assert_eq!(state.merge(&mut intents), 1);
    assert!(state.players[&actor].inventory.slots[0].is_empty());
    let synced = state.outbox.get(&ConnectionId(1)).is_some_and(|queue| {
        queue
            .iter()
            .any(|change| matches!(change, crate::boundary::PlayerChange::MenuContents { .. }))
    });
    assert!(synced, "the debited cod must be resynced to the client");
}
#[test]
fn dolphin_follows_surfaces_and_plays_without_item_duplication() {
    let (mut state, actor, dolphin) = fixture();
    let block_at = |x, y, z| state.world.block_at(x, y, z);
    let pos = [11.5, 200.0, 8.5];
    let velocity = drive(
        pos,
        4800,
        1.2,
        None,
        std::iter::once([8.5, 200.0, 8.5]),
        std::iter::empty(),
        &block_at,
        &state.registry,
    )
    .unwrap();
    assert!(velocity[0] < 0.0);
    assert!(
        drive(
            pos,
            100,
            1.2,
            None,
            std::iter::empty(),
            std::iter::empty(),
            &block_at,
            &state.registry
        )
        .unwrap()[1]
            > 0.0
    );
    let stone = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    let item = crate::tick::spawn_item_entity(
        &mut state,
        [11.0, 200.0, 8.5],
        mc_data::ItemStack::new(stone, 3),
    );
    let item_slot = state.entities.slot_of(item).unwrap();
    state.entities.body_mut(item_slot).pickup_delay = 0;
    let count = state.entities.len();
    state.tick = 40;
    run(&mut state);
    assert_eq!(state.entities.len(), count);
    assert_eq!(
        state.entities.row(item_slot).item.as_ref().unwrap().count,
        3
    );
    let position = state.entities.pos(item_slot);
    assert!(position[0] > 11.0 && position[1] > 200.0 && position[2] > 8.5);
    let _ = (actor, dolphin);
}
#[test]
fn dolphin_air_depletes_underwater_and_refills_at_surface() {
    assert_eq!(breathe(4800, true), (4799, false));
    assert_eq!(breathe(-19, true), (0, true));
    assert_eq!(breathe(50, false), (4800, false));
    let (mut state, _, dolphin) = fixture();
    let slot = state.entities.slot_of(dolphin).unwrap();
    assert_eq!(state.entities.body(slot).air_supply, 4800);
    state.entities.body_mut(slot).air_supply = 130;
    run_tick(&mut state, &[]);
    assert_eq!(state.entities.body(slot).air_supply, 129);
    assert!(state.entities.vel(slot)[1] > 0.0);
    state.entities.set_pos(slot, [11.5, 203.8, 8.5]);
    run_tick(&mut state, &[]);
    assert_eq!(state.entities.body(slot).air_supply, 4800);
}
#[test]
fn dolphin_corrupt_gotfish_fails_closed_with_exact_field() {
    let record = mc_data::entity_nbt::VanillaEntity {
        id: "minecraft:dolphin".into(),
        extra_fields: vec![("GotFish".into(), Tag::Int(1))],
        ..Default::default()
    };
    assert_eq!(
        from_record(&record).unwrap_err().to_string(),
        crate::CoreError::Storage("dolphin GotFish: expected TAG_Byte boolean".into()).to_string()
    );
}

#[test]
fn fed_dolphin_locates_the_structure_through_its_normal_goal_tick() {
    // Normal path: feeding sets GotFish, the AI tick requests DOLPHIN_LOCATED,
    // the drained site drives the swim, and arrival clears GotFish.
    let reg = registry();
    let site = mc_world::BlockPos {
        x: 30,
        y: 200,
        z: 8,
    };
    let generator = Arc::new(Structured {
        air: reg.states.air,
        water: reg.states.water,
        chunk: (1, 0),
        site,
    });
    let (mut state, dolphin) = treasure_world(generator);
    let slot = state.entities.slot_of(dolphin).unwrap();
    let start = state.entities.pos(slot);
    // Park the only player far from both leads so following cannot mask the
    // target choice: the old approximation would swim at the chest on the
    // opposite side, the vanilla goal at the wreck 22 blocks to +x.
    let player = state.connections.entity_of(ConnectionId(1)).unwrap();
    let player_slot = state.entities.slot_of(player).unwrap();
    state
        .entities
        .set_pos(player_slot, [start[0] - 20.0, 200.0, start[2] + 20.0]);
    // A player-placed chest 8 blocks on the side opposite the wreck is not a
    // treasure target.
    let chest = state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:chest").unwrap())
        .unwrap()
        .default
        .0;
    state
        .world
        .set_block(start[0] as i32 - 8, start[1] as i32, start[2] as i32, chest);
    state.world.fold_overlay();
    feed_normally(&mut state, dolphin);
    // The goal must resolve its lead to the DOLPHIN_LOCATED wreck and nothing
    // else: the chest 8 blocks on -x is the old container approximation. The
    // goal only requests once its chunk is active and it sits in water, and the
    // locate worker then answers on its own thread: wait for that message
    // instead of racing it (a 300-tick spin finishes in about a millisecond and
    // loses the race under fleet load).
    let mut chosen = None;
    for _ in 0..300 {
        run_tick(&mut state, &[]);
        match crate::command_locate::dolphin_treasure(&state, dolphin) {
            Some(crate::command_locate::DolphinTreasure::Pending { .. }) => {
                assert!(
                    crate::command_locate::await_queued_search(
                        &mut state,
                        std::time::Duration::from_secs(30)
                    ),
                    "the locate worker must answer the dolphin search"
                );
            }
            Some(lead) => {
                chosen = Some(lead);
                break;
            }
            None => {}
        }
    }
    match chosen {
        Some(crate::command_locate::DolphinTreasure::Found { pos, .. }) => assert_eq!(
            pos,
            [30, 200, 8],
            "the lead must be the shipwreck, never the chest"
        ),
        other => panic!("a DOLPHIN_LOCATED structure must answer the search: {other:?}"),
    }
    // Holding the fish, it swims toward the wreck (+x), away from the chest (-x).
    let before = state.entities.pos(slot);
    for _ in 0..60 {
        run_tick(&mut state, &[]);
        if !got_fish(&state, slot) {
            break;
        }
    }
    let end = state.entities.pos(slot);
    assert!(
        end[0] > before[0] + 0.5,
        "the fed dolphin must swim toward the wreck: {end:?} from {before:?}"
    );
}

#[test]
fn a_blocked_route_gets_stuck_without_crossing_the_wall() {
    let reg = registry();
    let site = mc_world::BlockPos {
        x: 30,
        y: 200,
        z: 8,
    };
    let (mut state, dolphin) = treasure_world(Arc::new(Structured {
        air: reg.states.air,
        water: reg.states.water,
        chunk: (1, 0),
        site,
    }));
    // Keep both the start chunk and the wall chunk active with the player
    // between them; an inactive chunk cancels the held goal by design.
    let player = state.connections.entity_of(ConnectionId(1)).unwrap();
    let player_slot = state.entities.slot_of(player).unwrap();
    state.entities.set_pos(player_slot, [12.5, 200.0, 8.5]);
    let slot = state.entities.slot_of(dolphin).unwrap();
    let block = |name| {
        state
            .registry
            .blocks
            .block(&mc_data::Identifier::parse(name).unwrap())
            .unwrap()
            .default
            .0
    };
    let stone = block("minecraft:stone");
    for y in 198..=203 {
        for z in 3..=13 {
            state.world.set_block(13, y, z, stone);
        }
    }
    state.world.fold_overlay();
    let start = state.entities.pos(slot);
    feed_normally(&mut state, dolphin);
    for _ in 0..600 {
        run_tick(&mut state, &[]);
        if !got_fish(&state, slot) {
            break;
        }
        std::thread::yield_now();
    }
    assert!(
        state.entities.pos(slot)[0] < 13.0,
        "dolphin crossed the wall"
    );
    // The production fixture keeps the far chunk inactive, so exercise the
    // bounded stuck planner directly: repeated no-progress passes must report
    // stuck instead of steering at the wall forever.
    let at = state.entities.pos(slot);
    let mut stuck = false;
    for _ in 0..8 {
        stuck |= plan_treasure(&mut state, slot, at, [30, 200, 8], 999);
    }
    assert!(stuck, "three blocked waypoints are vanilla's stuck path");
    assert!(state.entities.pos(slot)[0] < start[0] + 5.0);
}

#[test]
fn treasure_navigation_is_cancelled_on_a_world_change() {
    let reg = registry();
    let (mut state, dolphin) = treasure_world(Arc::new(Structured {
        air: reg.states.air,
        water: reg.states.water,
        chunk: (1, 0),
        site: mc_world::BlockPos {
            x: 30,
            y: 200,
            z: 8,
        },
    }));
    feed_normally(&mut state, dolphin);
    for _ in 0..400 {
        run_tick(&mut state, &[]);
        if crate::command_locate::dolphin_navigation(&state, dolphin)
            .is_some_and(|nav| nav.target.is_some())
        {
            break;
        }
        std::thread::yield_now();
    }
    let old_run = crate::command_locate::dolphin_navigation(&state, dolphin)
        .expect("navigation before transfer")
        .run;
    assert!(
        crate::command_locate::dolphin_navigation(&state, dolphin)
            .is_some_and(|nav| nav.target.is_some()),
        "the fed dolphin must hold a water waypoint before transfer"
    );
    state.dimension = crate::dimensions::Dimension::Nether;
    for _ in 0..12 {
        run_tick(&mut state, &[]);
    }
    assert!(
        crate::command_locate::dolphin_navigation(&state, dolphin)
            .is_none_or(|nav| nav.run != old_run),
        "the old-world waypoint/run must not survive the transfer"
    );
}

#[test]
fn a_fed_dolphin_low_on_air_keeps_gotfish_and_waits() {
    let reg = registry();
    let (mut state, dolphin) = treasure_world(Arc::new(Structured {
        air: reg.states.air,
        water: reg.states.water,
        chunk: (1, 0),
        site: mc_world::BlockPos {
            x: 30,
            y: 200,
            z: 8,
        },
    }));
    let slot = state.entities.slot_of(dolphin).unwrap();
    let start = state.entities.pos(slot);
    feed_normally(&mut state, dolphin);
    state.entities.body_mut(slot).air_supply = 99;
    for _ in 0..12 {
        run_tick(&mut state, &[]);
    }
    assert!(
        got_fish(&state, slot),
        "low air stops the goal, not GotFish"
    );
    assert_eq!(
        crate::command_locate::dolphin_treasure(&state, dolphin),
        None,
        "the stopped goal keeps no lead while surfacing"
    );
    let now = state.entities.pos(slot);
    assert_eq!(
        [now[0], now[2]],
        [start[0], start[2]],
        "surfacing may bob vertically but not follow a treasure"
    );
}

#[test]
fn a_no_ai_fed_dolphin_does_not_admit_the_goal() {
    let reg = registry();
    let (mut state, dolphin) = treasure_world(Arc::new(Structured {
        air: reg.states.air,
        water: reg.states.water,
        chunk: (1, 0),
        site: mc_world::BlockPos {
            x: 30,
            y: 200,
            z: 8,
        },
    }));
    // A second ordinary wet dolphin makes the Grace pass run; NoAI must still
    // keep the fed dolphin out of treasure navigation.
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:dolphin").unwrap())
        .unwrap();
    state.spawn_mob(type_id, [5.5, 200.0, 8.5], 10.0);
    let slot = state.entities.slot_of(dolphin).unwrap();
    let start = state.entities.pos(slot);
    state.entities.row_mut(slot).controls.no_ai = true;
    feed_normally(&mut state, dolphin);
    for _ in 0..20 {
        run_tick(&mut state, &[]);
    }
    assert_eq!(
        crate::command_locate::dolphin_treasure(&state, dolphin),
        None,
        "NoAI admits no goal request"
    );
    assert_eq!(state.entities.pos(slot), start, "NoAI body must not move");
    assert!(got_fish(&state, slot), "NoAI keeps the fed state");
}

#[test]
fn a_player_chest_without_a_located_structure_is_not_a_treasure_lead() {
    // The old approximation led a fed dolphin to any chest/barrel. Vanilla
    // searches DOLPHIN_LOCATED structures only; with none in range the goal
    // stops (`stuck`) and clears GotFish instead of steering at a container.
    let reg = registry();
    let (mut state, dolphin) = treasure_world(Arc::new(EmptyWater {
        air: reg.states.air,
        water: reg.states.water,
    }));
    // A chest 10 blocks on +x (well past the old 4-block search) and the only
    // player parked off the axis behind the dolphin: old code swims at the
    // chest, vanilla stops with no DOLPHIN_LOCATED structure.
    let player = state.connections.entity_of(ConnectionId(1)).unwrap();
    let player_slot = state.entities.slot_of(player).unwrap();
    state
        .entities
        .set_pos(player_slot, [8.5, 200.0, 8.5 + 20.0]);
    let slot = state.entities.slot_of(dolphin).unwrap();
    let start = state.entities.pos(slot);
    let chest = state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:chest").unwrap())
        .unwrap()
        .default
        .0;
    state.world.set_block(
        start[0] as i32 + 10,
        start[1] as i32,
        start[2] as i32,
        chest,
    );
    state.world.fold_overlay();
    feed_normally(&mut state, dolphin);
    let chest_at = [start[0] + 10.0, start[1], start[2]];
    let mut closest = f64::INFINITY;
    let mut ever_found = false;
    for _ in 0..300 {
        run_tick(&mut state, &[]);
        match crate::command_locate::dolphin_treasure(&state, dolphin) {
            Some(crate::command_locate::DolphinTreasure::Found { .. }) => ever_found = true,
            Some(crate::command_locate::DolphinTreasure::Pending { .. }) => {
                // The worker answers on its own thread; a tick spin loses that
                // race under fleet load and would leave the goal pending.
                assert!(
                    crate::command_locate::await_queued_search(
                        &mut state,
                        std::time::Duration::from_secs(30)
                    ),
                    "the locate worker must answer the dolphin search"
                );
            }
            _ => {}
        }
        let at = state.entities.pos(slot);
        let dx = at[0] - chest_at[0];
        let dz = at[2] - chest_at[2];
        closest = closest.min(dx * dx + dz * dz);
        if !got_fish(&state, slot) {
            break;
        }
    }
    assert!(
        !got_fish(&state, slot),
        "a missing structure stops the goal and clears GotFish"
    );
    assert!(
        !ever_found,
        "no DOLPHIN_LOCATED structure exists, so the chest must never become a lead"
    );
    assert!(
        closest > 9.0,
        "a player chest must not attract the dolphin: closest squared {closest}"
    );
}

fn registry() -> Arc<CoreRegistry> {
    static REG: LazyLock<Arc<CoreRegistry>> =
        LazyLock::new(|| Arc::new(CoreRegistry::load(None).unwrap()));
    Arc::clone(&REG)
}

/// A generator planning one `minecraft:shipwreck` (a
/// `#minecraft:dolphin_located` member) at `chunk`, with the site at `site`.
struct Structured {
    air: u32,
    water: u32,
    chunk: (i32, i32),
    site: mc_world::BlockPos,
}

struct EmptyWater {
    air: u32,
    water: u32,
}

impl mc_world::ChunkGenerator for EmptyWater {
    fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
        water_chunk(pos, self.air, self.water)
    }
}

fn water_chunk(pos: mc_world::ChunkPos, air: u32, water: u32) -> mc_world::Chunk {
    let mut chunk = mc_world::Chunk::empty(
        pos,
        mc_world::BlockStateId(air),
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
    );
    for x in 0..16 {
        for y in 198..=203 {
            for z in 0..16 {
                chunk.set_block(x, y, z, mc_world::BlockStateId(water));
            }
        }
    }
    chunk
}

impl mc_world::ChunkGenerator for Structured {
    fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
        let mut chunk = water_chunk(pos, self.air, self.water);
        if (pos.x, pos.z) == self.chunk {
            mc_world::structure_starts::insert(
                &mut chunk,
                &mc_world::chunk::GeneratedStructureSite {
                    structure: mc_data::Identifier::parse("minecraft:shipwreck").unwrap(),
                    start_chunk: self.chunk,
                    position: self.site,
                },
                self.site,
                self.site,
            );
        }
        chunk
    }

    fn structure_candidates_in_ring(
        &self,
        targets: &[mc_data::Identifier],
        centre: (i32, i32),
        ring: u32,
    ) -> Vec<mc_world::ChunkPos> {
        let wreck = mc_data::Identifier::parse("minecraft:shipwreck").unwrap();
        (targets.contains(&wreck)
            && (self.chunk.0 - centre.0)
                .unsigned_abs()
                .max((self.chunk.1 - centre.1).unsigned_abs())
                == ring)
            .then_some(mc_world::ChunkPos {
                x: self.chunk.0,
                z: self.chunk.1,
            })
            .into_iter()
            .collect()
    }

    fn structure_sites_at(
        &self,
        targets: &[mc_data::Identifier],
        pos: mc_world::ChunkPos,
    ) -> Vec<mc_world::chunk::GeneratedStructureSite> {
        let wreck = mc_data::Identifier::parse("minecraft:shipwreck").unwrap();
        (pos.x == self.chunk.0 && pos.z == self.chunk.1 && targets.contains(&wreck))
            .then_some(mc_world::chunk::GeneratedStructureSite {
                structure: wreck,
                start_chunk: self.chunk,
                position: self.site,
            })
            .into_iter()
            .collect()
    }
}

fn treasure_world(generator: Arc<dyn mc_world::ChunkGenerator>) -> (TickState, EntityId) {
    let registry = registry();
    let mut world = ChunkWorld::new(
        42,
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [8, 200, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    world.ensure_blocking(&[(ChunkPos { x: 0, z: 0 }, 0)].into_iter().collect());
    let mut state = TickState::new(registry, world, 0, 512);
    state
        .world
        .set_active_chunks([(ChunkPos { x: 0, z: 0 })].into_iter().collect(), 0);
    let water = state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:water").unwrap())
        .unwrap()
        .default
        .0;
    let _ = water;
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(1),
            uuid: uuid::Uuid::new_v4(),
            name: "treasure-swimmer".into(),
            permission_level: 0,
        }],
    );
    let player = state.connections.entity_of(ConnectionId(1)).unwrap();
    let player_slot = state.entities.slot_of(player).unwrap();
    state.entities.set_pos(player_slot, [30.5, 200.0, 8.5]);
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:dolphin").unwrap())
        .unwrap();
    let dolphin = state.spawn_mob(type_id, [11.5, 200.0, 8.5], 10.0);
    let slot = state.entities.slot_of(dolphin).unwrap();
    state.entities.body_mut(slot).air_supply = 4800;
    (state, dolphin)
}

fn feed_normally(state: &mut TickState, dolphin: EntityId) {
    // Normal consumer path: player interaction intent through the merge.
    let actor = state.connections.entity_of(ConnectionId(1)).unwrap();
    let cod = mc_data::ItemStack::new(
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:cod").unwrap())
            .unwrap(),
        1,
    );
    state.players.get_mut(&actor).unwrap().inventory.slots[0] = cod;
    let mut intents = Vec::new();
    assert!(interact(state, actor, dolphin, Hand::Main, &mut intents));
    assert_eq!(state.merge(&mut intents), 1);
    let slot = state.entities.slot_of(dolphin).unwrap();
    assert!(got_fish(state, slot), "merge feeding must set GotFish");
}

fn got_fish(state: &TickState, slot: crate::entities::EntitySlot) -> bool {
    state
        .entities
        .row(slot)
        .entity_life
        .dolphin
        .as_ref()
        .is_some_and(|d| d.got_fish)
}
