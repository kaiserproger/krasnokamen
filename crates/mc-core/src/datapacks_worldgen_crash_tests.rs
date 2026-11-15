//! Process SIGKILL at a durable real tick WAL, before checkpoint/shutdown.
use super::*;
use std::io::{BufRead, Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};

const CHILD: &str = "KRASNOKAMEN_DATAPACK_WAL_CRASH_CHILD";
const POSITION: &str = "KRASNOKAMEN_DATAPACK_WAL_CRASH_POSITION";
const READY: &str = "DATAPACK_WAL_DURABLE ";

fn config(root: &Path) -> crate::CoreConfig {
    let mut config = crate::CoreConfig::new(root.to_owned(), 81);
    config.vanilla_data_dir = Some(
        std::env::var_os("KRASNOKAMEN_CONTENT_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla")
            }),
    );
    config.view_distance = 0;
    config.simulation_distance = 0;
    config.checkpoint_every_ticks = u64::MAX;
    config.journal_fsync_every_batches = 1;
    config
}

fn village_tags() -> Vec<String> {
    ["plains", "desert", "savanna", "snowy", "taiga"]
        .map(|name| format!("minecraft/tags/worldgen/biome/has_structure/village_{name}.json"))
        .into()
}

fn command(core: &crate::Core, text: String) {
    let (input, completed) = crate::command_console::ConsoleCommand::new(text);
    assert!(core.submit(crate::boundary::PlayerInput::ConsoleCommand(input)));
    completed.blocking_recv().unwrap();
}

fn crash_child(root: &Path) {
    let (x, z) = std::env::var(POSITION)
        .unwrap()
        .split_once(',')
        .map(|(x, z)| (x.parse::<i32>().unwrap(), z.parse::<i32>().unwrap()))
        .unwrap();
    let pos = crate::boundary::ChunkPos { x, z };
    let target = [x * 16, 200, z * 16];
    let (observed, ready) = std::sync::mpsc::channel();
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = Some((
        root.to_owned(),
        Box::new(move |tick, entries| {
            if !entries.iter().any(|entry| matches!(entry, crate::journal::JournalEntry::Block { pos, .. } if *pos == target)) {
            return;
        }
            // The production observer boundary already flushed the real writer.
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "{READY}{tick}").unwrap();
            stdout.flush().unwrap();
            drop(stdout);
            observed.send(()).unwrap();
            // Parent SIGKILL is the only successful exit from this rendezvous.
            std::io::stdin().read_exact(&mut [0]).unwrap();
            panic!("parent must SIGKILL, not release the durable-WAL rendezvous");
        }),
    ));
    let core = crate::Core::open(config(root)).unwrap();
    command(&core, "gamerule randomTickSpeed 0".into());
    let mut ticks = core.subscribe();
    core.add_ticket(crate::core::RemoteTicket {
        owner: "overlay:crash-baseline".into(),
        chunk: pos,
        radius: 0,
        reason: "datapack baseline SIGKILL regression".into(),
        budget_chunks: 1,
    })
    .unwrap();
    let mut resident = false;
    for _ in 0..100 {
        if ticks
            .blocking_recv()
            .unwrap()
            .snapshot
            .chunks
            .iter()
            .any(|chunk| chunk.pos == pos)
        {
            resident = true;
            break;
        }
    }
    assert!(resident, "ticket must admit the actual generated chunk");
    command(
        &core,
        format!(
            "setblock {} 200 {} minecraft:gold_block",
            target[0], target[2]
        ),
    );
    ready
        .recv_timeout(std::time::Duration::from_secs(120))
        .expect("real block WAL must reach the durable rendezvous");
    core.shutdown(); // Cannot complete while the journal observer is held.
    panic!("parent must kill the process before shutdown/checkpoint");
}

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn kill_at_real_wal(root: &Path, pos: crate::boundary::ChunkPos, test: &str) -> u64 {
    // Rust's libtest names omit the leading crate component of module_path!.
    let test = test.split_once("::").unwrap().1;
    let mut child = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--ignored", "--nocapture"])
            .env(CHILD, root)
            .env(POSITION, format!("{},{}", pos.x, pos.z))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let tick = std::io::BufReader::new(stdout)
        .lines()
        .find_map(|line| {
            let line = line.unwrap();
            eprintln!("crash child {test}: {line}");
            line.strip_prefix(READY)
                .map(|tick| tick.parse::<u64>().unwrap())
        })
        .expect("child exited before the durable real block WAL");
    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    assert_eq!(
        status.signal(),
        Some(9),
        "the child must actually die from SIGKILL"
    );
    tick
}

fn assert_chunk(state: &mut crate::tick::TickState, expected: &mc_world::Chunk) {
    let pos = expected.pos;
    state
        .world
        .ensure_blocking(&[(pos, 0)].into_iter().collect());
    let actual = state
        .world
        .chunk(pos)
        .expect("recover the edited native chunk");
    let gold = state
        .registry
        .blocks
        .by_name_and_props(&id("minecraft:gold_block"), &[])
        .unwrap();
    for y in -64..320 {
        for z in 0..16 {
            for x in 0..16 {
                let wanted = if (x, y, z) == (0, 200, 0) {
                    Some(gold)
                } else {
                    expected.get_block(x, y, z)
                };
                assert_eq!(
                    actual.get_block(x, y, z),
                    wanted,
                    "untouched baseline at {x},{y},{z}"
                );
            }
        }
    }
    assert_eq!(
        actual.extras.iter().find(|(key, _)| key == "structures"),
        expected.extras.iter().find(|(key, _)| key == "structures"),
        "untouched selected-pack structure metadata must not regenerate as vanilla",
    );
}

#[derive(Clone, Copy)]
enum BaselineCase {
    Change,
    Remove,
    Add,
}

fn run_case(case: BaselineCase, test: &str) {
    if let Some(root) = std::env::var_os(CHILD) {
        crash_child(Path::new(&root));
        unreachable!();
    }
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());
    let cache = config.vanilla_data_dir.as_ref().unwrap();
    let mut registry = crate::registry::CoreRegistry::load(Some(cache)).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let vanilla = crate::core::overworld_terrain(&config, &registry).unwrap();
    let site = vanilla
        .village_sites_in_region((-68, -68), (101, 101))
        .into_iter()
        .next()
        .unwrap();
    let pos = crate::boundary::ChunkPos {
        x: site.start_chunk.0,
        z: site.start_chunk.1,
    };
    let vanilla_chunk = mc_world::ChunkGenerator::generate(&vanilla, pos);
    assert!(
        mc_world::structure_starts::read(&vanilla_chunk.extras, pos, false)
            .unwrap()
            .iter()
            .any(|site| site.structure.as_str().starts_with("minecraft:village_"))
    );
    let tags = village_tags();
    let resources: Vec<_> = tags
        .iter()
        .map(|path| (path.as_str(), br#"{"replace":true,"values":[]}"#.as_slice()))
        .collect();
    if !matches!(case, BaselineCase::Add) {
        pack(
            &directory.path().join("datapacks"),
            "crash-baseline",
            &resources,
        );
    }
    let core = crate::Core::open(config.clone()).unwrap();
    let registry = Arc::clone(core.registry());
    let generator = core.overworld_generator().unwrap();
    let mut expected = generator.generate(pos);
    assert_eq!(
        mc_world::structure_starts::read(&expected.extras, pos, false)
            .unwrap()
            .iter()
            .any(|site| site.structure.as_str().starts_with("minecraft:village_")),
        matches!(case, BaselineCase::Add)
    );
    core.shutdown();
    let checkpoint = crate::native_checkpoint::tests::read_manifest(&config)
        .unwrap()
        .tick;
    let wal_tick = kill_at_real_wal(directory.path(), pos, test);
    assert!(wal_tick > checkpoint);
    // Like production startup, first open the sole native writer so redb can
    // recover allocator bookkeeping after SIGKILL; do not publish a checkpoint.
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    assert_eq!(
        crate::native_checkpoint::tests::read_manifest(&config)
            .unwrap()
            .tick,
        checkpoint,
        "SIGKILL precedes checkpoint publication"
    );
    // A real vanilla village can also publish legitimate farming/door edits
    // before the gold command. Compare every untouched cell against the
    // original baseline plus ALL actual durable block post-images.
    let replay = crate::native_checkpoint::recover_with_reader(
        &config,
        Arc::clone(&registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    for (&(x, y, z), &state) in &replay.dimensions[&crate::dimensions::Dimension::Overworld].blocks
    {
        if x.div_euclid(16) == pos.x && z.div_euclid(16) == pos.z {
            eprintln!("durable block post-image {x},{y},{z}={state}");
            expected
                .set_block(
                    x.rem_euclid(16) as u8,
                    y,
                    z.rem_euclid(16) as u8,
                    mc_world::BlockStateId(state),
                )
                .unwrap();
        }
    }
    drop(replay);
    let pack_root = directory.path().join("datapacks/crash-baseline");
    match case {
        BaselineCase::Remove => {
            std::fs::rename(&pack_root, directory.path().join("removed-pack")).unwrap();
        }
        BaselineCase::Change => {
            for path in tags {
                write(
                    &pack_root.join("data"),
                    &path,
                    &std::fs::read(cache.join("data").join(&path)).unwrap(),
                );
            }
        }
        BaselineCase::Add => {
            pack(
                &directory.path().join("datapacks"),
                "crash-baseline",
                &resources,
            );
        }
    }
    for _ in 0..2 {
        let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
        let (mut recovered, _, _) =
            crate::core::recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
        assert_chunk(&mut recovered, &expected);
    }
    drop(store);
    let core = crate::Core::open(config.clone()).unwrap();
    core.subscribe().blocking_recv().unwrap();
    assert!(core.checkpoint());
    core.shutdown();
    assert!(
        crate::native_checkpoint::tests::read_manifest(&config)
            .unwrap()
            .tick
            > checkpoint
    );
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let (mut recovered, _, _) =
        crate::core::recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    assert_chunk(&mut recovered, &expected);
}

#[test]
#[ignore = "requires the local vanilla gameplay cache; run with --include-ignored"]
fn sigkill_real_wal_then_pack_change_preserves_original_baseline() {
    run_case(
        BaselineCase::Change,
        concat!(
            module_path!(),
            "::sigkill_real_wal_then_pack_change_preserves_original_baseline"
        ),
    );
}

#[test]
#[ignore = "requires the local vanilla gameplay cache; run with --include-ignored"]
fn sigkill_real_wal_then_pack_removal_preserves_original_baseline() {
    run_case(
        BaselineCase::Remove,
        concat!(
            module_path!(),
            "::sigkill_real_wal_then_pack_removal_preserves_original_baseline"
        ),
    );
}

#[test]
#[ignore = "requires the local vanilla gameplay cache; run with --include-ignored"]
fn sigkill_real_wal_then_added_pack_preserves_original_vanilla_baseline() {
    run_case(
        BaselineCase::Add,
        concat!(
            module_path!(),
            "::sigkill_real_wal_then_added_pack_preserves_original_vanilla_baseline"
        ),
    );
}

#[test]
#[ignore = "requires the local vanilla gameplay cache; run with --include-ignored"]
fn wal_only_missing_baseline_refuses_changed_inputs_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());
    let core = crate::Core::open(config.clone()).unwrap();
    let registry = Arc::clone(core.registry());
    core.shutdown();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let before = store.read_checkpoint().unwrap();
    let counts = store.record_counts().unwrap();
    let mut writer = crate::journal::JournalWriter::open(directory.path(), 1).unwrap();
    writer
        .append(
            before.tick + 1,
            before.tick + 1,
            &[crate::journal::JournalEntry::Block {
                pos: [30_400, 200, 32_000],
                state: registry.states.stone,
            }],
        )
        .unwrap();
    drop(writer);
    let wal: Vec<_> = std::fs::read_dir(directory.path().join("journal"))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    pack(
        &directory.path().join("datapacks"),
        "added-after-wal",
        &[(
            "minecraft/tags/worldgen/biome/has_structure/village_plains.json",
            br#"{"replace":true,"values":[]}"#,
        )],
    );
    let error = crate::core::recover_tick_state(&config, &registry, &store, false, None, None)
        .err()
        .expect("WAL-only missing baseline cannot use changed inputs");
    assert!(error.to_string().contains("baseline"), "{error}");
    assert!(
        error.to_string().contains("file/added-after-wal"),
        "{error}"
    );
    assert_eq!(store.read_checkpoint().unwrap().tick, before.tick);
    assert_eq!(store.read_checkpoint().unwrap().globals, before.globals);
    assert_eq!(store.record_counts().unwrap(), counts);
    for (path, bytes) in wal {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
