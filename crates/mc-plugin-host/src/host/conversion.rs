//! Mapping the server's events and failures to the contract's shapes.

use super::*;

use crate::bindings::exports::krasnokamen::plugin::events::{
    ChatSent, EventContext, InventoryClick, InventoryMenuClicked, InventoryStorageOutcome,
    InventoryStorageTransactionAnswered, OnlinePlayersAnswered, OperationAnswered,
    OperationCommitted, OperationOutcome, OperationPayload, OperationRefused, PlayerJoined,
    PlayerLeft, PlayerSnapshot, PlayerTeleportAnswered, PlayerTeleportFailure,
    PlayerTeleportOutcome, PlayerZoneTransition, StorageCasAnswered, StorageGetAnswered,
    ZoneCommandAnswered, ZoneCommandOutcome,
};
use crate::bindings::krasnokamen::plugin::storage::{
    StorageCasOutcome, StorageChange, StorageFailure, StorageGetOutcome, StorageRecord,
};
use crate::bindings::krasnokamen::plugin::types::Position;
use mc_script::{
    ScriptEventKind, ScriptInventoryClick, ScriptOperationOutcome, ScriptOperationPayload,
    ScriptOperationState, ScriptPlayerTeleportFailure, ScriptPluginStorageFailure,
    ScriptStorageChange,
};

/// Map one server event onto the contract's events.
///
/// Only the events the contract declares are mapped; anything else is skipped
/// rather than approximated, and the caller can see which names arrive by
/// subscribing to them.
pub(super) fn contract_events(
    event: &ScriptEvent,
    tick: u64,
) -> Option<(EventContext, Vec<Event>)> {
    let mapped = match event.kind() {
        ScriptEventKind::PlayerJoined {
            player_id,
            username,
            context,
        } => vec![Event::PlayerJoined(PlayerJoined {
            player: context.uuid().to_owned(),
            session: session_of(*player_id),
            name: username.clone(),
        })],
        ScriptEventKind::PlayerLeft { player_id, .. } => vec![Event::PlayerLeft(PlayerLeft {
            session: session_of(*player_id),
        })],
        ScriptEventKind::PlayerChat {
            player_id,
            message,
            context,
        } => vec![Event::ChatSent(ChatSent {
            player: context.uuid().to_owned(),
            session: session_of(*player_id),
            message: message.clone(),
        })],
        ScriptEventKind::PlayerCommand {
            player_id,
            root,
            arguments,
            context,
            ..
        } => vec![Event::CommandInvoked(CommandInvoked {
            player: context.uuid().to_owned(),
            session: session_of(*player_id),
            username: context.username().to_owned(),
            operator: context.operator(),
            position: Position {
                x: context.x(),
                y: context.y(),
                z: context.z(),
            },
            dimension: context.dimension().to_owned(),
            name: root.clone(),
            arguments: arguments.split_whitespace().map(str::to_owned).collect(),
            raw_arguments: arguments.clone(),
        })],
        // The server's own snapshot of who is connected, renamed into the
        // contract's shape. The host neither re-derives nor re-orders it.
        ScriptEventKind::OnlinePlayersResult {
            request_id,
            players,
            truncated,
        } => vec![Event::OnlinePlayersAnswered(OnlinePlayersAnswered {
            request: request_id.clone(),
            players: players
                .iter()
                .map(|player| PlayerSnapshot {
                    player: player.context().uuid().to_owned(),
                    session: session_of(player.player_id()),
                    name: player.context().username().to_owned(),
                    dimension: player.dimension().to_owned(),
                    position: Position {
                        x: player.context().x(),
                        y: player.context().y(),
                        z: player.context().z(),
                    },
                })
                .collect(),
            truncated: *truncated,
        })],
        // The server's own typed answer to one admitted teleport, renamed the
        // same way: the reason a request did not commit is the server's, and a
        // failure the contract does not name yet is reported as the owner not
        // taking the commit, because the world did not change either way.
        ScriptEventKind::PlayerTeleportResult {
            request_id,
            player_id,
            position,
            failure,
        } => vec![Event::PlayerTeleportAnswered(PlayerTeleportAnswered {
            request: request_id.clone(),
            session: session_of(*player_id),
            position: Position {
                x: position.x(),
                y: position.y(),
                z: position.z(),
            },
            outcome: match failure {
                Some(failure) => PlayerTeleportOutcome::Refused(teleport_failure(*failure)),
                None => PlayerTeleportOutcome::Committed,
            },
        })],
        ScriptEventKind::EntitySpawnResult {
            request_id,
            player_id,
            entity_type,
            position,
            failure,
        } => {
            use crate::bindings::exports::krasnokamen::plugin::events::{
                EntitySpawnAnswered, EntitySpawnFailure,
            };
            vec![Event::EntitySpawnAnswered(EntitySpawnAnswered {
                request: request_id.clone(),
                session: session_of(*player_id),
                entity_type: entity_type.clone(),
                position: Position {
                    x: position.x(),
                    y: position.y(),
                    z: position.z(),
                },
                failure: failure.map(|failure| match failure {
                    mc_script::ScriptEntitySpawnFailure::UnknownEntityType => {
                        EntitySpawnFailure::UnknownEntityType
                    }
                    mc_script::ScriptEntitySpawnFailure::ActorUnavailable => {
                        EntitySpawnFailure::ActorUnavailable
                    }
                    mc_script::ScriptEntitySpawnFailure::Busy => EntitySpawnFailure::Busy,
                    mc_script::ScriptEntitySpawnFailure::Rejected => EntitySpawnFailure::Rejected,
                    _ => EntitySpawnFailure::RuntimeUnavailable,
                }),
            })]
        }
        // The two storage answers are the contract's typed results: the plugin
        // reads what the server committed, never what it hoped it committed.
        ScriptEventKind::PluginStorageGetResult {
            request_id,
            value,
            version,
            failure,
            ..
        } => vec![Event::StorageGetAnswered(StorageGetAnswered {
            request: request_id.clone(),
            outcome: match failure {
                Some(failure) => StorageGetOutcome::Failed(storage_failure(*failure)),
                None => StorageGetOutcome::Read(StorageRecord {
                    value: value.clone(),
                    version: *version,
                }),
            },
        })],
        ScriptEventKind::PluginStorageCasResult {
            request_id,
            applied,
            version,
            failure,
            ..
        } => vec![Event::StorageCasAnswered(StorageCasAnswered {
            request: request_id.clone(),
            outcome: match failure {
                Some(failure) => StorageCasOutcome::Failed(storage_failure(*failure)),
                // The server reports a committed swap with the new revision and a
                // swap it did not apply without one; there is no third shape.
                None => match (*applied, *version) {
                    (true, Some(version)) => StorageCasOutcome::Committed(version),
                    _ => StorageCasOutcome::Refused,
                },
            },
        })],
        // The server's own typed answer to one operation this plugin issued,
        // renamed into the contract's event. The envelope is the plugin's request
        // id, the durable operation id it named and the outcome, and the durable id
        // is what a later `operation-status` addresses, so it is carried through
        // unchanged rather than re-derived from the request.
        ScriptEventKind::OperationResult {
            request_id,
            operation_id,
            outcome,
        } => vec![Event::OperationAnswered(operation_answered(
            request_id,
            operation_id.as_deref(),
            outcome,
        )?)],
        // The server's zone owner answers one bit, and this is that bit in the
        // contract's own shape. It names the zone it acted on rather than a request,
        // so the answer is keyed by the zone and the host adds no correlation id the
        // server never sent - inventing one would promise an attribution the answer
        // cannot support when a plugin has two commands for one zone in flight. The
        // finer reasons behind a refusal are the zone owner's own vocabulary,
        // which never crosses the script boundary, so the contract does not
        // invent one here: a member is added to `zone-command-outcome` by the
        // change that carries a reason.
        ScriptEventKind::ZoneCommandResult { zone_id, accepted } => {
            vec![Event::ZoneCommandAnswered(ZoneCommandAnswered {
                zone: zone_id.clone(),
                outcome: if *accepted {
                    ZoneCommandOutcome::Applied
                } else {
                    ZoneCommandOutcome::Refused
                },
            })]
        }
        ScriptEventKind::PlayerZoneEntered {
            player_id,
            context,
            zone_id,
        } => vec![Event::PlayerZoneEntered(PlayerZoneTransition {
            player: context.uuid().to_owned(),
            session: session_of(*player_id),
            zone: zone_id.clone(),
        })],
        ScriptEventKind::PlayerZoneExited {
            player_id,
            context,
            zone_id,
        } => vec![Event::PlayerZoneExited(PlayerZoneTransition {
            player: context.uuid().to_owned(),
            session: session_of(*player_id),
            zone: zone_id.clone(),
        })],
        // The owners of one inventory and storage transaction answer one bit, and
        // this is that bit in the contract's own shape. The answer names the
        // plugin's correlation id and nothing else, as the command does: this
        // transaction has no durable operation id, and the reasons behind a
        // refusal stay with the owners that decided it.
        ScriptEventKind::InventoryStorageTransactionResult {
            request_id,
            committed,
        } => vec![Event::InventoryStorageTransactionAnswered(
            InventoryStorageTransactionAnswered {
                request: request_id.clone(),
                outcome: if *committed {
                    InventoryStorageOutcome::Committed
                } else {
                    InventoryStorageOutcome::Refused
                },
            },
        )],
        // One click on one of this plugin's own menu slots, in the contract's own
        // shape. The server's menu owner already addressed the event to the plugin
        // that opened the menu and normalized the click, so the host carries the
        // owner's own values through: the id and the slot index unchanged, and the
        // player and the session exactly as `command-invoked` takes them - the
        // identity from the context the click was built with, and the runtime id of
        // the connection it arrived on, not a lookup of whatever session the
        // identity holds now. A kind this contract does not name yet is dropped
        // rather than reported as one of the four it does: a plugin told "primary"
        // for a click the server decided was something else would act on a promise
        // the owner never made.
        ScriptEventKind::InventoryMenuClicked {
            player_id,
            context,
            menu_id,
            slot,
            click,
        } => {
            let click = inventory_click(*click)?;
            vec![Event::InventoryMenuClicked(InventoryMenuClicked {
                player: context.uuid().to_owned(),
                session: session_of(*player_id),
                menu: menu_id.clone(),
                slot: *slot,
                click,
            })]
        }
        kind => vec![
            crate::world_events::map_event(kind)
                .or_else(|| crate::client_presentation::map_event(kind))?,
        ],
    };
    Some((
        EventContext {
            tick,
            first_sequence: 0,
            count: u32::try_from(mapped.len()).unwrap_or(u32::MAX),
        },
        mapped,
    ))
}

/// The server's storage failure, in the contract's own vocabulary.
///
/// The server's enum is `non_exhaustive`, and a failure the contract does not
/// name yet is reported as a durability failure: storage did not answer, and the
/// plugin must not read that as "the key holds nothing".
fn storage_failure(failure: ScriptPluginStorageFailure) -> StorageFailure {
    match failure {
        ScriptPluginStorageFailure::Unavailable => StorageFailure::Unavailable,
        ScriptPluginStorageFailure::DurabilityFailed | _ => StorageFailure::DurabilityFailed,
    }
}

/// The server's teleport failure, in the contract's own vocabulary.
///
/// The server's enum is `non_exhaustive`, and a reason the contract does not name
/// yet is reported as `runtime-unavailable`: the owner did not take the commit,
/// which is what a plugin has to know before it retries.
fn teleport_failure(failure: ScriptPlayerTeleportFailure) -> PlayerTeleportFailure {
    match failure {
        ScriptPlayerTeleportFailure::PlayerUnavailable => PlayerTeleportFailure::PlayerUnavailable,
        ScriptPlayerTeleportFailure::DimensionMismatch => PlayerTeleportFailure::DimensionMismatch,
        ScriptPlayerTeleportFailure::RuntimeUnavailable | _ => {
            PlayerTeleportFailure::RuntimeUnavailable
        }
    }
}

/// Preserve native correlation, including queries without durable ids and
/// per-member details on rejected operations.
fn operation_answered(
    request_id: &str,
    operation_id: Option<&str>,
    outcome: &ScriptOperationOutcome,
) -> Option<OperationAnswered> {
    let payload = operation_payload(outcome.payload())?;
    let outcome = match (outcome.state(), outcome.failure()) {
        (ScriptOperationState::Committed, None) => {
            OperationOutcome::Committed(OperationCommitted {
                revision: outcome.revision()?,
                payload,
            })
        }
        (ScriptOperationState::Rejected, Some(failure)) => {
            OperationOutcome::Refused(OperationRefused {
                reason: operation_failure(failure),
                payload,
            })
        }
        _ => return None,
    };
    Some(OperationAnswered {
        request: request_id.to_owned(),
        operation_id: operation_id.map(str::to_owned),
        outcome,
    })
}

fn operation_payload(payload: &ScriptOperationPayload) -> Option<OperationPayload> {
    Some(match payload {
        ScriptOperationPayload::None => OperationPayload::None,
        ScriptOperationPayload::StorageBatch { changes } => {
            OperationPayload::StorageBatch(changes.iter().map(storage_change).collect())
        }
        ScriptOperationPayload::WorldStructure { result } => {
            OperationPayload::WorldStructure(crate::domain_world_structures::encode_result(result)?)
        }
        ScriptOperationPayload::Npc { result } => {
            OperationPayload::Npc(crate::domain_npcs::encode_result(result)?)
        }
        ScriptOperationPayload::Settlement { result } => {
            crate::domain_settlements::encode_result(result)?
        }
        ScriptOperationPayload::NpcOrder { result } => {
            OperationPayload::NpcOrder(crate::domain_npcs::encode_order_result(result)?)
        }
        ScriptOperationPayload::NpcRoute { result } => {
            OperationPayload::NpcRoute(crate::domain_npc_routes::encode_result(result)?)
        }
        ScriptOperationPayload::OwnedInventory { result } => {
            OperationPayload::OwnedInventory(crate::domain_inventories::encode_result(result)?)
        }
        _ => return None,
    })
}

/// One click kind as the contract names it, or nothing when the contract has no
/// member for it.
///
/// The four members the contract carries are exactly the four the server can
/// decide today (`mc_script::ScriptInventoryClick`), and the host renames them
/// one for one. The server's enum is `non_exhaustive`, so a kind it starts
/// deciding later reaches this match as an unrecognized variant: it is left to the
/// slice that extends the contract, which is why the click is dropped instead of
/// being reported as one of the four - the plugin would act on a promise its menu
/// owner never made.
fn inventory_click(click: ScriptInventoryClick) -> Option<InventoryClick> {
    match click {
        ScriptInventoryClick::Primary => Some(InventoryClick::Primary),
        ScriptInventoryClick::Secondary => Some(InventoryClick::Secondary),
        ScriptInventoryClick::ShiftPrimary => Some(InventoryClick::ShiftPrimary),
        ScriptInventoryClick::ShiftSecondary => Some(InventoryClick::ShiftSecondary),
        _ => None,
    }
}

/// One key a committed batch changed, as the contract names it.
fn storage_change(change: &ScriptStorageChange) -> StorageChange {
    StorageChange {
        key: change.key.clone(),
        deleted: change.deleted,
    }
}

/// The session a runtime player id names: the contract's `session-id` is exactly
/// the server's runtime id, so this is a rename, not a lookup.
fn session_of(player_id: ScriptPlayerId) -> u64 {
    player_id.value()
}
