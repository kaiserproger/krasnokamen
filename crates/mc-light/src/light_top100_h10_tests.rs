//! H10 fences and an isolated M5-shaped diagnostic, not an entity-capacity gate.
use super::*;
use mc_world::ChunkGeometry;
use std::time::Instant;

const AIR: BlockStateId = BlockStateId(0);
const STONE: BlockStateId = BlockStateId(1);
const LAMP: BlockStateId = BlockStateId(2);

type Chunks = FxHashMap<ChunkPos, Arc<Chunk>>;

fn engine() -> LightEngine {
    LightEngine::new(Arc::new(BlockLightTable::from_arrays(
        "h10",
        vec![0, 0, 15],
        vec![0, 15, 0],
        vec![true, false, true],
    )))
}

fn roofed(pos: ChunkPos, geometry: ChunkGeometry) -> Chunk {
    let mut chunk = Chunk::empty_with_geometry(
        pos,
        AIR,
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
        geometry,
    );
    for z in 0..16 {
        for x in 0..16 {
            chunk.set_block(x, 0, z, STONE);
        }
    }
    chunk
}

fn insert(engine: &mut LightEngine, chunks: &mut Chunks, mut chunk: Chunk) {
    let light = compute_chunk_light_in(
        &mut engine.workspace,
        [[None; 3], [None, Some(&chunk), None], [None; 3]],
        &engine.table,
    );
    chunk.set_baked_light(&light);
    chunks.insert(chunk.pos, Arc::new(chunk));
}

fn publish(chunks: &mut Chunks, updates: Vec<(ChunkPos, ChunkLight)>) {
    for (pos, light) in updates {
        Arc::make_mut(chunks.get_mut(&pos).unwrap()).set_baked_light(&light);
    }
}

fn assert_oracle(engine: &mut LightEngine, chunks: &Chunks) {
    for (&pos, chunk) in chunks {
        let expected = compute_chunk_light_in(
            &mut engine.workspace,
            neighbours(chunks, pos),
            &engine.table,
        );
        let actual = engine.cache.get(pos).unwrap();
        // Check individual cells to keep a failed mutant's receipt readable.
        for y in chunk.geometry().min_y()..chunk.geometry().max_y() {
            for z in 0..16 {
                for x in 0..16 {
                    assert_eq!(
                        (actual.sky_at(x, y, z), actual.block_at(x, y, z)),
                        (expected.sky_at(x, y, z), expected.block_at(x, y, z)),
                        "full oracle at {pos:?}, ({x}, {y}, {z})"
                    );
                }
            }
        }
        assert_eq!(ChunkLight::from_chunk(chunk).as_ref(), Some(actual));
    }
}

fn edit(engine: &mut LightEngine, chunks: &mut Chunks, pos: (i32, i32, i32), next: BlockStateId) {
    let chunk = Arc::make_mut(
        chunks
            .get_mut(&ChunkPos {
                x: pos.0.div_euclid(16),
                z: pos.2.div_euclid(16),
            })
            .unwrap(),
    );
    let x = pos.0.rem_euclid(16) as u8;
    let z = pos.2.rem_euclid(16) as u8;
    let previous = chunk.get_block(x, pos.1, z).unwrap();
    chunk.set_block(x, pos.1, z, next);
    let updates = engine.edit(chunks, pos, previous, next);
    publish(chunks, updates);
}

fn fixture() -> (LightEngine, Chunks) {
    let mut engine = engine();
    let mut chunks = Chunks::default();
    let geometry = ChunkGeometry::new(-16, 32).unwrap();
    for z in -1..=1 {
        for x in -1..=1 {
            insert(
                &mut engine,
                &mut chunks,
                roofed(ChunkPos { x, z }, geometry),
            );
        }
    }
    let mut loaded: Vec<_> = chunks.keys().copied().collect();
    loaded.sort_unstable_by_key(|pos| (pos.x, pos.z));
    let updates = engine.admit(&chunks, &loaded);
    publish(&mut chunks, updates);
    (engine, chunks)
}

#[test]
fn h10_ceiling_open_close_and_edit_bursts_match_full_oracle() {
    let (mut engine, mut chunks) = fixture();
    assert_oracle(&mut engine, &chunks);
    // The actual edit owner consumes post-images serially, including edits in
    // one tick. Compare the whole batch, not only the edited chunk or cell.
    let holes = [(15, 0, 15), (16, 0, 15), (15, 0, 16), (16, 0, 16)];
    for pos in holes {
        edit(&mut engine, &mut chunks, pos, AIR);
    }
    assert_eq!(
        engine
            .cache
            .get(ChunkPos { x: 0, z: 0 })
            .unwrap()
            .sky_at(15, -1, 15),
        15
    );
    assert_oracle(&mut engine, &chunks);
    for pos in holes.into_iter().rev() {
        edit(&mut engine, &mut chunks, pos, STONE);
    }
    assert_oracle(&mut engine, &chunks);
    for pos in [(15, -4, 8), (16, -4, 8), (-1, -4, 0), (0, -4, -1)] {
        edit(&mut engine, &mut chunks, pos, LAMP);
    }
    assert_oracle(&mut engine, &chunks);
    for pos in [(16, -4, 8), (15, -4, 8), (0, -4, -1), (-1, -4, 0)] {
        edit(&mut engine, &mut chunks, pos, AIR);
    }
    assert_oracle(&mut engine, &chunks);
}

#[test]
fn h10_simultaneous_mixed_edits_are_order_independent() {
    let edits = [
        ((15, 0, 15), AIR),
        ((16, 0, 16), AIR),
        ((15, -4, 16), LAMP),
        ((16, -4, 15), LAMP),
    ];
    let mut reference = None;
    for order in [[0, 1, 2, 3], [3, 2, 1, 0], [1, 3, 0, 2], [2, 0, 3, 1]] {
        let (mut engine, mut chunks) = fixture();
        for index in order {
            let (pos, next) = edits[index];
            edit(&mut engine, &mut chunks, pos, next);
        }
        assert_oracle(&mut engine, &chunks);
        let actual: FxHashMap<_, _> = chunks
            .keys()
            .map(|pos| (*pos, engine.cache.get(*pos).unwrap().clone()))
            .collect();
        if let Some(expected) = &reference {
            assert_eq!(&actual, expected, "mixed edit order {order:?}");
        } else {
            reference = Some(actual);
        }
        for index in order.into_iter().rev() {
            let (pos, _) = edits[index];
            edit(
                &mut engine,
                &mut chunks,
                pos,
                if pos.1 == 0 { STONE } else { AIR },
            );
        }
        assert_oracle(&mut engine, &chunks);
    }
}

#[test]
fn h10_missing_diagonal_admission_and_seam_removal_match_full_oracle() {
    let geometry = ChunkGeometry::new(-16, 32).unwrap();
    let centre = ChunkPos { x: -2, z: -2 };
    let east = ChunkPos { x: -1, z: -2 };
    let diagonal = ChunkPos { x: -1, z: -1 };
    let mut engine = engine();
    let mut chunks = Chunks::default();
    insert(&mut engine, &mut chunks, roofed(centre, geometry));
    let updates = engine.admit(&chunks, &[centre]);
    publish(&mut chunks, updates);
    assert_oracle(&mut engine, &chunks);
    let mut source = roofed(diagonal, geometry);
    source.set_block(0, -4, 0, LAMP);
    insert(&mut engine, &mut chunks, source);
    let updates = engine.admit(&chunks, &[diagonal]);
    publish(&mut chunks, updates);
    assert_oracle(&mut engine, &chunks);
    assert_eq!(engine.cache.get(centre).unwrap().block_at(15, -4, 15), 13);
    // An opaque neighbour replaces virtual open air. Its seam removal wave
    // must reach the resident interior, while the other corner stays absent.
    let opaque = Chunk::empty_with_geometry(
        east,
        STONE,
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
        geometry,
    );
    insert(&mut engine, &mut chunks, opaque);
    let updates = engine.admit(&chunks, &[east]);
    publish(&mut chunks, updates);
    assert_oracle(&mut engine, &chunks);
    chunks.remove(&diagonal);
    engine.remove(diagonal);
    let updates = engine.admit(&chunks, &[east]);
    publish(&mut chunks, updates);
    assert_oracle(&mut engine, &chunks);
    assert_eq!(engine.cache.get(centre).unwrap().block_at(15, -4, 15), 0);
}

// Linux's per-thread scheduler clock avoids attributing Cargo, other tests or
// the run-queue wait to the light solver. Wall times still reflect host load.
fn thread_cpu_ns() -> u64 {
    std::fs::read_to_string("/proc/thread-self/schedstat")
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
#[ignore = "M5-shaped diagnostic, not capacity evidence; run the built executable under mem-run"]
fn h10_edit_burst_diagnostic() {
    let (mut engine, mut chunks) = fixture();
    let mut samples = Vec::new();
    let mut cpu_start = 0;
    // Four holes straddle a diagonal seam. Each sample opens/closes them and
    // inserts/removes four emitters: equal geometry and 16 useful edits.
    for sample in 0..220 {
        if sample == 20 {
            cpu_start = thread_cpu_ns();
        }
        let start = Instant::now();
        for state in [AIR, STONE] {
            for pos in [(15, 0, 15), (16, 0, 15), (15, 0, 16), (16, 0, 16)] {
                edit(&mut engine, &mut chunks, pos, state);
            }
        }
        for state in [LAMP, AIR] {
            for pos in [(15, -4, 8), (16, -4, 8), (-1, -4, 0), (0, -4, -1)] {
                edit(&mut engine, &mut chunks, pos, state);
            }
        }
        if sample >= 20 {
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let cpu_ms = (thread_cpu_ns() - cpu_start) as f64 / 1_000_000.0;
    assert_oracle(&mut engine, &chunks);
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "H10 diagnostic chunks=9 height=32 background_entities=0 warmup_batches=20 measured_batches=200 edits_per_batch=16 cpu_ms={:.6} p50_ms={:.6} p95_ms={:.6} p99_ms={:.6} max_ms={:.6}",
        cpu_ms, samples[100], samples[190], samples[198], samples[199]
    );
}
