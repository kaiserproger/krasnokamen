//! Live network players in a staged battle: a player who takes a front-rank
//! slot with that rank's kit, and spectators placed behind a player.
//! Console only (`battle watch|live|spectate`).
//!
//! Invariant: a held slot is rank 0 of one file of a foot regiment. The file's
//! soldiers stand from rank 1 back (`FormationRuntime::held`), so no soldier
//! steps into the holder's place. The holder is an ordinary player: he takes
//! damage and dies. The battle moves him only when he (re)enters the rank:
//! `battle live` before start, and each respawn, which puts him back into a
//! current front-rank slot of his side near its centre with the kit again.
//! `keep_inventory` keeps the kit from littering the field. Enemy soldiers in
//! reach of a holder swing at him (`strike`) through the ordinary player
//! damage path; the battle's own targets stay soldiers.
use super::{BattleIntent, BattleState, Frame, Output};
use crate::battle::formation::{FormationRuntime, Order, Regiment, position, slot_position};
use crate::battle::ground::{self, Placement};
use crate::boundary::{ChangeSet, EntityEvent, EntityId, Hand};
use crate::commands::{Command, GameMode, Target};
use crate::intent::{DamageSource, Footprint, IntentKind};
use crate::tick::TickState;
use mc_data::ItemStack;

const MAX_WATCHED: usize = 8;
/// Spectator offset behind the holder (blocks back, blocks up) and pitch.
const SPECTATOR_BACK: f64 = 4.0;
const SPECTATOR_UP: f64 = 2.5;
const SPECTATOR_PITCH: f32 = 20.0;
/// Player body for grounding a placement.
const PLAYER_HALF_WIDTH: f64 = 0.3;
const PLAYER_HEIGHT: f64 = 1.8;
/// One soldier's blow on a live player: an ordinary iron-weapon melee hit,
/// reduced by his armour and blocked by his raised shield like any player's.
const SOLDIER_HIT: f32 = 5.0;
/// A soldier in sword reach of a live enemy player decides once a second
/// (the battle's attack decision cadence) and swings half the time.
const STRIKE_DECISION_TICKS: u64 = 20;
const STRIKE_ROLLS: u32 = 5_000;
const STRIKE_REACH: f64 = 2.5;
const STRIKE_SALT: u64 = 0x4C49_5645;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hold {
    pub name: String,
    pub side: u16,
    pub regiment: usize,
    pub file: usize,
    /// Where the holder was placed before start, and his yaw.
    pub at: [f64; 3],
    pub yaw: f32,
    /// The kit taken from his file's soldier at placement, given again on
    /// every respawn and rejoin wherever the rank puts him.
    pub kit: [Option<ItemStack>; 6],
    pub kit_from: uuid::Uuid,
    /// The holder was seen dead and is placed again when he respawns.
    pub dead: bool,
    pub deaths: u32,
}

#[derive(Debug, Default)]
pub(crate) struct LiveState {
    watched: Vec<String>,
    /// Entity last announced online for each watched name.
    announced: Vec<Option<EntityId>>,
    pub holds: Vec<Hold>,
    /// Living holders this tick (entity, side, position): what enemy soldiers
    /// in reach swing at (`strike`).
    targets: Vec<(EntityId, u16, [f64; 3])>,
}

pub(crate) fn valid_name(name: &str) -> bool {
    (1..=16).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub(crate) fn parse_side(word: &str) -> Option<u16> {
    match word {
        "red" | "1" => Some(1),
        "blue" | "2" => Some(2),
        _ => None,
    }
}

pub(crate) fn watch_count_ok(names: &[String]) -> bool {
    (1..=MAX_WATCHED).contains(&names.len())
}

/// The connected player with this account name, alive or dead.
fn player_named(state: &TickState, name: &str) -> Option<EntityId> {
    state
        .players
        .keys()
        .copied()
        .find(|id| state.connections.name_of(*id) == Some(name))
}

/// Dead: no live body (a dead player's body leaves the live slots) or no health.
fn is_dead(state: &TickState, id: EntityId) -> bool {
    state
        .entities
        .slot_of(id)
        .is_none_or(|slot| !state.entities.body(slot).alive)
        || state.players[&id].vitals.is_dead()
}

/// Per tick, before the battle kernels: log each watched name's join and
/// leave once (the runner's barrier waits on these exact lines), and put each
/// holder who respawned back into the rank.
pub(crate) fn update(state: &mut TickState) {
    for index in 0..state.hostile.battle.live.watched.len() {
        let name = &state.hostile.battle.live.watched[index];
        let online = player_named(state, name);
        if online == state.hostile.battle.live.announced[index] {
            continue;
        }
        let name = name.clone();
        state.hostile.battle.live.announced[index] = online;
        match online {
            Some(id) => tracing::info!("Battle player online: {name} entity={}", id.0),
            None => tracing::info!("Battle player offline: {name}"),
        }
    }
    state.hostile.battle.live.targets.clear();
    for index in 0..state.hostile.battle.live.holds.len() {
        let hold = &state.hostile.battle.live.holds[index];
        let Some(player) = player_named(state, &hold.name) else {
            continue;
        };
        let dead = is_dead(state, player);
        if dead && !hold.dead {
            let hold = &mut state.hostile.battle.live.holds[index];
            hold.dead = true;
            hold.deaths += 1;
            tracing::info!("Battle live died: {} deaths={}", hold.name, hold.deaths);
        } else if !dead && hold.dead {
            state.hostile.battle.live.holds[index].dead = false;
            match reenter(state, index, player) {
                Ok(line) => tracing::info!("{line}"),
                Err(error) => {
                    // Not left bare: the kit again where the respawn put him.
                    let kit = state.hostile.battle.live.holds[index].kit.clone();
                    equip(state, player, &kit);
                    tracing::warn!("Battle live respawn not placed (kit restored): {error}");
                }
            }
        }
        if !is_dead(state, player) {
            let pos = state
                .entities
                .pos(state.entities.slot_of(player).expect("living player"));
            let side = state.hostile.battle.live.holds[index].side;
            state.hostile.battle.live.targets.push((player, side, pos));
        }
    }
}

/// Enemy soldiers in sword reach of a living holder swing at him, at the
/// battle's attack decision cadence: an ordinary melee hit through the player
/// damage path (armour, shield block, knockback), never the battle's outcome
/// roll. Additive: battle targets stay soldiers; mounted, archer and routing
/// men do not swing.
pub(super) fn strike(frame: &Frame<'_>, battle: &BattleState, out: &mut Output) {
    let columns = &battle.columns;
    for &(player, side, pos) in &battle.live.targets {
        for bi in 0..columns.ids.len() {
            if !columns.active[bi]
                || columns.carried[bi]
                || columns.ranged[bi]
                || !super::enemies(columns.factions[bi], side)
            {
                continue;
            }
            let id = columns.ids[bi];
            if !(frame.tick + id.0 as u64).is_multiple_of(STRIKE_DECISION_TICKS) {
                continue;
            }
            let at = columns.positions[bi];
            let reach = (at[0] - pos[0]).powi(2) + (at[2] - pos[2]).powi(2);
            if reach > STRIKE_REACH * STRIKE_REACH || (at[1] - pos[1]).abs() > 2.0 {
                continue;
            }
            let Some(slot) = columns.slots[bi] else {
                continue;
            };
            if frame.entities.row(slot).entity_life.mount.vehicle.is_some()
                || (bi < battle.formation.members.len()
                    && battle.formation.context_at(bi).routing)
            {
                continue;
            }
            if super::combat::roll(id, frame.tick / STRIKE_DECISION_TICKS, STRIKE_SALT)
                >= STRIKE_ROLLS
            {
                continue;
            }
            out.events.push(EntityEvent::Swing {
                entity: id,
                hand: Hand::Main,
                animation: crate::boundary::SwingKind::Whack,
                duration_ticks: 21,
            });
            out.intents.push(BattleIntent {
                actor: id,
                kind: IntentKind::DamageEntity {
                    target: player,
                    amount: SOLDIER_HIT,
                    kind: crate::survival::DamageKind::Melee,
                    source: Some(DamageSource::Melee {
                        attacker: id,
                        position: at,
                        yaw: frame.entities.rot(slot)[0],
                        bonus_knockback: 0.0,
                        sweep: false,
                        fire_ticks: 0,
                        bane_ticks: 0,
                    }),
                },
                footprint: Footprint {
                    entity_reads: vec![id, player],
                    entity_writes: vec![player],
                    ..Default::default()
                },
            });
        }
    }
}

pub(crate) fn watch(state: &mut TickState, names: &[String]) -> String {
    let live = &mut state.hostile.battle.live;
    live.watched = names.to_vec();
    live.announced = vec![None; names.len()];
    format!("Battle watching: {}", names.join(","))
}

/// A holder's hit on a soldier of his own side, refused like the battle's
/// own fighters, who only ever strike an enemy faction (`retained_enemy`).
pub(crate) fn same_side(state: &TickState, attacker: EntityId, victim: EntityId) -> bool {
    let holds = &state.hostile.battle.live.holds;
    if holds.is_empty() {
        return false;
    }
    let Some(name) = state.connections.name_of(attacker) else {
        return false;
    };
    let Some(hold) = holds.iter().find(|hold| hold.name == name) else {
        return false;
    };
    state
        .entities
        .slot_of(victim)
        .is_some_and(|slot| state.entities.body(slot).faction == hold.side)
}

/// Interval the formation uses for a regiment's slots right now.
fn interval(r: &Regiment) -> f64 {
    if r.gap <= 6.0 {
        r.interval
    } else {
        r.spec.frontage
    }
}

/// A regiment that can take a holder in `tier`: 0 is `side`'s sword-and-shield
/// foot line (not spear, archer or mounted); 1 any foot line; 2 a broken foot
/// line that still has men. Before start only tier 0 exists; once running a
/// line must still have men, and tiers 0 and 1 must not be broken.
fn eligible(state: &TickState, regiment: &Regiment, side: u16, tier: u8) -> bool {
    let spec = &regiment.spec;
    if spec.faction != side || spec.order == Order::Hold || spec.frontage != 1.0 {
        return false;
    }
    if state.hostile.battle.running
        && (regiment.live_count == 0 || (tier < 2 && regiment.morale.broken))
    {
        return false;
    }
    if tier > 0 {
        return true;
    }
    let formation = &state.hostile.battle.formation;
    let trident = mc_data::Identifier::parse("minecraft:trident".to_owned())
        .ok()
        .and_then(|id| state.registry.items.id_of(&id));
    let first = formation.members[regiment.start].id;
    let kind = (regiment.start..regiment.start + regiment.len)
        .find_map(|i| state.entities.slot_of(formation.members[i].id))
        .or_else(|| state.entities.slot_of(first));
    kind.is_some_and(|slot| {
        let row = state.entities.row(slot);
        row.entity_life.mount.vehicle.is_none()
            && row.appearance.equipment[0]
                .as_ref()
                .is_some_and(|stack| Some(stack.item_id) != trident)
    })
}

/// The front-rank file nearest the centre of `side`'s foot line, among files
/// nobody else holds (`keep` is the caller's own hold, which stays eligible),
/// in the first tier that has one: standing infantry, then (once running) any
/// standing foot line, then any foot line with men left.
fn choose_slot(
    state: &TickState,
    side: u16,
    keep: Option<(usize, usize)>,
) -> Result<(usize, usize), String> {
    let formation = &state.hostile.battle.formation;
    let foot: Vec<usize> = (0..formation.regiments.len())
        .filter(|&i| {
            let spec = &formation.regiments[i].spec;
            spec.faction == side && spec.order != Order::Hold && spec.frontage == 1.0
        })
        .collect();
    if foot.is_empty() {
        return Err(format!("no staged foot regiment on side {side}"));
    }
    let centre = foot.iter().fold([0.0, 0.0], |sum, &i| {
        let origin = formation.regiments[i].spec.origin;
        [sum[0] + origin[0], sum[1] + origin[2]]
    });
    let centre = [centre[0] / foot.len() as f64, centre[1] / foot.len() as f64];
    let tiers = if state.hostile.battle.running { 0..3 } else { 0..1 };
    for tier in tiers {
        let mut best: Option<(f64, usize, usize)> = None;
        for &i in &foot {
            let r = &formation.regiments[i];
            if !eligible(state, r, side, tier) {
                continue;
            }
            for file in 0..usize::from(r.spec.files).min(r.len) {
                if formation.held.contains(&(i, file)) && keep != Some((i, file)) {
                    continue;
                }
                let pos = position(&r.spec, file, 0, interval(r));
                let distance = (pos[0] - centre[0]).powi(2) + (pos[2] - centre[1]).powi(2);
                if best.is_none_or(|(d, _, _)| distance < d) {
                    best = Some((distance, i, file));
                }
            }
        }
        if let Some((_, i, file)) = best {
            return Ok((i, file));
        }
    }
    let lines: Vec<String> = foot
        .iter()
        .map(|&i| {
            let r = &formation.regiments[i];
            format!(
                "{}:live={},broken={}",
                r.spec.id, r.live_count, r.morale.broken
            )
        })
        .collect();
    Err(format!(
        "no free front-rank file on side {side} (foot lines {})",
        lines.join(" ")
    ))
}

/// Members of one file, front to back in staging order.
fn file_members(formation: &FormationRuntime, regiment: usize, file: usize) -> Vec<usize> {
    let r = &formation.regiments[regiment];
    (r.start + file..r.start + r.len)
        .step_by(usize::from(r.spec.files))
        .collect()
}

/// The equipment of the first soldier of the file whose body still exists,
/// else of any soldier of the regiment.
fn kit_of(
    state: &TickState,
    regiment: usize,
    file: usize,
) -> Option<(uuid::Uuid, [Option<ItemStack>; 6])> {
    let formation = &state.hostile.battle.formation;
    let r = &formation.regiments[regiment];
    file_members(formation, regiment, file)
        .into_iter()
        .chain(r.start..r.start + r.len)
        .find_map(|index| {
            let slot = state.entities.slot_of(formation.members[index].id)?;
            let row = state.entities.row(slot);
            Some((row.uuid, row.appearance.equipment.clone()))
        })
}

/// Vanilla yaw of a horizontal facing (0 = +z, 90 = -x).
fn yaw_of(facing: [f64; 2]) -> f32 {
    -(mc_physics::mth::atan2(facing[0], facing[1]) as f32) * mc_physics::mth::RAD_TO_DEG
}

fn teleport(state: &mut TickState, id: EntityId, to: [f64; 3], yaw: f32, pitch: f32) {
    let slot = state.entities.slot_of(id).expect("online player has a body");
    crate::mounts::dismount(state, id);
    state.entities.set_rot(slot, yaw, pitch);
    state.entities.set_pos(slot, to);
    state.entities.set_vel(slot, [0.0; 3]);
    state.entities.set_ground(slot, false);
    if let Some(player) = state.players.get_mut(&id) {
        player.fall_distance = 0.0;
    }
    if let Some(connection) = state.connections.entity_connection(id) {
        state.correct_position(connection, to, yaw, pitch);
    }
}

/// Mob equipment index -> player inventory index (main hand is the selected
/// hotbar slot).
fn player_index(equipment: usize, selected: usize) -> usize {
    [selected, 40, 36, 37, 38, 39][equipment]
}

fn equip(state: &mut TickState, id: EntityId, kit: &[Option<ItemStack>; 6]) -> usize {
    let player = state.players.get_mut(&id).expect("online player");
    let selected = usize::from(player.inventory.selected.min(8));
    let mut changed = 0_u64;
    for (equipment, stack) in kit.iter().enumerate() {
        let index = player_index(equipment, selected);
        player.inventory.slots[index] = stack.clone().unwrap_or(ItemStack::EMPTY);
        changed |= 1 << index;
    }
    let changes = player.pickup_changes(changed);
    if let Some(connection) = state.connections.entity_connection(id) {
        for change in changes {
            state.push_player(connection, change);
        }
    }
    crate::projectiles::weapons::sync(state, id);
    kit.iter().filter(|stack| stack.is_some()).count()
}

/// The kit must not litter the field when the holder dies.
fn keep_inventory(state: &mut TickState) {
    if state.gamerules.get("keep_inventory").map(String::as_str) != Some("true") {
        state
            .gamerules
            .insert("keep_inventory".into(), "true".into());
        state.gamerules_dirty = true;
    }
}

/// `battle live <name> <side>`: before start, hold a free infantry
/// front-rank file nearest the line centre (or the name's existing hold),
/// give the player that file's kit and stand him in rank 0 facing the enemy;
/// the file's soldiers move one rank back. After start a held name is only
/// re-armed (a rejoin): the battle moves him only on respawn.
pub(crate) fn live(state: &mut TickState, name: &str, side: u16) -> Result<String, String> {
    if state.hostile.battle.formation.regiments.is_empty() {
        return Err("no staged formation; setup required".into());
    }
    let player = player_named(state, name).ok_or_else(|| format!("{name} is not online"))?;
    if is_dead(state, player) {
        return Err(format!("{name} is dead; placed again on respawn"));
    }
    let running = state.hostile.battle.running;
    let existing = state
        .hostile
        .battle
        .live
        .holds
        .iter()
        .position(|hold| hold.name == name);
    let (regiment, file) = match existing {
        Some(index) => {
            let hold = &state.hostile.battle.live.holds[index];
            if hold.side != side {
                return Err(format!("{name} holds a slot of side {}, not {side}", hold.side));
            }
            (hold.regiment, hold.file)
        }
        None if running => {
            return Err(format!(
                "battle running; {name} holds no slot and a new one is taken only before start"
            ));
        }
        None => choose_slot(state, side, None)?,
    };
    let spec = state.hostile.battle.formation.regiments[regiment].spec;
    keep_inventory(state);
    let (at, yaw, source, kit) = match existing {
        Some(index) => {
            let hold = &state.hostile.battle.live.holds[index];
            (hold.at, hold.yaw, hold.kit_from, hold.kit.clone())
        }
        None => {
            let (source, kit) =
                kit_of(state, regiment, file).ok_or("held regiment has no soldier left to copy")?;
            let at = take_front(state, regiment, file)?;
            let yaw = yaw_of(spec.facing);
            state.hostile.battle.formation.held.push((regiment, file));
            state.hostile.battle.live.holds.push(Hold {
                name: name.to_owned(),
                side,
                regiment,
                file,
                at,
                yaw,
                kit: kit.clone(),
                kit_from: source,
                dead: false,
                deaths: 0,
            });
            (at, yaw, source, kit)
        }
    };
    let slots = equip(state, player, &kit);
    if running {
        return Ok(format!(
            "Battle live rearmed: {name} side={side} regiment={} file={file} kit_slots={slots} kit_from={source} moved=false",
            spec.id
        ));
    }
    teleport(state, player, at, yaw, 0.0);
    Ok(format!(
        "Battle live placed: {name} side={side} regiment={} file={file} rank=0 at={:.2},{:.2},{:.2} yaw={yaw:.1} kit_slots={slots} kit_from={source}",
        spec.id, at[0], at[1], at[2]
    ))
}

/// Move every soldier of the file one rank back and return where its front
/// soldier stood (his settled ground). The deepest soldier is placed one rank
/// deeper; settle grounds them before the next physics step.
fn take_front(state: &mut TickState, regiment: usize, file: usize) -> Result<[f64; 3], String> {
    let members = file_members(&state.hostile.battle.formation, regiment, file);
    let front = state.hostile.battle.formation.members[members[0]].id;
    let battle = &state.hostile.battle;
    if battle.ground_pending.contains(&front) || battle.ground_refused.contains(&front) {
        return Err("battle not ready: the slot's front soldier is not yet on its ground".into());
    }
    let at = state.entities.pos(
        state
            .entities
            .slot_of(front)
            .ok_or("the slot's front soldier is gone")?,
    );
    let spec = state.hostile.battle.formation.regiments[regiment].spec;
    let files = usize::from(spec.files);
    for index in members {
        let member = &mut state.hostile.battle.formation.members[index];
        member.rank += 1;
        let target = slot_position(&spec, usize::from(member.rank) * files + file);
        let id = member.id;
        if let Some(slot) = state.entities.slot_of(id) {
            let y = state.entities.pos(slot)[1];
            state.entities.set_pos(slot, [target[0], y, target[2]]);
            state.hostile.battle.ground_pending.push(id);
        }
    }
    super::staging::settle(state);
    Ok(at)
}

/// A respawned holder goes back into the rank with his kit again: before
/// start to where he was placed; once running to rank 0 of a current file of
/// his side near its centre (`choose_slot` tiers: standing infantry first).
fn reenter(state: &mut TickState, index: usize, player: EntityId) -> Result<String, String> {
    let hold = state.hostile.battle.live.holds[index].clone();
    let running = state.hostile.battle.running;
    let (regiment, file) = if running {
        choose_slot(state, hold.side, Some((hold.regiment, hold.file)))?
    } else {
        (hold.regiment, hold.file)
    };
    if (regiment, file) != (hold.regiment, hold.file) {
        let formation = &mut state.hostile.battle.formation;
        formation
            .held
            .retain(|held| *held != (hold.regiment, hold.file));
        formation.held.push((regiment, file));
        for member in file_members(formation, regiment, file) {
            formation.members[member].rank += 1;
        }
        let moved = &mut state.hostile.battle.live.holds[index];
        moved.regiment = regiment;
        moved.file = file;
    }
    let (source, kit) = (hold.kit_from, hold.kit.clone());
    let r = &state.hostile.battle.formation.regiments[regiment];
    let (at, yaw) = if running {
        let mut at = position(&r.spec, file, 0, interval(r));
        // Stand on the ground the file walks on.
        at[1] = file_members(&state.hostile.battle.formation, regiment, file)
            .into_iter()
            .find_map(|m| {
                let id = state.hostile.battle.formation.members[m].id;
                state.entities.slot_of(id).map(|slot| state.entities.pos(slot)[1])
            })
            .unwrap_or(hold.at[1]);
        if let Placement::At(ground) =
            ground::place(&state.world, &state.registry, at, PLAYER_HALF_WIDTH, PLAYER_HEIGHT)
        {
            at = ground;
        }
        (at, yaw_of(r.spec.facing))
    } else {
        (hold.at, hold.yaw)
    };
    let id = r.spec.id;
    let slots = equip(state, player, &kit);
    teleport(state, player, at, yaw, 0.0);
    Ok(format!(
        "Battle live respawned: {} side={} regiment={id} file={file} rank=0 at={:.2},{:.2},{:.2} yaw={yaw:.1} kit_slots={slots} kit_from={source} deaths={}",
        hold.name, hold.side, at[0], at[1], at[2], hold.deaths
    ))
}

/// `battle spectate <name> <target>`: spectator mode, standing behind the
/// target and looking his way (his rank's facing when he holds a slot).
/// The core has no Set Camera packet, so the camera is the spectator's own.
pub(crate) fn spectate(
    state: &mut TickState,
    changes: &mut ChangeSet,
    name: &str,
    target: &str,
) -> Result<String, String> {
    let viewer = player_named(state, name).ok_or_else(|| format!("{name} is not online"))?;
    let subject =
        player_named(state, target).ok_or_else(|| format!("{target} is not online"))?;
    if viewer == subject {
        return Err("a spectator cannot follow itself".into());
    }
    if is_dead(state, viewer) || is_dead(state, subject) {
        return Err(format!("{name} or {target} is dead; respawn first"));
    }
    let subject_slot = state.entities.slot_of(subject).expect("live player");
    let pos = state.entities.pos(subject_slot);
    let yaw = state
        .hostile
        .battle
        .live
        .holds
        .iter()
        .find(|hold| hold.name == target)
        .map_or_else(|| state.entities.rot(subject_slot)[0], |hold| hold.yaw);
    let radians = f64::from(yaw).to_radians();
    // Vanilla look vector of a yaw: (-sin, cos).
    let forward = [-radians.sin(), radians.cos()];
    let at = [
        pos[0] - forward[0] * SPECTATOR_BACK,
        pos[1] + SPECTATOR_UP,
        pos[2] - forward[1] * SPECTATOR_BACK,
    ];
    let command = Command::GameMode {
        target: Target::Entity(viewer),
        mode: GameMode::Spectator,
    };
    let effects = state
        .players
        .get_mut(&viewer)
        .expect("online player")
        .apply_command(&state.registry.item_rules(), &command);
    if let Some(connection) = state.connections.entity_connection(viewer) {
        crate::tick::apply_player_effects(state, connection, viewer, effects, changes);
    }
    teleport(state, viewer, at, yaw, SPECTATOR_PITCH);
    Ok(format!(
        "Battle spectator placed: {name} target={target} at={:.2},{:.2},{:.2} yaw={yaw:.1} gamemode=spectator camera=own",
        at[0], at[1], at[2]
    ))
}

#[cfg(test)]
#[path = "live_players_tests.rs"]
mod tests;
