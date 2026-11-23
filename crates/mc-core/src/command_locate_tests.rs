//! Failing-first tests for `/locate structure|biome|poi`.
use std::collections::HashMap;
use std::sync::Arc;

use mc_data::Identifier;
use mc_nbt::Tag;
use mc_world::{BlockPos, BlockRegistry, BlockStateId, Chunk, ChunkGenerator, ChunkPos};

use super::{
    LocateCommand, ResourceOrTag, SavedStarts, StructureSearchOptions, found_distance, poi_states,
    sample_heights, search_biome, search_poi, search_structure, square_ring,
};
use crate::boundary::{ChangeSet, ConnectionId, PlayerChange};
use crate::commands::parse;
use crate::tick::TickState;

#[path = "command_locate_crystal_tests.rs"]
mod crystal;

#[path = "command_locate_placement_tests.rs"]
mod placement;

fn id(name: &str) -> Identifier {
    Identifier::parse(name).unwrap()
}

/// Fake generator: fixed village sites and a synthetic biome field.
struct FakeGenerator {
    sites: Vec<mc_world::GeneratedVillageSite>,
}

impl FakeGenerator {
    fn site(start_chunk: (i32, i32), structure: &str) -> mc_world::GeneratedVillageSite {
        mc_world::GeneratedVillageSite {
            start_chunk,
            structure: id(structure),
            min: BlockPos {
                x: start_chunk.0 * 16,
                y: 60,
                z: start_chunk.1 * 16,
            },
            max: BlockPos {
                x: start_chunk.0 * 16 + 15,
                y: 80,
                z: start_chunk.1 * 16 + 15,
            },
            pieces: Vec::new(),
        }
    }
}

impl ChunkGenerator for FakeGenerator {
    fn generate(&self, pos: ChunkPos) -> Chunk {
        Chunk::empty(pos, BlockStateId(0), id("minecraft:plains"))
    }

    fn surface_height(&self, _world_x: i32, _world_z: i32) -> Option<i32> {
        Some(64)
    }

    fn village_sites_in_region(
        &self,
        min_chunk: (i32, i32),
        max_chunk: (i32, i32),
    ) -> Vec<mc_world::GeneratedVillageSite> {
        self.sites
            .iter()
            .filter(|site| {
                site.start_chunk.0 >= min_chunk.0
                    && site.start_chunk.0 <= max_chunk.0
                    && site.start_chunk.1 >= min_chunk.1
                    && site.start_chunk.1 <= max_chunk.1
            })
            .cloned()
            .collect()
    }

    fn structure_candidates_in_ring(
        &self,
        targets: &[Identifier],
        centre: (i32, i32),
        ring: u32,
    ) -> Vec<ChunkPos> {
        // Synthetic placement cells are one chunk wide.
        self.sites
            .iter()
            .filter(|site| {
                targets.contains(&site.structure)
                    && (site.start_chunk.0 - centre.0)
                        .unsigned_abs()
                        .max((site.start_chunk.1 - centre.1).unsigned_abs())
                        == ring
            })
            .map(|site| ChunkPos {
                x: site.start_chunk.0,
                z: site.start_chunk.1,
            })
            .collect()
    }

    fn structure_sites_at(
        &self,
        targets: &[Identifier],
        pos: ChunkPos,
    ) -> Vec<mc_world::chunk::GeneratedStructureSite> {
        self.sites
            .iter()
            .filter(|site| site.start_chunk == (pos.x, pos.z) && targets.contains(&site.structure))
            .map(|site| mc_world::chunk::GeneratedStructureSite {
                structure: site.structure.clone(),
                start_chunk: site.start_chunk,
                position: BlockPos {
                    x: pos.x * 16 + 8,
                    y: 64,
                    z: pos.z * 16 + 8,
                },
            })
            .collect()
    }

    fn noise_biome(&self, world_x: i32, _y: i32, world_z: i32) -> Option<Identifier> {
        // Deterministic synthetic field on the 32-block sampling grid.
        if ((world_x >> 5) ^ (world_z >> 5)) & 1 == 0 {
            Some(id("minecraft:plains"))
        } else {
            Some(id("minecraft:desert"))
        }
    }
}

#[test]
fn locate_uses_non_village_start_and_actual_underground_height() {
    struct Structures;
    impl ChunkGenerator for Structures {
        fn generate(&self, _: ChunkPos) -> Chunk {
            panic!("locate must not generate chunks")
        }
        fn structure_candidates_in_ring(
            &self,
            _: &[Identifier],
            _: (i32, i32),
            ring: u32,
        ) -> Vec<ChunkPos> {
            if ring == 0 {
                vec![ChunkPos { x: 128, z: -2 }, ChunkPos { x: 7, z: 8 }]
            } else {
                Vec::new()
            }
        }
        fn structure_sites_in_region(
            &self,
            min: (i32, i32),
            max: (i32, i32),
        ) -> Vec<mc_world::chunk::GeneratedStructureSite> {
            assert_eq!(min, max);
            if min == (7, 8) {
                return Vec::new();
            }
            assert_eq!(min, (128, -2));
            vec![mc_world::chunk::GeneratedStructureSite {
                structure: id("minecraft:stronghold"),
                position: BlockPos {
                    x: 2048,
                    y: -31,
                    z: -32,
                },
                start_chunk: (128, -2),
            }]
        }
    }
    let hit = search_structure(
        &Structures,
        (0, 64, 0),
        &[id("minecraft:stronghold")],
        &SavedStarts::new(),
        &StructureSearchOptions::default(),
        None,
    )
    .unwrap()
    .expect("native stronghold start");
    assert_eq!(hit.pos, (2048, -31, -32));
    let mut saved = SavedStarts::new();
    saved.insert(ChunkPos { x: 128, z: -2 }, Vec::new());
    assert!(
        search_structure(
            &Structures,
            (0, 64, 0),
            &[id("minecraft:stronghold")],
            &saved,
            &StructureSearchOptions::default(),
            None,
        )
        .unwrap()
        .is_none(),
        "covered empty start must suppress procedural plan"
    );
    saved.insert(
        ChunkPos { x: 7, z: 8 },
        vec![mc_world::chunk::GeneratedStructureSite {
            structure: id("minecraft:stronghold"),
            start_chunk: (7, 8),
            position: BlockPos {
                x: 117,
                y: -20,
                z: 130,
            },
        }],
    );
    let imported = search_structure(
        &Structures,
        (0, 64, 0),
        &[id("minecraft:stronghold")],
        &saved,
        &StructureSearchOptions::default(),
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        imported.pos,
        (117, -20, 130),
        "saved start at a candidate wins even when generation rejects it"
    );
}

#[test]
fn locate_namespaced_village_prefix_uses_structure_sites() {
    struct Structures;
    impl ChunkGenerator for Structures {
        fn generate(&self, _: ChunkPos) -> Chunk {
            panic!("locate must not generate chunks");
        }
        fn structure_candidates_in_ring(
            &self,
            _: &[Identifier],
            _: (i32, i32),
            ring: u32,
        ) -> Vec<ChunkPos> {
            if ring == 0 {
                vec![ChunkPos { x: 2, z: 3 }]
            } else {
                Vec::new()
            }
        }
        fn structure_sites_in_region(
            &self,
            _: (i32, i32),
            _: (i32, i32),
        ) -> Vec<mc_world::chunk::GeneratedStructureSite> {
            vec![mc_world::chunk::GeneratedStructureSite {
                structure: id("example:village_tower"),
                position: BlockPos {
                    x: 40,
                    y: 70,
                    z: 56,
                },
                start_chunk: (2, 3),
            }]
        }
    }
    let hit = search_structure(
        &Structures,
        (0, 64, 0),
        &[id("example:village_tower")],
        &SavedStarts::new(),
        &StructureSearchOptions::default(),
        None,
    )
    .unwrap()
    .expect("namespaced village-prefix structure is found");
    assert_eq!(hit.found_id, id("example:village_tower"));
    assert_eq!(hit.pos, (40, 70, 56));
}

fn fake_generator() -> FakeGenerator {
    let a = FakeGenerator::site((3, -2), "minecraft:village_plains");
    let b = FakeGenerator::site((-5, 4), "minecraft:village_desert");
    FakeGenerator { sites: vec![a, b] }
}

#[test]
fn parse_locate_grammar() {
    assert!(matches!(
        parse("locate structure minecraft:village_plains", true),
        Ok(crate::commands::Command::Locate(LocateCommand::Structure(target)))
        if !target.tag && target.id.as_str() == "minecraft:village_plains"
    ));
    assert!(matches!(
        parse("locate biome #minecraft:is_forest", true),
        Ok(crate::commands::Command::Locate(LocateCommand::Biome(target)))
        if target.tag && target.id.as_str() == "minecraft:is_forest"
    ));
    assert!(matches!(
        parse("locate poi meeting", true),
        Ok(crate::commands::Command::Locate(LocateCommand::Poi(target)))
        if !target.tag && target.id.as_str() == "minecraft:meeting"
    ));
    // Bare words default to the minecraft namespace.
    assert!(matches!(
        parse("locate structure village_plains", true),
        Ok(crate::commands::Command::Locate(LocateCommand::Structure(target)))
        if target.id.as_str() == "minecraft:village_plains"
    ));
    // Incomplete or unknown kinds are usage errors.
    assert!(parse("locate", true).is_err());
    assert!(parse("locate structure", true).is_err());
    assert!(parse("locate dimension overworld", true).is_err());
    assert!(parse("locate structure a b", true).is_err());
    // Non-operators never see the command.
    assert!(parse("locate structure minecraft:village_plains", false).is_err());
}

#[test]
fn square_ring_visits_rings_in_order() {
    assert_eq!(square_ring((0, 0), 0), vec![(0, 0)]);
    let ring1 = square_ring((0, 0), 1);
    assert_eq!(ring1.len(), 8);
    assert!(ring1.contains(&(1, 1)) && ring1.contains(&(-1, -1)));
    let ring2 = square_ring((5, -3), 2);
    assert_eq!(ring2.len(), 16);
    assert!(
        ring2
            .iter()
            .all(|(x, z)| (x - 5).abs() == 2 || (z + 3).abs() == 2)
    );
}

#[test]
fn structure_search_finds_nearest_matching_site() {
    let generator = fake_generator();
    let site_structures = SavedStarts::new();
    let hit = search_structure(
        &generator,
        (0, 64, 0),
        &[id("minecraft:village_desert")],
        &site_structures,
        &StructureSearchOptions::default(),
        None,
    )
    .unwrap()
    .expect("desert village exists");
    assert_eq!(hit.found_id.as_str(), "minecraft:village_desert");
    assert_eq!(hit.pos, (-5 * 16 + 8, 64, 4 * 16 + 8));
    let expected = ((hit.pos.0).pow(2) + (hit.pos.2).pow(2)) as f64;
    assert_eq!(hit.distance, expected.sqrt().floor() as i32);
    // Non-village structures resolve but never match generated sites.
    assert!(
        search_structure(
            &generator,
            (0, 64, 0),
            &[id("minecraft:stronghold")],
            &site_structures,
            &StructureSearchOptions::default(),
            None,
        )
        .unwrap()
        .is_none()
    );
    // Empty targets never match.
    assert!(
        search_structure(
            &generator,
            (0, 64, 0),
            &[],
            &site_structures,
            &StructureSearchOptions::default(),
            None,
        )
        .unwrap()
        .is_none()
    );
    // Covered empty village starts suppress generation.
    assert!(
        search_structure(
            &generator,
            (0, 64, 0),
            &[id("minecraft:village_plains")],
            &HashMap::from([(ChunkPos { x: 3, z: -2 }, Vec::new())]),
            &StructureSearchOptions::default(),
            None,
        )
        .unwrap()
        .is_none()
    );
    // With one-chunk synthetic placement cells, 101 cells exceeds 100 rings.
    let far = FakeGenerator::site((101, 0), "minecraft:village_plains");
    let far_generator = FakeGenerator { sites: vec![far] };
    let far_map = SavedStarts::new();
    assert!(
        search_structure(
            &far_generator,
            (0, 64, 0),
            &[id("minecraft:village_plains")],
            &far_map,
            &StructureSearchOptions::default(),
            None,
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn biome_search_matches_spiral_order_and_distance() {
    let generator = fake_generator();
    let noise = |x: i32, y: i32, z: i32| generator.noise_biome(x, y, z);
    // The origin column is sampled first: an origin-biome target hits at 0.
    let origin_biome = noise(0, 64, 0).unwrap();
    let hit = search_biome(
        &noise,
        (0, 64, 0),
        std::slice::from_ref(&origin_biome),
        -64,
        320,
    )
    .expect("origin column matches");
    assert_eq!(hit.distance, 0);
    assert_eq!(hit.found_id, origin_biome);
    assert_eq!((hit.pos.0, hit.pos.2), (0, 0));
    // A desert target from the origin finds the nearest desert column.
    let hit = search_biome(&noise, (0, 64, 0), &[id("minecraft:desert")], -64, 320)
        .expect("desert exists in the synthetic field");
    assert_eq!(hit.found_id.as_str(), "minecraft:desert");
    assert_eq!(hit.distance, found_distance((0, 64, 0), hit.pos, true));
    // Unknown biomes never match a real generator.
    assert!(search_biome(&noise, (0, 64, 0), &[id("minecraft:the_void")], -64, 320).is_none());
    // A generator without a noise biome misses everything.
    let none = |_: i32, _: i32, _: i32| None;
    assert!(search_biome(&none, (0, 64, 0), &[id("minecraft:plains")], -64, 320).is_none());
}

#[test]
fn sample_heights_start_at_origin_and_cover_bounds() {
    let heights = sample_heights(70, -63, 320);
    assert_eq!(heights[0], 70);
    // Vanilla `Mth.outFromOrigin(70, -63, 320, 64)`: origin, then alternating
    // +/- 64 steps until the next step leaves the bounds on both sides.
    assert_eq!(heights, vec![70, 134, 6, 198, -58, 262]);
    // Degenerate bounds stay empty, never panic.
    assert!(sample_heights(0, 10, 5).is_empty());
}

#[test]
fn poi_search_finds_nearest_loaded_state() {
    let mut blocks: HashMap<(i32, i32, i32), u32> = HashMap::new();
    blocks.insert((10, 64, 0), 7);
    blocks.insert((-300, 64, 0), 7);
    let candidates = || vec![(10, 0, 60, 70), (-300, 0, 60, 70)];
    let block_at = |x: i32, y: i32, z: i32| blocks.get(&(x, y, z)).copied();
    let is_poi = |state: u32| state == 7;
    let hit = search_poi(&candidates, &block_at, &is_poi, (0, 64, 0)).expect("bell is loaded");
    assert_eq!(hit.pos, (10, 64, 0));
    assert_eq!(hit.distance, 10);
    assert!(search_poi(&candidates, &block_at, &|_| false, (0, 64, 0)).is_none());
    assert!(search_poi(&|| Vec::new(), &block_at, &is_poi, (0, 64, 0)).is_none());
}

#[test]
fn poi_states_cover_vanilla_tables_and_head_only_beds() {
    let report = mc_data::blocks::krasnokamen_required_blocks_report();
    let blocks = BlockRegistry::from_report(&report).unwrap();
    let states = poi_states(&blocks, &[id("minecraft:meeting"), id("minecraft:home")]);
    assert!(!states.is_empty());
    let bell = blocks.block(&id("minecraft:bell")).expect("bell exists");
    assert!(bell.states.iter().all(|state| states.contains(&state.0)));
    // Every bed state in the set is a head part.
    for state in &states {
        let resolved = blocks.by_id(BlockStateId(*state)).unwrap();
        if resolved.block.id.as_str().ends_with("_bed") {
            assert!(
                resolved
                    .properties
                    .iter()
                    .any(|(name, value)| name == "part" && value == "head")
                    || !resolved.properties.iter().any(|(name, _)| name == "part"),
                "foot-part bed state leaked into POI states"
            );
        }
    }
    // Unknown members resolve to nothing, never panic.
    assert!(poi_states(&blocks, &[id("minecraft:not_a_poi")]).is_empty());
}

#[test]
fn structure_tag_resolution_covers_village_and_rejects_unknown() {
    assert_eq!(
        super::resolve_structure_tag(&id("minecraft:village"))
            .expect("village tag exists")
            .len(),
        5
    );
    assert!(super::resolve_structure_tag(&id("minecraft:no_such_tag")).is_none());
}

#[test]
fn poi_tag_resolution_expands_nested_tags() {
    let members = super::resolve_poi_tag(&id("minecraft:village")).expect("village POI tag exists");
    assert!(members.contains(&id("minecraft:home")));
    assert!(members.contains(&id("minecraft:meeting")));
    // Nested job-site members expand through `#acquirable_job_site`.
    assert!(members.contains(&id("minecraft:armorer")));
    assert!(super::resolve_poi_tag(&id("minecraft:no_such_tag")).is_none());
}

fn locate_fixture() -> (TickState, ConnectionId) {
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let generator = Arc::new(mc_worldgen::TerrainGenerator::new(
        42,
        Arc::clone(&registry.blocks),
    ));
    let world = crate::world::ChunkWorld::new(
        42,
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [0, 80, 0],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = TickState::new(registry, world, 1, 4096);
    crate::tick::run_tick(
        &mut state,
        &[crate::boundary::PlayerInput::Join {
            connection: ConnectionId(1),
            uuid: uuid::Uuid::new_v4(),
            name: "Alice".into(),
            permission_level: 2,
        }],
    );
    let slot = state
        .connections
        .entity_of(ConnectionId(1))
        .and_then(|entity| state.entities.slot_of(entity))
        .unwrap();
    state.entities.set_pos(slot, [0.0, 80.0, 0.0]);
    state.outbox.clear();
    (state, ConnectionId(1))
}

fn styled_key(state: &TickState, connection: ConnectionId, key: &str) -> bool {
    state.outbox.get(&connection).is_some_and(|queue| {
        queue.iter().any(|change| {
            matches!(change,
        PlayerChange::StyledChat { content:Tag::Compound(fields),.. }
        if fields.iter().any(|(name,value)|name=="translate" && value==&Tag::String(key.into())))
        })
    })
}

#[test]
fn invalid_structure_id_fails_without_queuing() {
    let (mut state, connection) = locate_fixture();
    let executor = state.connections.entity_of(connection).unwrap();
    let target = ResourceOrTag {
        tag: false,
        id: id("minecraft:not_a_structure"),
    };
    super::execute(
        &mut state,
        connection,
        executor,
        &LocateCommand::Structure(target),
    );
    assert!(styled_key(
        &state,
        connection,
        "commands.locate.structure.invalid"
    ));
    assert_eq!(state.locate.in_flight.get(&connection), None);
}

#[test]
fn locate_admits_only_selected_datapack_structure_ids() {
    let (mut state, connection) = locate_fixture();
    Arc::make_mut(&mut state.registry).datapack_structures =
        Arc::from([id("example:village_tower")]);
    let executor = state.connections.entity_of(connection).unwrap();
    let target = |name| {
        LocateCommand::Structure(ResourceOrTag {
            tag: false,
            id: id(name),
        })
    };
    super::execute(
        &mut state,
        connection,
        executor,
        &target("other:village_tower"),
    );
    assert!(styled_key(
        &state,
        connection,
        "commands.locate.structure.invalid"
    ));
    assert_eq!(state.locate.in_flight.get(&connection), None);
    state.outbox.clear();
    super::execute(
        &mut state,
        connection,
        executor,
        &target("example:village_tower"),
    );
    assert!(styled_key(
        &state,
        connection,
        "commands.locate.structure.searching"
    ));
    assert_eq!(state.locate.in_flight.get(&connection), Some(&1));
}

#[test]
fn unknown_biome_id_fails_without_queuing() {
    let (mut state, connection) = locate_fixture();
    let executor = state.connections.entity_of(connection).unwrap();
    let target = ResourceOrTag {
        tag: false,
        id: id("minecraft:not_a_biome"),
    };
    super::execute(
        &mut state,
        connection,
        executor,
        &LocateCommand::Biome(target),
    );
    assert!(styled_key(
        &state,
        connection,
        "argument.resource_or_id.invalid"
    ));
    assert_eq!(state.locate.in_flight.get(&connection), None);
}

#[test]
fn unknown_poi_id_reports_not_found() {
    let (mut state, connection) = locate_fixture();
    let executor = state.connections.entity_of(connection).unwrap();
    let target = ResourceOrTag {
        tag: false,
        id: id("minecraft:not_a_poi"),
    };
    super::execute(
        &mut state,
        connection,
        executor,
        &LocateCommand::Poi(target),
    );
    assert!(styled_key(
        &state,
        connection,
        "commands.locate.poi.not_found"
    ));
}

/// Drain until the worker answered (the search runs on another thread).
fn drain_until_answered(state: &mut TickState, connection: ConnectionId) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while state.locate.in_flight.contains_key(&connection) {
        assert!(
            std::time::Instant::now() < deadline,
            "locate worker never answered"
        );
        super::drain(state);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn real_generator_biome_search_answers_searching_then_success() {
    // Hermetic: the plain (non-vanilla) router still answers `noise_biome`.
    let (mut state, connection) = locate_fixture();
    let executor = state.connections.entity_of(connection).unwrap();
    let command = parse("locate biome minecraft:plains", true).expect("parses");
    let mut changes = ChangeSet::default();
    crate::command_execution::execute(&mut state, connection, executor, &command, &mut changes);
    // The tick answers "searching" now; the verdict arrives through the drain.
    assert!(styled_key(
        &state,
        connection,
        "commands.locate.biome.searching"
    ));
    drain_until_answered(&mut state, connection);
    assert!(styled_key(
        &state,
        connection,
        "commands.locate.biome.success"
    ));
}

#[test]
fn locate_result_drain_delivers_success_and_releases_slot() {
    let (mut state, connection) = locate_fixture();
    let executor = state.connections.entity_of(connection).unwrap();
    let command = parse("locate biome minecraft:plains", true).expect("parses");
    let mut changes = ChangeSet::default();
    crate::command_execution::execute(&mut state, connection, executor, &command, &mut changes);
    drain_until_answered(&mut state, connection);
    assert!(styled_key(
        &state,
        connection,
        "commands.locate.biome.success"
    ));
    assert_eq!(state.locate.in_flight.get(&connection), None);
}

#[test]
fn real_terrain_biome_search_uses_noise_field() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    if !root
        .join("data/minecraft/worldgen/noise_settings/overworld.json")
        .is_file()
    {
        eprintln!("SKIP real terrain biome search: no vanilla sidecar");
        return;
    }
    let blocks = Arc::new(
        BlockRegistry::from_report(&mc_data::blocks::krasnokamen_required_blocks_report()).unwrap(),
    );
    let mut terrain = mc_worldgen::TerrainGenerator::new(0, blocks);
    terrain.load_vanilla_worldgen(&root).unwrap();
    let origin = (0, 64, 0);
    let biome = terrain.noise_biome(0, 64, 0);
    let hit = search_biome(
        &|x, y, z| Some(terrain.noise_biome(x, y, z)),
        origin,
        std::slice::from_ref(&biome),
        -64,
        320,
    )
    .expect("origin column carries its own noise biome");
    assert_eq!(hit.distance, 0);
    assert_eq!(hit.found_id, biome);
    // An exhaustive miss over the full vanilla radius measures the worst case.
    let start = std::time::Instant::now();
    let miss = search_biome(
        &|x, y, z| Some(terrain.noise_biome(x, y, z)),
        origin,
        &[id("minecraft:the_void")],
        -64,
        320,
    );
    let elapsed = start.elapsed();
    assert!(miss.is_none());
    eprintln!(
        "locate exhaustive biome miss took {} ms",
        elapsed.as_millis()
    );
}

#[test]
fn map_structure_search_obeys_radius_and_persisted_destinations() {
    let generator = fake_generator();
    let targets = [id("minecraft:village_plains")];
    let position = BlockPos {
        x: 56,
        y: 64,
        z: -24,
    };
    let saved = SavedStarts::from([(
        ChunkPos { x: 3, z: -2 },
        vec![mc_world::chunk::GeneratedStructureSite {
            structure: targets[0].clone(),
            position,
            start_chunk: (3, -2),
        }],
    )]);
    let mut options = StructureSearchOptions {
        radius_rings: 2,
        skip_referenced: true,
        referenced: std::collections::HashSet::new(),
    };
    assert!(
        search_structure(&generator, (0, 64, 0), &targets, &saved, &options, None)
            .unwrap()
            .is_none()
    );
    options.radius_rings = 3;
    assert_eq!(
        search_structure(&generator, (0, 64, 0), &targets, &saved, &options, None)
            .unwrap()
            .unwrap()
            .pos,
        (56, 64, -24)
    );
    options.referenced.insert(position);
    assert!(
        search_structure(&generator, (0, 64, 0), &targets, &saved, &options, None)
            .unwrap()
            .is_none()
    );
    options.skip_referenced = false;
    assert!(
        search_structure(&generator, (0, 64, 0), &targets, &saved, &options, None)
            .unwrap()
            .is_some()
    );
}

#[test]
fn dolphin_located_tag_is_ocean_ruins_and_shipwrecks_only() {
    let tag = id("minecraft:dolphin_located");
    let members = super::resolve_structure_tag(&tag).expect("tag resolves");
    for expected in [
        "minecraft:ocean_ruin_cold",
        "minecraft:ocean_ruin_warm",
        "minecraft:shipwreck",
        "minecraft:shipwreck_beached",
    ] {
        assert!(
            members.contains(&id(expected)),
            "dolphin_located must contain {expected}"
        );
    }
    for rejected in ["minecraft:monument", "minecraft:ocean_monument"] {
        assert!(
            !members.contains(&id(rejected)),
            "vanilla dolphin_located excludes {rejected}"
        );
    }
}

#[test]
fn dolphin_requests_reserve_command_queue_capacity_before_the_snapshot() {
    let (mut state, connection) = locate_fixture();
    let (jobs_tx, _jobs_rx) = std::sync::mpsc::sync_channel(super::MAX_QUEUED_JOBS);
    let (_done_tx, done_rx) = std::sync::mpsc::channel();
    state.locate.jobs = Some(jobs_tx);
    state.locate.done = Some(done_rx);
    let dolphin = crate::boundary::EntityId(4242);
    state.locate.queued = super::MAX_QUEUED_JOBS - super::COMMAND_QUEUE_RESERVE;
    assert!(!super::request_dolphin_treasure(
        &mut state,
        dolphin,
        [8.5, 200.0, 8.5]
    ));
    assert!(state.locate.dolphin.get(&dolphin).is_none());
    assert_eq!(
        state.locate.queued,
        super::MAX_QUEUED_JOBS - super::COMMAND_QUEUE_RESERVE,
        "a refused entity request must not build or queue a snapshot"
    );
    // The reserved capacity is still available to an interactive command.
    assert!(
        super::submit(
            &mut state,
            super::LocateOwner::Connection(connection),
            super::LocateKind::Structure,
            (0, 64, 0),
            vec![id("minecraft:shipwreck")],
            super::LocateCompletion::Command("shipwreck".into()),
            super::StructureSearchOptions {
                radius_rings: 2,
                skip_referenced: false,
                referenced: Default::default(),
            },
        )
        .is_some()
    );
    assert_eq!(state.locate.queued, super::MAX_QUEUED_JOBS - 1);
}

#[test]
fn a_late_completion_does_not_replace_a_newer_dolphin_run() {
    let (mut state, _) = locate_fixture();
    let (jobs_tx, _jobs_rx) = std::sync::mpsc::sync_channel(super::MAX_QUEUED_JOBS);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    state.locate.jobs = Some(jobs_tx);
    state.locate.done = Some(done_rx);
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:dolphin").unwrap())
        .unwrap();
    let dolphin = state.spawn_mob(type_id, [8.5, 80.0, 8.5], 10.0);
    let slot = state.entities.slot_of(dolphin).unwrap();
    state.entities.row_mut(slot).entity_life.dolphin =
        Some(crate::dolphin::DolphinState { got_fish: true });
    let dimension = state.dimension;
    state.locate.next_job = 3;
    state.locate.queued = 1;
    state.locate.entity_in_flight.insert(dolphin, 1);
    state.locate.dolphin.insert(
        dolphin,
        super::DolphinTreasure::Pending { run: 2, dimension },
    );
    state.locate.pending.insert(
        1,
        super::PendingLocate {
            owner: super::LocateOwner::Entity(dolphin),
            kind: super::LocateKind::Structure,
            origin: (0, 64, 0),
            dimension,
            completion: super::LocateCompletion::Dolphin(dolphin),
        },
    );
    done_tx
        .send(super::LocateDone {
            id: 1,
            hit: Some(super::LocateHit {
                pos: (5, 60, 5),
                found_id: id("minecraft:shipwreck"),
                distance: 1,
            }),
            error: None,
            elapsed_ms: 0,
        })
        .unwrap();
    super::drain(&mut state);
    assert_eq!(
        state.locate.dolphin.get(&dolphin),
        Some(&super::DolphinTreasure::Pending { run: 2, dimension }),
        "run 1 must not satisfy run 2"
    );
    assert_eq!(state.locate.queued, 0);
    assert!(state.locate.entity_in_flight.is_empty());
}

#[test]
fn worker_loss_releases_a_map_loot_callback_so_the_container_opens_again() {
    // The pending loot plan lives in `maps.pending_loot` until the callback's
    // `completed` removes it. Dropping the continuation left it registered
    // forever: every later `before_open` returned waiting=true and the container
    // never opened again. The failure continuation must release it.
    let (mut state, connection) = locate_fixture();
    let (jobs_tx, _jobs_rx) = std::sync::mpsc::sync_channel(super::MAX_QUEUED_JOBS);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    state.locate.jobs = Some(jobs_tx);
    state.locate.done = Some(done_rx);
    let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reply_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let release_flag = Arc::clone(&released);
    let reply_flag = Arc::clone(&reply_called);
    state.locate.queued = 1;
    state.locate.pending.insert(
        1,
        super::PendingLocate {
            owner: super::LocateOwner::Connection(connection),
            kind: super::LocateKind::Structure,
            origin: (0, 64, 0),
            dimension: state.dimension,
            completion: super::LocateCompletion::Callback {
                reply: Box::new(move |_, _| {
                    reply_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                }),
                on_failure: Some(Box::new(move |_| {
                    release_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                })),
            },
        },
    );
    drop(done_tx);
    super::drain(&mut state);
    assert!(
        released.load(std::sync::atomic::Ordering::SeqCst),
        "worker loss must release the pending loot/trade plan"
    );
    assert!(
        !reply_called.load(std::sync::atomic::Ordering::SeqCst),
        "the success reply must not consume an unopened container"
    );
    assert!(state.locate.pending.is_empty());
    assert!(state.locate.jobs.is_none() && state.locate.done.is_none());
}

#[test]
fn a_lost_worker_terminates_dolphin_and_command_jobs() {
    let (mut state, connection) = locate_fixture();
    let (jobs_tx, _jobs_rx) = std::sync::mpsc::sync_channel(super::MAX_QUEUED_JOBS);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    state.locate.jobs = Some(jobs_tx);
    state.locate.done = Some(done_rx);
    let dolphin = crate::boundary::EntityId(88);
    let dimension = state.dimension;
    state.locate.next_job = 3;
    state.locate.queued = 2;
    state.locate.entity_in_flight.insert(dolphin, 1);
    state.locate.in_flight.insert(connection, 1);
    state.locate.dolphin.insert(
        dolphin,
        super::DolphinTreasure::Pending { run: 1, dimension },
    );
    state.locate.pending.insert(
        1,
        super::PendingLocate {
            owner: super::LocateOwner::Entity(dolphin),
            kind: super::LocateKind::Structure,
            origin: (0, 64, 0),
            dimension,
            completion: super::LocateCompletion::Dolphin(dolphin),
        },
    );
    state.locate.pending.insert(
        2,
        super::PendingLocate {
            owner: super::LocateOwner::Connection(connection),
            kind: super::LocateKind::Structure,
            origin: (0, 64, 0),
            dimension,
            completion: super::LocateCompletion::Command("shipwreck".into()),
        },
    );
    drop(done_tx);
    super::drain(&mut state);
    assert!(
        matches!(
            state.locate.dolphin.get(&dolphin),
            Some(super::DolphinTreasure::Failed { run: 1, dimension: failed_dimension, retry_at })
                if *failed_dimension == dimension && *retry_at > state.tick
        ),
        "worker loss is Failed, not Missing: {:?}",
        state.locate.dolphin.get(&dolphin)
    );
    assert!(state.outbox.get(&connection).is_some_and(|events| {
        events.iter().any(|event| {
            matches!(event, PlayerChange::Chat { message } if message.contains("unavailable"))
        })
    }));
    assert_eq!(state.locate.queued, 0);
}

#[test]
fn structure_lookup_callback_runs_on_drain_even_after_requester_disconnects() {
    let (mut state, _) = locate_fixture();
    let connection = ConnectionId(999);
    let (tx, rx) = std::sync::mpsc::channel();
    state.locate.done = Some(rx);
    state.locate.in_flight.insert(connection, 1);
    state.locate.queued = 1;
    let run = state.locate.next_job;
    state.locate.next_job = run + 1;
    let observed = Arc::new(parking_lot::Mutex::new(None));
    let output = Arc::clone(&observed);
    state.locate.pending.insert(
        run,
        super::PendingLocate {
            owner: super::LocateOwner::Connection(connection),
            kind: super::LocateKind::Structure,
            origin: (0, 64, 0),
            dimension: state.dimension,
            completion: super::LocateCompletion::Callback {
                reply: Box::new(move |state, result| {
                    assert_eq!(
                        state
                            .locate
                            .busy(super::LocateOwner::Connection(connection)),
                        0,
                        "release before callback can resubmit"
                    );
                    *output.lock() = Some(result);
                }),
                on_failure: None,
            },
        },
    );
    tx.send(super::LocateDone {
        id: run,
        hit: Some(super::LocateHit {
            pos: (117, -20, 130),
            found_id: id("minecraft:buried_treasure"),
            distance: 175,
        }),
        error: None,
        elapsed_ms: 0,
    })
    .ok()
    .unwrap();
    assert_eq!(*observed.lock(), None);
    super::drain(&mut state);
    assert_eq!(
        *observed.lock(),
        Some(Some(BlockPos {
            x: 117,
            y: -20,
            z: 130
        }))
    );
    assert!(state.outbox.is_empty());
    assert_eq!(state.locate.queued, 0);
    assert!(state.locate.pending.is_empty());
}

#[test]
fn structure_lookup_refuses_unknown_tags_radius_and_busy_connections_without_chat() {
    let (mut state, connection) = locate_fixture();
    let request = |destination, search_radius_rings| super::StructureLookupRequest {
        connection,
        origin: (0, 64, 0),
        destination: id(destination),
        search_radius_rings,
        skip_existing_chunks: true,
        referenced: Vec::new(),
        reply: Box::new(|_, _| panic!("rejected request must not complete")),
        on_failure: None,
    };
    assert!(!super::submit_structure_lookup(
        &mut state,
        request("minecraft:not_a_tag", 50)
    ));
    assert!(!super::submit_structure_lookup(
        &mut state,
        request("minecraft:on_treasure_maps", 257)
    ));
    state
        .locate
        .in_flight
        .insert(connection, super::MAX_JOBS_PER_PLAYER);
    assert!(!super::submit_structure_lookup(
        &mut state,
        request("minecraft:on_treasure_maps", 50)
    ));
    assert!(state.outbox.is_empty());
    assert!(state.locate.jobs.is_none());
}
