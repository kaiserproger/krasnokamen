use std::sync::Arc;

use mc_world::{BlockStateId, Chunk, ChunkGenerator, ChunkPos};
use rustc_hash::FxHashMap;

use crate::registry::CoreRegistry;
use crate::world::ChunkWorld;

struct EmptyWorld {
    air: u32,
    stone: u32,
    roof: bool,
}

impl ChunkGenerator for EmptyWorld {
    fn generate(&self, pos: ChunkPos) -> Chunk {
        let mut chunk = Chunk::empty(
            pos,
            BlockStateId(self.air),
            mc_data::Identifier::parse("minecraft:plains").unwrap(),
        );
        if self.roof && pos == (ChunkPos { x: 0, z: 0 }) {
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set_block(x, 101, z, BlockStateId(self.stone));
                }
            }
        }
        chunk
    }
}

#[cfg(test)]
fn light_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla")
}

fn world(dir: Option<std::path::PathBuf>, roof: bool) -> (ChunkWorld, Arc<CoreRegistry>) {
    let registry = Arc::new(CoreRegistry::load(Some(&light_root())).unwrap());
    let generator: Arc<dyn ChunkGenerator> = Arc::new(EmptyWorld {
        air: registry.states.air,
        stone: registry.states.stone,
        roof,
    });
    let air = BlockStateId(registry.states.air);
    let world = if let Some(dir) = dir {
        let config = crate::CoreConfig::new(dir, 42);
        ChunkWorld::new_native(
            42,
            crate::native_checkpoint::tests::fixture_store(&config).unwrap(),
            crate::native_checkpoint::dimension_id(crate::dimensions::Dimension::Overworld),
            generator,
            &registry,
            air,
            [0, 100, 0],
            mc_world::OVERWORLD_GEOMETRY,
        )
    } else {
        ChunkWorld::new(
            42,
            generator,
            &registry,
            air,
            [0, 100, 0],
            mc_world::OVERWORLD_GEOMETRY,
        )
    };
    (world, registry)
}

fn want(positions: &[ChunkPos]) -> FxHashMap<ChunkPos, i32> {
    positions.iter().map(|pos| (*pos, 0)).collect()
}

const CENTRE: ChunkPos = ChunkPos { x: 0, z: 0 };
const EAST: ChunkPos = ChunkPos { x: 1, z: 0 };

#[test]
fn physics_slab_bottom_blocks_direct_skylight_below_it() {
    let (mut world, registry) = world(None, true);
    world.ensure_blocking(&want(&[CENTRE]));
    world.set_block(8, 101, 8, registry.states.air);
    world.fold_overlay();
    assert_eq!(world.light_at(8, 100, 8).unwrap().0, 15);
    let slab = registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:stone_slab").unwrap())
        .unwrap()
        .default
        .0;
    world.set_block(8, 101, 8, slab);
    world.fold_overlay();
    assert!(world.light_at(8, 100, 8).unwrap().0 < 15);
    let chunk = world.chunk(CENTRE).unwrap();
    let baked = mc_world::light::compute_chunk_light(
        [[None; 3], [None, Some(chunk), None], [None; 3]],
        &registry.light,
    );
    assert!(baked.sky_at(8, 100, 8) < 15);
    world.set_block(8, 101, 8, registry.states.air);
    world.fold_overlay();
    assert_eq!(world.light_at(8, 100, 8).unwrap().0, 15);
}

#[test]
fn physics_complementary_slab_faces_block_light_and_shape_edits_relight() {
    let (mut world, registry) = world(None, false);
    world.ensure_blocking(&want(&[CENTRE]));
    let slab = registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:stone_slab").unwrap())
        .unwrap();
    let bottom = slab.default.0;
    let mut props = registry
        .blocks
        .by_id(slab.default)
        .unwrap()
        .properties
        .clone();
    props.iter_mut().find(|(key, _)| key == "type").unwrap().1 = "top".into();
    let top = registry
        .blocks
        .by_name_and_props(&slab.id, &props)
        .unwrap()
        .0;
    for x in 2..=10 {
        for y in 99..=101 {
            for z in 7..=9 {
                world.set_block(x, y, z, registry.states.stone);
            }
        }
    }
    for x in 3..=9 {
        world.set_block(x, 100, 8, registry.states.air);
    }
    world.set_block(3, 100, 8, registry.states.torch);
    world.set_block(5, 100, 8, bottom);
    world.set_block(6, 100, 8, top);
    world.fold_overlay();
    assert_eq!(world.light_at(7, 100, 8).unwrap().1, 0);
    world.set_block(6, 100, 8, bottom);
    world.fold_overlay();
    assert!(world.light_at(7, 100, 8).unwrap().1 > 0);
    world.set_block(6, 100, 8, top);
    world.fold_overlay();
    assert_eq!(world.light_at(7, 100, 8).unwrap().1, 0);
    let chunk = world.chunk(CENTRE).unwrap();
    let baked = mc_world::light::compute_chunk_light(
        [[None; 3], [None, Some(chunk), None], [None; 3]],
        &registry.light,
    );
    assert_eq!(
        baked.block_at(7, 100, 8),
        0,
        "fresh bake uses the same faces"
    );
}

#[test]
fn torch_place_remove_relights_across_chunk_boundary_without_full_recompute() {
    let (mut world, registry) = world(None, false);
    world.ensure_blocking(&want(&[CENTRE, EAST]));
    assert_eq!(world.light_at(8, i32::MIN, 8), None);
    assert_eq!(world.light_at(8, i32::MAX, 8), None);
    assert_eq!(world.light_at(8, mc_world::MAX_Y, 8), None);
    let before = world.snapshot();
    let full = world.light.full_recomputes;
    assert!(
        full > 0,
        "blocking startup uses its full oracle before incremental tick edits"
    );
    world.set_block(15, 100, 8, registry.states.torch);
    world.fold_overlay();
    assert_eq!(world.light_at(15, 100, 8), Some((15, 14)));
    assert_eq!(world.light_at(16, 100, 8), Some((15, 13)));
    assert_eq!(world.light_at(28, 100, 8), Some((15, 1)));
    assert_eq!(world.light_at(29, 100, 8), Some((15, 0)));
    assert_eq!(
        world.light.full_recomputes, full,
        "single edits never recompute a chunk"
    );
    let old_east = before.iter().find(|entry| entry.chunk.pos == EAST).unwrap();
    assert_eq!(
        mc_world::light::ChunkLight::from_chunk(&old_east.chunk)
            .unwrap()
            .block_at(0, 100, 8),
        0,
        "snapshot stays immutable"
    );
    assert_ne!(
        world.chunk_revision(1, 0),
        old_east.revision,
        "neighbor light-only update bumps revision"
    );
    for old in before.iter() {
        let current = world.chunk(old.pos).unwrap();
        let changed_sections = old
            .chunk
            .section_lights
            .iter()
            .zip(&current.section_lights)
            .filter(|(previous, next)| previous != next)
            .count();
        assert!(
            changed_sections <= 3,
            "torch affects only its bounded vertical radius"
        );
    }
    world.set_block(15, 100, 8, registry.states.air);
    world.fold_overlay();
    for x in 2..=28 {
        assert_eq!(
            world.light_at(x, 100, 8).unwrap().1,
            0,
            "stale torch light at x={x}"
        );
    }
    assert_eq!(world.light.full_recomputes, full);
}

#[test]
fn removal_keeps_an_independent_neighbouring_emitter() {
    let (mut world, registry) = world(None, false);
    world.ensure_blocking(&want(&[CENTRE, EAST]));
    world.set_block(15, 100, 8, registry.states.torch);
    world.set_block(18, 100, 8, registry.states.torch);
    world.fold_overlay();
    world.set_block(15, 100, 8, registry.states.air);
    world.fold_overlay();
    assert_eq!(world.light_at(18, 100, 8).unwrap().1, 14);
    assert_eq!(world.light_at(15, 100, 8).unwrap().1, 11);
}

#[test]
fn sky_under_overhang_uses_sideways_light_and_reseeds_opened_column() {
    let (mut world, registry) = world(None, true);
    world.ensure_blocking(&want(&[CENTRE]));
    assert_eq!(world.light_at(8, 102, 8).unwrap().0, 15);
    let shaded = world.light_at(8, 100, 8).unwrap().0;
    assert!(
        shaded < 9 && shaded > 0,
        "overhang gets lateral sky light, got {shaded}"
    );
    assert_eq!(world.light_at(0, 100, 8).unwrap().0, 14);
    let full = world.light.full_recomputes;
    world.set_block(8, 101, 8, registry.states.air);
    world.fold_overlay();
    assert_eq!(world.light_at(8, 100, 8).unwrap().0, 15);
    assert_eq!(
        world.light_at(8, mc_world::MIN_Y, 8).unwrap().0,
        15,
        "opening skylight reseeds the full vertical column"
    );
    world.set_block(8, 101, 8, registry.states.stone);
    world.fold_overlay();
    assert_eq!(world.light_at(8, 100, 8).unwrap().0, shaded);
    assert_eq!(world.light.full_recomputes, full);
}

#[test]
fn late_neighbour_admission_recovers_boundary_light() {
    let (mut world, registry) = world(None, false);
    world.ensure_blocking(&want(&[CENTRE]));
    world.set_block(15, 100, 8, registry.states.torch);
    world.fold_overlay();
    assert_eq!(world.light_at(16, 100, 8), None);
    world.ensure_blocking(&want(&[CENTRE, EAST]));
    assert_eq!(world.light_at(16, 100, 8).unwrap().1, 13);
}

#[test]
fn checkpoint_persists_both_light_channels_and_journal_replay_removes_light() {
    let dir = tempfile::tempdir().unwrap();
    let (mut first, registry) = world(Some(dir.path().to_path_buf()), true);
    first.ensure_blocking(&want(&[CENTRE, EAST]));
    first.set_block(15, 100, 8, registry.states.torch);
    first.fold_overlay();
    let expected: Vec<_> = [CENTRE, EAST]
        .into_iter()
        .map(|pos| {
            (
                pos,
                mc_world::light::ChunkLight::from_chunk(first.chunk(pos).unwrap()).unwrap(),
            )
        })
        .collect();
    assert_eq!(first.flush_dirty(&Default::default()).unwrap(), 2);
    drop(first);
    let config = crate::CoreConfig::new(dir.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let dimension = crate::native_checkpoint::dimension_id(crate::dimensions::Dimension::Overworld);
    for (pos, light) in &expected {
        let base = store
            .resolve_base(
                dimension,
                *pos,
                &registry.blocks,
                &registry.items,
                &registry.chest_loot,
                || Ok(None),
            )
            .unwrap()
            .expect("checkpoint persists the immutable native base");
        let chunk = store
            .compose(
                dimension,
                &base,
                &registry.blocks,
                &registry.items,
                &registry.chest_loot,
            )
            .unwrap();
        assert_eq!(
            mc_world::light::ChunkLight::from_chunk(&chunk).as_ref(),
            Some(light),
            "actual native storage retains sky+block arrays"
        );
    }
    drop(store);
    let mut writer = crate::journal::JournalWriter::open_test(dir.path(), 1).unwrap();
    writer
        .append(
            1,
            1,
            &[crate::journal::JournalEntry::Block {
                pos: [15, 100, 8],
                state: registry.states.air,
            }],
        )
        .unwrap();
    let batches =
        crate::journal::read_complete_batches_after_test(&dir.path().join("journal"), 0).unwrap();
    let replay = crate::journal::replay_batches(Default::default(), &batches);
    let (mut reopened, _) = world(Some(dir.path().to_path_buf()), true);
    for ((x, y, z), state) in replay.blocks {
        reopened.set_block(x, y, z, state);
    }
    reopened.ensure_blocking(&want(&[CENTRE, EAST]));
    let full = reopened.light.full_recomputes;
    reopened.fold_overlay();
    assert_eq!(reopened.light_at(15, 100, 8).unwrap().1, 0);
    assert_eq!(reopened.light_at(16, 100, 8).unwrap().1, 0);
    assert!(
        reopened.light_at(8, 100, 8).unwrap().0 < 9,
        "persisted roof still shades sky"
    );
    assert_eq!(
        reopened.light.full_recomputes, full,
        "replayed edits use incremental lighting too"
    );
}

#[test]
fn sky_darkening_is_time_and_weather_dependent_but_does_not_change_block_light() {
    assert_eq!(mc_light::sky_darken(6_000, 0.0, 0.0), 0);
    assert_eq!(mc_light::sky_darken(18_000, 0.0, 0.0), 11);
    assert_eq!(mc_light::sky_darken(13_000, 0.0, 0.0), 6);
    assert_eq!(mc_light::sky_darken(13_670, 0.0, 0.0), 11);
    assert_eq!(mc_light::sky_darken(23_000, 0.0, 0.0), 6);
    assert!(mc_light::sky_darken(6_000, 1.0, 1.0) > mc_light::sky_darken(6_000, 1.0, 0.0));
    let (mut world, registry) = world(None, false);
    world.ensure_blocking(&want(&[CENTRE]));
    world.set_block(8, 100, 8, registry.states.torch);
    world.fold_overlay();
    let mut state = crate::tick::TickState::new(registry, world, 0, 64);
    state.day_time = 18_000;
    assert_eq!(state.brightness(8, 100, 8), 14);
    assert_eq!(state.brightness(0, 100, 0), 4);
    assert_eq!(
        state.world.light_at(0, 100, 0).unwrap().0,
        15,
        "night never changes baked skylight"
    );
}

#[test]
fn worker_preserves_complete_arrays_and_bakes_omitted_layers() {
    let registry = CoreRegistry::load(Some(&light_root())).unwrap();
    let mut chunk = Chunk::empty(
        CENTRE,
        BlockStateId(registry.states.air),
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
    );
    chunk.set_block(8, 0, 8, BlockStateId(registry.states.stone));
    chunk.set_baked_light(&mc_world::light::ChunkLight::filled(15, 15));
    mc_light::bake_missing(&mut chunk, &registry.light);
    assert_eq!(
        mc_world::light::ChunkLight::from_chunk(&chunk).unwrap(),
        mc_world::light::ChunkLight::filled(15, 15),
        "complete producer arrays are not rebaked"
    );
    chunk.section_lights[0].sky = None;
    mc_light::bake_missing(&mut chunk, &registry.light);
    let actual = mc_world::light::ChunkLight::from_chunk(&chunk).unwrap();
    let expected = mc_world::light::compute_chunk_light(
        [[None; 3], [None, Some(&chunk), None], [None; 3]],
        &registry.light,
    );
    assert_eq!(actual, expected);
    assert_eq!(actual.block_at(8, -1, 8), 0);
    assert_eq!(actual.sky_at(8, -1, 8), 14);
}
