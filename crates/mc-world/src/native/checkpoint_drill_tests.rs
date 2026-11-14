//! Real subprocess stops and redb backend ENOSPC; no graceful shutdown can commit.
use super::super::checkpoint_tests::{globals, registries};
use super::*;
use redb::StorageBackend;
use std::cell::Cell;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

struct NoGeneration;
impl crate::ChunkGenerator for NoGeneration {
    fn generate(&self, _: ChunkPos) -> Chunk {
        panic!("checkpoint baseline must win")
    }
}

thread_local! {
    static STOP_AT: Cell<Option<&'static str>> = const { Cell::new(None) };
}

pub(super) fn checkpoint_stage(stage: &str) {
    if STOP_AT.get().is_some_and(|stop| stop == stage) {
        std::process::exit(39);
    }
}

fn write_image(store: &NativeStore, tick: u64) -> Result<(), NativeError> {
    let (blocks, items) = registries();
    let air = blocks
        .block(&mc_data::Identifier::parse("minecraft:air").unwrap())
        .unwrap()
        .default;
    let stone = blocks
        .block(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap()
        .default;
    let base = Chunk::empty(
        ChunkPos { x: 2, z: -3 },
        air,
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
    );
    let mut effective = base.clone();
    effective
        .set_block(1, 64, 1, if tick == 0 { air } else { stone })
        .unwrap();
    let entity = NativeEntitySnapshot {
        identity: [2; 16],
        dimension: 0,
        position: base.pos,
        generated_slot: Some([3; 32]),
        journal_lsn: tick,
        payload: Some(Tag::Compound(vec![
            ("solaris:WireId".into(), Tag::Int(9)),
            ("Health".into(), Tag::Float(tick as f32)),
        ])),
    };
    let player = NativePlayerSnapshot {
        uuid: [6; 16],
        record: NativeDomainRecord {
            version: 1,
            captured_tick: tick,
            payload: codec::encode(&Tag::Compound(vec![(
                "Score".into(),
                Tag::Int(tick as i32),
            )]))
            .unwrap(),
        },
    };
    store.commit_checkpoint(
        NativeCheckpoint {
            tick,
            previous_tick: 0,
            chunks: &[NativeChunkSnapshot {
                dimension: 0,
                base: &base,
                effective: &effective,
                dirty_sections: None,
            }],
            entities: &[entity],
            players: &[player],
            globals: &globals(tick),
        },
        &blocks,
        &items,
    )
}

fn assert_image(store: &NativeStore, tick: u64) {
    let image = store.read_checkpoint().unwrap();
    assert_eq!(image.tick, tick);
    assert!(
        image
            .globals
            .values()
            .all(|record| record.captured_tick == tick)
    );
    assert_eq!(image.players.len(), 1);
    assert_eq!(image.players[0].record.captured_tick, tick);
    assert_eq!(
        codec::decode(&image.players[0].record.payload).unwrap(),
        Tag::Compound(vec![("Score".into(), Tag::Int(tick as i32))])
    );
    assert_eq!(image.entities.len(), 1);
    assert_eq!(image.entities[0].journal_lsn, tick);
    assert_eq!(
        image.entities[0].payload.as_ref().unwrap(),
        &Tag::Compound(vec![
            ("solaris:WireId".into(), Tag::Int(9)),
            ("Health".into(), Tag::Float(tick as f32))
        ])
    );
    assert_eq!(store.generated_slot([3; 32]).unwrap(), Some([2; 16]));
    let (blocks, items) = registries();
    let chunk = store
        .read_chunk(
            0,
            ChunkPos { x: 2, z: -3 },
            &blocks,
            &items,
            &ChestLootCatalog::new(),
            &NoGeneration,
        )
        .unwrap()
        .unwrap();
    let name = if tick == 0 {
        "minecraft:air"
    } else {
        "minecraft:stone"
    };
    assert_eq!(
        chunk.get_block(1, 64, 1),
        Some(
            blocks
                .block(&mc_data::Identifier::parse(name).unwrap())
                .unwrap()
                .default
        )
    );
}

#[test]
fn abrupt_stop_at_every_checkpoint_stage_never_mixes_domains() {
    const CHILD: &str = "SOLARIS_CHECKPOINT_DRILL_PATH";
    const STAGES: [&str; 5] = ["globals", "chunks", "entities", "players", "committed"];
    if let Some(path) = std::env::var_os(CHILD) {
        let stage = std::env::var("SOLARIS_CHECKPOINT_DRILL_STAGE").unwrap();
        let stage = STAGES.into_iter().find(|value| *value == stage).unwrap();
        let store = NativeStore::open(std::path::Path::new(&path), [1; 32]).unwrap();
        STOP_AT.set(Some(stage));
        write_image(&store, 7).unwrap();
        panic!("stop hook not reached");
    }
    for stage in STAGES {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.redb");
        let store = NativeStore::create(&path, [1; 32]).unwrap();
        write_image(&store, 0).unwrap();
        drop(store);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native::checkpoint::drill_tests::abrupt_stop_at_every_checkpoint_stage_never_mixes_domains", "--nocapture"])
            .env(CHILD, &path).env("SOLARIS_CHECKPOINT_DRILL_STAGE", stage).status().unwrap();
        assert_eq!(status.code(), Some(39), "stage {stage}");
        let store = NativeStore::open(&path, [1; 32]).unwrap();
        assert_image(&store, if stage == "committed" { 7 } else { 0 });
        drop(store);
        assert_image(
            &NativeStore::open(&path, [1; 32]).unwrap(),
            if stage == "committed" { 7 } else { 0 },
        );
    }
}

#[derive(Debug)]
struct FullDisk {
    file: redb::backends::FileBackend,
    writes_left: Arc<AtomicI64>,
}
impl StorageBackend for FullDisk {
    fn len(&self) -> std::io::Result<u64> {
        self.file.len()
    }
    fn read(&self, offset: u64, out: &mut [u8]) -> std::io::Result<()> {
        self.file.read(offset, out)
    }
    fn set_len(&self, len: u64) -> std::io::Result<()> {
        self.file.set_len(len)
    }
    fn sync_data(&self) -> std::io::Result<()> {
        self.file.sync_data()
    }
    fn write(&self, offset: u64, data: &[u8]) -> std::io::Result<()> {
        let left = self.writes_left.load(Ordering::SeqCst);
        if left == 0 {
            return Err(std::io::Error::from_raw_os_error(28));
        }
        if left > 0 {
            self.writes_left.fetch_sub(1, Ordering::SeqCst);
        }
        self.file.write(offset, data)
    }
    fn close(&self) -> std::io::Result<()> {
        self.file.close()
    }
}

#[test]
fn enospc_checkpoint_backend_refuses_and_recovers_last_durable_image() {
    // Sweep every physical write in this small transaction, including the commit
    // marker. A failure after the durable commit may expose the complete new image.
    let mut failures = 0;
    for cut in 0..128 {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.redb");
        let store = NativeStore::create(&path, [1; 32]).unwrap();
        write_image(&store, 0).unwrap();
        drop(store);
        let writes_left = Arc::new(AtomicI64::new(-1));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let database = redb::Database::builder()
            .set_cache_size(1024 * 1024)
            .create_with_backend(FullDisk {
                file: redb::backends::FileBackend::new(file).unwrap(),
                writes_left: writes_left.clone(),
            })
            .unwrap();
        let store = NativeStore {
            database,
            identity: [1; 32],
        };
        writes_left.store(cut, Ordering::SeqCst);
        let result = write_image(&store, 7);
        // Space returns before cleanup/reopen; the refused transaction must not be
        // acknowledged or retried by this owner, even if redb is poisoned.
        writes_left.store(-1, Ordering::SeqCst);
        let succeeded = result.is_ok();
        if let Err(error) = result {
            assert!(error.to_string().contains("No space left"), "{error}");
            failures += 1;
        }
        drop(store);
        let store = NativeStore::open(&path, [1; 32]).unwrap();
        let tick = store.read_checkpoint().unwrap().tick;
        assert!(tick == 0 || tick == 7, "cut {cut}: invalid fence {tick}");
        if succeeded {
            assert_eq!(tick, 7);
        }
        assert_image(&store, tick);
        if succeeded {
            assert!(failures > 0);
            return;
        }
    }
    panic!("write sweep never reached a successful commit");
}
