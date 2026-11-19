use super::tests::{native_checkpoint, native_state};
use super::*;
use crate::boundary::{BlockPos, ChangeSet, ChunkPos, ConnectionId, EntityId, PlayerInput};
use crate::commands::Command;
use crate::settlements::{commands::SettlementCommand, starter_chest};
use std::sync::Arc;

const PLAYER: uuid::Uuid = uuid::Uuid::nil();
const AT: [i32; 3] = [8, 201, 8];

pub(super) fn founder(state: &mut TickState) -> (ConnectionId, EntityId) {
    let connection = ConnectionId(77);
    crate::tick::run_tick(
        state,
        &[PlayerInput::Join {
            connection,
            uuid: PLAYER,
            name: "Founder".into(),
            permission_level: 0,
        }],
    );
    let actor = state.connections.entity_of(connection).unwrap();
    let slot = state.entities.slot_of(actor).unwrap();
    state.entities.set_pos(slot, [8.5, 201.0, 8.5]);
    (connection, actor)
}

fn create(
    state: &mut TickState,
    connection: ConnectionId,
    actor: EntityId,
    name: &str,
) -> ChangeSet {
    let mut changes = ChangeSet::default();
    crate::command_execution::execute(
        state,
        connection,
        actor,
        &Command::Settlement(SettlementCommand::Create {
            name: name.into(),
            radius: 48,
        }),
        &mut changes,
    );
    changes
}

/// Leave exactly one admissible site, so every rejection is observable rather
/// than being hidden by the search finding another candidate.
fn only_site(state: &mut TickState) -> [i32; 3] {
    for x in 7..=9 {
        for z in 7..=9 {
            if (x, z) == (8, 8) {
                continue;
            }
            for y in 197..=205 {
                state.world.set_block(x, y, z, state.registry.states.stone);
                state.block_entities.remove(&(x, y, z));
            }
        }
    }
    let at = [9, 201, 8];
    state
        .world
        .set_block(at[0], at[1], at[2], state.registry.states.air);
    state
        .world
        .set_block(at[0], at[1] + 1, at[2], state.registry.states.air);
    at
}

#[test]
fn starter_site_requires_headroom_clear_neighbours_and_non_container_support() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(dir.path().to_owned(), 42);
    let mut state = native_state(&config);
    let at = only_site(&mut state);
    let executor = crate::command_console::EXECUTOR;
    let site = storage_chest_site(&state, executor, AT).unwrap();
    assert_eq!(site.at, at);
    let chest = site.block;
    state
        .world
        .set_block(at[0], at[1] + 1, at[2], state.registry.states.stone);
    assert!(storage_chest_site(&state, executor, AT).is_none());
    state
        .world
        .set_block(at[0], at[1] + 1, at[2], state.registry.states.air);
    state.world.set_block(at[0] + 1, at[1], at[2], chest);
    assert!(storage_chest_site(&state, executor, AT).is_none());
    state
        .world
        .set_block(at[0] + 1, at[1], at[2], state.registry.states.air);
    state.world.set_block(at[0], at[1] - 1, at[2], chest);
    assert!(storage_chest_site(&state, executor, AT).is_none());
    state
        .world
        .set_block(at[0], at[1] - 1, at[2], state.registry.states.stone);
    assert_eq!(storage_chest_site(&state, executor, AT).unwrap().at, at);
}

#[test]
fn founding_refusals_write_nothing_and_no_site_does_not_spend_the_grant() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(dir.path().to_owned(), 42);
    let mut state = native_state(&config);
    let (connection, actor) = founder(&mut state);
    let at = only_site(&mut state);
    let count = state.settlements.records().count();
    let rejected = create(&mut state, connection, actor, "invalid name");
    assert_eq!(state.settlements.records().count(), count);
    assert!(rejected.blocks.is_empty());
    assert_eq!(
        state.world.block_at(at[0], at[1], at[2]),
        Some(state.registry.states.air)
    );
    assert!(!starter_chest::received(&state, PLAYER));
    state
        .world
        .set_block(at[0], at[1], at[2], state.registry.states.stone);
    let blocked = create(&mut state, connection, actor, "blocked");
    let town = state.settlements.find("blocked").unwrap();
    assert!(town.bootstrap_storage.is_none());
    assert!(blocked.blocks.is_empty());
    assert!(!starter_chest::received(&state, PLAYER));
    only_site(&mut state);
    let granted = create(&mut state, connection, actor, "granted");
    assert_eq!(granted.blocks.len(), 1);
    assert!(starter_chest::received(&state, PLAYER));
    let town = state.settlements.find("granted").unwrap();
    let id = town.id;
    let chest = town.bootstrap_storage.unwrap();
    state
        .world
        .set_block(chest[0], chest[1], chest[2], state.registry.states.air);
    state.block_entities.remove(&(chest[0], chest[1], chest[2]));
    assert!(state.settlements.remove(id));
    let repeated = create(&mut state, connection, actor, "repeated");
    assert!(repeated.blocks.is_empty());
    assert!(
        state
            .settlements
            .find("repeated")
            .unwrap()
            .bootstrap_storage
            .is_none()
    );
    assert!(starter_chest::received(&state, PLAYER));
}

#[test]
fn console_founding_has_no_player_grant_or_free_chest() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(dir.path().to_owned(), 42);
    let mut state = native_state(&config);
    let changes = create(
        &mut state,
        crate::command_console::CONNECTION,
        crate::command_console::EXECUTOR,
        "server",
    );
    assert!(changes.blocks.is_empty());
    assert!(
        state
            .settlements
            .find("server")
            .unwrap()
            .bootstrap_storage
            .is_none()
    );
}

#[test]
fn starter_grant_and_empty_chest_recover_from_wal_without_a_post_grant_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(dir.path().to_owned(), 42);
    let mut state = native_state(&config);
    let (connection, actor) = founder(&mut state);
    native_checkpoint(&mut state, &config, 1);
    let (mut output, ..) = crate::tick::run_tick(&mut state, &[]);
    assert_eq!(output.tick, 2);
    let changes = Arc::get_mut(&mut output.changes).unwrap();
    crate::command_execution::execute(
        &mut state,
        connection,
        actor,
        &Command::Settlement(SettlementCommand::Create {
            name: "walgrant".into(),
            radius: 48,
        }),
        changes,
    );
    let town = state.settlements.find("walgrant").unwrap();
    let id = town.id;
    let chest = town.bootstrap_storage.unwrap();
    let entries = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
    assert!(entries.iter().any(|entry| matches!(entry,
        crate::journal::JournalEntry::StarterChestGranted { player } if *player == PLAYER)));
    let mut writer = crate::journal::JournalWriter::open(dir.path(), 1).unwrap();
    writer.append(2, 2, &entries).unwrap();
    writer.flush().unwrap();
    assert_eq!(
        crate::native_checkpoint::tests::fixture_store(&config)
            .unwrap()
            .read_checkpoint()
            .unwrap()
            .tick,
        1
    );
    drop(writer);
    drop(state);
    let mut reopened =
        crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
            .unwrap();
    assert!(starter_chest::received(&reopened, PLAYER));
    assert_eq!(
        reopened.settlements.get(id).unwrap().bootstrap_storage,
        Some(chest)
    );
    reopened
        .world
        .ensure_blocking(&[(ChunkPos { x: 0, z: 0 }, 0)].into_iter().collect());
    assert!(is_chest_block(&reopened, chest));
    let chunk = reopened.world.chunk(ChunkPos { x: 0, z: 0 }).unwrap();
    assert!(
        chunk.chests[&BlockPos {
            x: chest[0],
            y: chest[1],
            z: chest[2]
        }]
            .slots
            .iter()
            .all(|slot| slot.is_empty())
    );
    let (connection, actor) = founder(&mut reopened);
    assert!(reopened.settlements.remove(id));
    let repeated = create(&mut reopened, connection, actor, "walrepeat");
    assert!(repeated.blocks.is_empty());
    assert!(
        reopened
            .settlements
            .find("walrepeat")
            .unwrap()
            .bootstrap_storage
            .is_none()
    );
}

#[test]
fn starter_grant_moves_with_global_authority_instead_of_resetting_per_dimension() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(dir.path().to_owned(), 42);
    let mut root = native_state(&config);
    let mut child = crate::settlements::allocator::tests::test_state();
    child.dimension = crate::dimensions::Dimension::Nether;
    starter_chest::record(&mut root, PLAYER);
    crate::dimensions::swap_globals(&mut root, &mut child);
    assert!(starter_chest::received(&child, PLAYER));
    let (connection, actor) = founder(&mut child);
    let changes = create(&mut child, connection, actor, "nether");
    assert!(changes.blocks.is_empty());
    assert!(
        child
            .settlements
            .find("nether")
            .unwrap()
            .bootstrap_storage
            .is_none()
    );
    crate::dimensions::swap_globals(&mut root, &mut child);
    assert!(starter_chest::received(&root, PLAYER));
    assert!(!starter_chest::received(&child, PLAYER));
}
