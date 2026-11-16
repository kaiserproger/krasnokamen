//! Native coverage cannot prove the identity of a lost immutable baseline.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Gap {
    _directory: tempfile::TempDir,
    store: NativeStore,
    blocks: BlockRegistry,
    items: mc_data::items::ItemRegistry,
    base: Chunk,
    effective: Chunk,
}

fn gap() -> Gap {
    let directory = tempfile::tempdir().unwrap();
    let store = NativeStore::create(&directory.path().join("state.redb"), [1; 32]).unwrap();
    let (blocks, items) = registries();
    commit(&store, 0, 0, &globals(0), &[], &[]).unwrap();
    let air = blocks
        .by_name_and_props(&mc_data::Identifier::parse("minecraft:air").unwrap(), &[])
        .unwrap();
    let stone = blocks
        .by_name_and_props(&mc_data::Identifier::parse("minecraft:stone").unwrap(), &[])
        .unwrap();
    let base = Chunk::empty(
        ChunkPos { x: 7, z: -9 },
        air,
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
    );
    store.import_chunk(0, &base, &blocks, &items).unwrap();
    let mut effective = base.clone();
    effective.set_block(0, 100, 0, stone).unwrap();
    effective.set_world_journal_lsn(1);
    store
        .apply_snapshots(
            &[NativeChunkSnapshot {
                dimension: 0,
                base: &base,
                effective: &effective,
                dirty_sections: None,
            }],
            &blocks,
            &items,
        )
        .unwrap();
    // Simulate loss of just the baseline, retaining real native overlays and
    // their applied watermark. No second store handle or mocked backend.
    let transaction = store.database.begin_write().unwrap();
    {
        let key = chunk_key(0, base.pos);
        let mut imported = transaction.open_table(IMPORTED).unwrap();
        assert!(imported.remove(key.as_slice()).unwrap().is_some());
    }
    transaction.commit().unwrap();
    let counts = store.record_counts().unwrap();
    assert_eq!(counts.imported_chunks, 0);
    assert_eq!(counts.applied_chunks, 1);
    assert!(counts.overlay_sections > 0);
    Gap {
        _directory: directory,
        store,
        blocks,
        items,
        base,
        effective,
    }
}

#[test]
fn native_covered_missing_baseline_refuses_before_current_generation() {
    let fixture = gap();
    let before = fixture.store.record_counts().unwrap();
    let result = fixture.store.read_chunk(
        0,
        fixture.base.pos,
        &fixture.blocks,
        &fixture.items,
        &mc_data::loot::chest_26_1_2::ChestLootCatalog::new(),
    );
    assert!(
        result.is_err(),
        "covered missing baseline must refuse native read"
    );
    assert_eq!(fixture.store.record_counts().unwrap(), before);
}

#[test]
fn native_covered_missing_baseline_refuses_resolver_before_procedural_fallback() {
    let fixture = gap();
    let calls = AtomicUsize::new(0);
    let result = fixture.store.resolve_base(
        0,
        fixture.base.pos,
        &fixture.blocks,
        &fixture.items,
        &mc_data::loot::chest_26_1_2::ChestLootCatalog::new(),
        || {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(Some(fixture.base.clone()))
        },
    );
    assert!(
        result.is_err(),
        "covered missing baseline reached procedural resolver"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn native_covered_missing_baseline_refuses_generated_fill_without_partial_insert() {
    let fixture = gap();
    let before = fixture.store.record_counts().unwrap();
    let mut new_chunk = fixture.base.clone();
    new_chunk.pos = ChunkPos { x: 8, z: -9 };
    assert!(
        fixture
            .store
            .fill_generated(
                0,
                &[new_chunk, fixture.base.clone()],
                &fixture.blocks,
                &fixture.items
            )
            .is_err(),
        "metadata/overlay coverage must not silently skip a lost dependent baseline"
    );
    assert_eq!(fixture.store.record_counts().unwrap(), before);
}

#[test]
fn native_covered_missing_baseline_refuses_checkpoint_rebaseline() {
    let fixture = gap();
    let before = fixture.store.record_counts().unwrap();
    let mut next = fixture.effective.clone();
    next.set_world_journal_lsn(2);
    let records = globals(2);
    assert!(
        fixture
            .store
            .commit_checkpoint(
                NativeCheckpoint {
                    tick: 2,
                    previous_tick: 0,
                    chunks: &[NativeChunkSnapshot {
                        dimension: 0,
                        base: &fixture.base,
                        effective: &next,
                        dirty_sections: None
                    }],
                    entities: &[],
                    players: &[],
                    globals: &records,
                },
                &fixture.blocks,
                &fixture.items
            )
            .is_err(),
        "checkpoint must not bless an unproven regenerated baseline"
    );
    assert_eq!(fixture.store.read_checkpoint().unwrap().tick, 0);
    assert_eq!(fixture.store.record_counts().unwrap(), before);
}

#[test]
fn native_covered_missing_baseline_refuses_replay_rebaseline() {
    let fixture = gap();
    let before = fixture.store.record_counts().unwrap();
    let mut next = fixture.effective.clone();
    next.set_world_journal_lsn(2);
    assert!(
        fixture
            .store
            .apply_snapshots(
                &[NativeChunkSnapshot {
                    dimension: 0,
                    base: &fixture.base,
                    effective: &next,
                    dirty_sections: None
                }],
                &fixture.blocks,
                &fixture.items,
            )
            .is_err()
    );
    assert_eq!(fixture.store.record_counts().unwrap(), before);
    assert!(
        fixture
            .store
            .generator_owns_baseline(0, fixture.base.pos)
            .is_err()
    );
}

#[test]
fn first_replay_snapshot_atomically_keeps_original_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.redb");
    let store = NativeStore::create(&path, [1; 32]).unwrap();
    let (blocks, items) = registries();
    commit(&store, 0, 0, &globals(0), &[], &[]).unwrap();
    let state = |name| {
        blocks
            .by_name_and_props(&mc_data::Identifier::parse(name).unwrap(), &[])
            .unwrap()
    };
    let base = Chunk::empty(
        ChunkPos { x: 8, z: -9 },
        state("minecraft:air"),
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
    );
    let mut effective = base.clone();
    effective
        .set_block(0, 100, 0, state("minecraft:stone"))
        .unwrap();
    effective.set_world_journal_lsn(1);
    store
        .apply_snapshots(
            &[NativeChunkSnapshot {
                dimension: 0,
                base: &base,
                effective: &effective,
                dirty_sections: None,
            }],
            &blocks,
            &items,
        )
        .unwrap();
    drop(store);
    let store = NativeStore::open(&path, [1; 32]).unwrap();
    let loot = mc_data::loot::chest_26_1_2::ChestLootCatalog::new();
    let original = store
        .resolve_base(0, base.pos, &blocks, &items, &loot, || {
            panic!("first snapshot must persist its original base, not require regeneration")
        })
        .unwrap()
        .unwrap();
    assert_eq!(original.get_block(0, 100, 0), base.get_block(0, 100, 0));
    let recovered = store
        .read_chunk(0, base.pos, &blocks, &items, &loot)
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered.get_block(0, 100, 0),
        effective.get_block(0, 100, 0)
    );
    assert_eq!(store.record_counts().unwrap().imported_chunks, 1);
}
