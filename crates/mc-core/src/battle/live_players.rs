//! Live network players in a staged battle: a player who takes a front-rank
//! slot with that rank's kit, and spectators kept behind a player.
//! Console only (`battle watch|live|spectate`).
//!
//! Invariant: a held slot is rank 0 of one file of a foot regiment. The file's
//! soldiers stand from rank 1 back (`FormationRuntime::held`), so no soldier
//! steps into the holder's place. The holder is an ordinary player: he takes
//! damage and dies. The battle moves him only when he (re)enters the rank:
//! `battle live` before start, and each respawn, which puts him back into the
//! fight with the kit again: near where he fell, in his own regiment while it
//! stands, else in a standing line of his side in contact (`respawn_slot`).
//! A spectator bound to him (`battle spectate`) is placed behind him again
//! whenever he respawns or is re-armed.
//!
//! Battle-mode custody: a holder's inventory IS his kit (`equip`). Placement,
//! re-arming and every respawn set his whole inventory to exactly the kit:
//! his menus are closed first (cursor and crafting grid fold back, overflow is
//! discarded, never dropped), then every inventory slot is set, the kit in its
//! fixed slots and every other slot empty. Whatever he moved, picked up or
//! kept, after a respawn he holds the kit once and nothing else.
//! `keep_inventory` keeps the kit from littering the field when he dies.
//!
//! A living holder of the other side in weapon reach is one more candidate
//! for a soldier's own swing (`combat`): the same ready state, cadence and
//! Windup → impact → Recover cycle as a swing at a soldier, one target per
//! swing; wounded, staggered and recovering soldiers do not swing. The blow
//! lands through the ordinary player damage path (`blow`); battle targets
//! stay soldiers.
use super::combat::Weapon;
use super::{BattleIntent, BattleState, EnemyTarget, Frame, Output, Unit};
use crate::battle::formation::{
    FormationRuntime, Order, Regiment, file_interval, position, slot_position,
};
use crate::battle::ground::{self, Placement};
use crate::boundary::{ChangeSet, EntityId};
use crate::commands::{Command, GameMode, Target};
use crate::entities::{EntitySlot, EntityStore};
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
    /// Where he last stood alive; after a death, where he fell.
    pub alive_at: [f64; 3],
}

#[derive(Debug, Default)]
pub(crate) struct LiveState {
    watched: Vec<String>,
    /// Entity last announced online for each watched name.
    announced: Vec<Option<EntityId>>,
    pub holds: Vec<Hold>,
    /// (spectator, holder) of each `battle spectate`: the spectator goes
    /// behind the holder again on each respawn and re-arm (`follow`).
    spectators: Vec<(String, String)>,
    /// Living holders this tick (entity, side): candidates for enemy
    /// soldiers' swings (`in_reach`).
    targets: Vec<(EntityId, u16)>,
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
            let name = state.hostile.battle.live.holds[index].name.clone();
            follow(state, &name);
        }
        if !is_dead(state, player)
            && let Some(slot) = state.entities.slot_of(player)
        {
            let at = state.entities.pos(slot);
            let hold = &mut state.hostile.battle.live.holds[index];
            hold.alive_at = at;
            let side = hold.side;
            state.hostile.battle.live.targets.push((player, side));
        }
    }
}

/// The nearest living holder of the other side within the melee range of
/// soldier `actor` (`only`: that holder alone): the extra candidate a ready
/// soldier weighs against his soldier target before he starts a swing, and the
/// range check again at its impact.
pub(super) fn in_reach(
    battle: &BattleState,
    entities: &EntityStore,
    actor: EntitySlot,
    range: mc_melee_range::MeleeReach,
    only: Option<EntityId>,
) -> Option<EnemyTarget> {
    if battle.live.targets.is_empty() {
        return None;
    }
    let unit = super::faction::live_unit(entities, actor)?;
    battle
        .live
        .targets
        .iter()
        .filter(|&&(player, side)| {
            super::enemies(unit.faction, side) && only.is_none_or(|id| id == player)
        })
        .filter_map(|&(player, _)| {
            let slot = entities.slot_of(player)?;
            let pos = entities.pos(slot);
            super::faction::in_melee_range(entities, actor, slot, range).then_some(EnemyTarget {
                id: player,
                pos,
                distance: crate::target_grid::distance(unit.pos, pos),
            })
        })
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
}

pub(super) fn is_holder(battle: &BattleState, id: EntityId) -> bool {
    battle.live.targets.iter().any(|&(player, _)| player == id)
}

/// The impact of a soldier's swing at a holder (`combat`): an ordinary melee
/// hit through the player damage path (armour, shield block, knockback),
/// never the battle's outcome roll. A holder out of reach by now is missed.
pub(super) fn blow(
    frame: &Frame<'_>,
    battle: &BattleState,
    out: &mut Output,
    actor: EntityId,
    target: EntityId,
    range: mc_melee_range::MeleeReach,
) {
    let Some(slot) = frame.entities.slot_of(actor) else {
        return;
    };
    if in_reach(battle, frame.entities, slot, range, Some(target)).is_none() {
        return;
    }
    out.intents.push(BattleIntent {
        actor,
        kind: IntentKind::DamageEntity {
            target,
            amount: SOLDIER_HIT,
            kind: crate::survival::DamageKind::Melee,
            source: Some(DamageSource::Melee {
                attacker: actor,
                position: frame.entities.pos(slot),
                yaw: frame.entities.rot(slot)[0],
                bonus_knockback: 0.0,
                sweep: false,
                fire_ticks: 0,
                bane_ticks: 0,
            }),
        },
        footprint: Footprint {
            entity_reads: vec![actor, target],
            entity_writes: vec![target],
            ..Default::default()
        },
    });
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
                let pos = position(&r.spec, file, 0, file_interval(r));
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

/// Where the file's front living, unwounded soldier stands now.
fn front_soldier(state: &TickState, regiment: usize, file: usize) -> Option<[f64; 3]> {
    let formation = &state.hostile.battle.formation;
    file_members(formation, regiment, file)
        .into_iter()
        .map(|m| &formation.members[m])
        .filter(|m| m.alive && !m.wounded)
        .min_by_key(|m| m.rank)
        .and_then(|m| state.entities.slot_of(m.id))
        .map(|slot| state.entities.pos(slot))
}

/// Battle unit `bi` in contact as of the last battle tick: a melee fighter
/// whose retained opponent, or any enemy, stands within his melee range
/// (the `in_weapon_reach` census of `combat::count_fighting`).
fn in_contact(entities: &EntityStore, battle: &BattleState, bi: usize) -> bool {
    let columns = &battle.columns;
    let Some(fighter) = battle.combat.fighters.get(bi) else {
        return false;
    };
    if bi >= columns.ids.len()
        || !columns.active[bi]
        || columns.carried[bi]
        || fighter.weapon == Weapon::Bow
    {
        return false;
    }
    let Some(actor) = columns.slots[bi] else {
        return false;
    };
    let range = fighter.range;
    let reaches = |enemy: EntityId| {
        entities
            .slot_of(enemy)
            .is_some_and(|enemy| super::faction::in_melee_range(entities, actor, enemy, range))
    };
    let unit = Unit {
        id: columns.ids[bi],
        faction: columns.factions[bi],
        pos: columns.positions[bi],
    };
    columns.targets[bi].is_some_and(|target| reaches(target.id))
        || battle
            .grid
            .any_enemy(unit, super::faction::reach_bound(range), |u| reaches(u.id))
            .is_some()
}

/// Of `regiment`'s files nobody else holds (`keep` is the holder's own) that
/// still have a living front soldier, and are in contact when `contact` (a
/// soldier of the file has an enemy within his weapon reach), the one whose
/// front soldier stands nearest `at`, horizontally, with that squared
/// distance.
fn nearest_file(
    state: &TickState,
    regiment: usize,
    keep: (usize, usize),
    at: [f64; 3],
    contact: bool,
) -> Option<(f64, usize)> {
    let battle = &state.hostile.battle;
    let formation = &battle.formation;
    let r = &formation.regiments[regiment];
    let mut best: Option<(f64, usize)> = None;
    for file in 0..usize::from(r.spec.files).min(r.len) {
        if (regiment, file) != keep && formation.held.contains(&(regiment, file)) {
            continue;
        }
        if contact
            && !file_members(formation, regiment, file)
                .into_iter()
                .any(|bi| in_contact(&state.entities, battle, bi))
        {
            continue;
        }
        let Some(front) = front_soldier(state, regiment, file) else {
            continue;
        };
        let distance = (front[0] - at[0]).powi(2) + (front[2] - at[2]).powi(2);
        if best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, file));
        }
    }
    best
}

/// Where a respawn in a running battle puts the holder, and why (logged).
struct Pick {
    slot: (usize, usize),
    contact: bool,
    /// Why he left his own regiment, when he did.
    reason: Option<&'static str>,
    /// The old rule took the slot (`choose_slot`).
    centre: bool,
}

/// Trial-7: the centre rule put PovRed 230 blocks from the fight, with no
/// enemy near for four minutes. In this order, nearest where he fell: a file
/// of his own regiment in contact, broken or not; else, while it is not
/// broken, any living file of it; else (it is broken and out of contact,
/// `regiment_broken`: a fleeing crowd is no fight to film, or has no living
/// file left, `regiment_gone`) a file of his side's standing foot lines in
/// contact; else the old rule (`choose_slot`). He never changes regiment
/// without the reason in the log.
fn respawn_slot(state: &TickState, hold: &Hold) -> Result<Pick, String> {
    let keep = (hold.regiment, hold.file);
    let own = &state.hostile.battle.formation.regiments[hold.regiment];
    if let Some((_, file)) = nearest_file(state, hold.regiment, keep, hold.alive_at, true) {
        return Ok(Pick {
            slot: (hold.regiment, file),
            contact: true,
            reason: None,
            centre: false,
        });
    }
    let living = nearest_file(state, hold.regiment, keep, hold.alive_at, false);
    if let Some((_, file)) = living
        && eligible(state, own, hold.side, 1)
    {
        return Ok(Pick {
            slot: (hold.regiment, file),
            contact: false,
            reason: None,
            centre: false,
        });
    }
    let reason = if living.is_some() {
        "regiment_broken"
    } else {
        "regiment_gone"
    };
    let formation = &state.hostile.battle.formation;
    let contact = (0..formation.regiments.len())
        .filter(|&i| i != hold.regiment && eligible(state, &formation.regiments[i], hold.side, 1))
        .filter_map(|i| {
            nearest_file(state, i, keep, hold.alive_at, true).map(|(d, file)| (d, (i, file)))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    Ok(match contact {
        Some((_, slot)) => Pick {
            slot,
            contact: true,
            reason: Some(reason),
            centre: false,
        },
        None => Pick {
            slot: choose_slot(state, hold.side, Some(keep))?,
            contact: false,
            reason: Some(reason),
            centre: true,
        },
    })
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

/// Battle-mode custody (module doc): his whole inventory becomes exactly the
/// kit. Menus close first so cursor and crafting grid fold into the inventory;
/// what does not fit is discarded with the rest, never dropped on the field.
/// Then every slot is set: the kit in its fixed slots, all others empty.
fn equip(state: &mut TickState, id: EntityId, kit: &[Option<ItemStack>; 6]) -> usize {
    let rules = state.registry.item_rules();
    let player = state.players.get_mut(&id).expect("online player");
    let mut changes = player.close(&rules).changes;
    let selected = usize::from(player.inventory.selected.min(8));
    let mut image = vec![ItemStack::EMPTY; player.inventory.slots.len()];
    for (equipment, stack) in kit.iter().enumerate() {
        if let Some(stack) = stack {
            image[player_index(equipment, selected)] = stack.clone();
        }
    }
    let mut changed = 0_u64;
    for (index, stack) in image.into_iter().enumerate() {
        if player.inventory.slots[index] != stack {
            player.inventory.slots[index] = stack;
            changed |= 1 << index;
        }
    }
    changes.extend(player.pickup_changes(changed));
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
/// re-armed (a rejoin): the battle moves him only on respawn. Either way a
/// spectator bound to him goes behind him again.
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
                alive_at: at,
            });
            (at, yaw, source, kit)
        }
    };
    let slots = equip(state, player, &kit);
    let reply = if running {
        format!(
            "Battle live rearmed: {name} side={side} regiment={} file={file} kit_slots={slots} kit_from={source} moved=false",
            spec.id
        )
    } else {
        teleport(state, player, at, yaw, 0.0);
        format!(
            "Battle live placed: {name} side={side} regiment={} file={file} rank=0 at={:.2},{:.2},{:.2} yaw={yaw:.1} kit_slots={slots} kit_from={source}",
            spec.id, at[0], at[1], at[2]
        )
    };
    follow(state, name);
    Ok(reply)
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
/// start to where he was placed; once running to the front of the men of the
/// file `respawn_slot` picks.
fn reenter(state: &mut TickState, index: usize, player: EntityId) -> Result<String, String> {
    let hold = state.hostile.battle.live.holds[index].clone();
    let running = state.hostile.battle.running;
    let pick = if running {
        respawn_slot(state, &hold)?
    } else {
        Pick {
            slot: (hold.regiment, hold.file),
            contact: false,
            reason: None,
            centre: false,
        }
    };
    let (regiment, file) = pick.slot;
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
        // Where his file's men stand, not the slot of the virtual leader:
        // `spec.origin` walks on along the facing while men stay in melee or
        // at the field bounds (trial-5: the holder respawned at z=1855.8, at
        // least 1,500 blocks from every soldier the recorder saw). A block
        // ahead of the file's front living soldier (on his centre neither is
        // pushed and the camera sits inside him), else the regiment's living
        // centroid, else the leader's slot.
        let front = front_soldier(state, regiment, file)
            .map(|pos| [pos[0] + r.spec.facing[0], pos[1], pos[2] + r.spec.facing[1]]);
        let mut at = front.unwrap_or_else(|| {
            if r.live_count > 0 {
                r.live_sum.map(|sum| sum / r.live_count as f64)
            } else {
                position(&r.spec, file, 0, file_interval(r))
            }
        });
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
    let formation = &state.hostile.battle.formation;
    let from = formation.regiments[hold.regiment].spec.id;
    let changed = if regiment == hold.regiment {
        String::new()
    } else {
        format!(" regiment_changed={from}->{id}")
    };
    let reason = pick
        .reason
        .map_or(String::new(), |reason| format!(" reason={reason}"));
    let rule = if pick.centre { " rule=centre" } else { "" };
    let slots = equip(state, player, &kit);
    teleport(state, player, at, yaw, 0.0);
    Ok(format!(
        "Battle live respawned: {} side={} regiment={id} file={file} rank=0 at={:.2},{:.2},{:.2} yaw={yaw:.1} kit_slots={slots} kit_from={source} deaths={} contact={}{changed}{reason}{rule}",
        hold.name, hold.side, at[0], at[1], at[2], hold.deaths, pick.contact
    ))
}

/// `battle spectate <name> <target>`: spectator mode, standing behind the
/// target (`place_behind`). The binding stays: he goes behind the target
/// again whenever the target respawns or is re-armed (`follow`).
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
    let spectators = &mut state.hostile.battle.live.spectators;
    match spectators.iter_mut().find(|(bound, _)| bound == name) {
        Some(binding) => binding.1 = target.to_owned(),
        None => spectators.push((name.to_owned(), target.to_owned())),
    }
    place_behind(state, name, target)
}

/// Each spectator bound to `target` goes behind him again, wherever his
/// respawn or re-arm put him, so the spectator's client gets the chunks and
/// entities around him (trial-7: CamSpec stayed in the cave where PovRed
/// died, 230 blocks from his respawn, for four minutes).
fn follow(state: &mut TickState, target: &str) {
    for index in 0..state.hostile.battle.live.spectators.len() {
        let (name, bound) = &state.hostile.battle.live.spectators[index];
        if bound != target {
            continue;
        }
        let name = name.clone();
        match place_behind(state, &name, target) {
            Ok(line) => tracing::info!("{line}"),
            Err(error) => tracing::warn!("Battle spectator not placed: {error}"),
        }
    }
}

/// Stand spectator `name` behind `target`, looking his way: his line's
/// facing when he holds a slot. The core has no Set Camera packet, so the
/// camera is the spectator's own. A spectator who left spectator mode is not
/// moved.
fn place_behind(state: &mut TickState, name: &str, target: &str) -> Result<String, String> {
    let viewer = player_named(state, name).ok_or_else(|| format!("{name} is not online"))?;
    let subject = player_named(state, target).ok_or_else(|| format!("{target} is not online"))?;
    if is_dead(state, viewer) || is_dead(state, subject) {
        return Err(format!("{name} or {target} is dead; respawn first"));
    }
    if !state.players[&viewer].spectator() {
        return Err(format!("{name} left spectator mode"));
    }
    let subject_slot = state.entities.slot_of(subject).expect("live player");
    let pos = state.entities.pos(subject_slot);
    let battle = &state.hostile.battle;
    let yaw = battle
        .live
        .holds
        .iter()
        .find(|hold| hold.name == target)
        .map_or_else(
            || state.entities.rot(subject_slot)[0],
            |hold| {
                battle
                    .formation
                    .regiments
                    .get(hold.regiment)
                    .map_or(hold.yaw, |r| yaw_of(r.spec.facing))
            },
        );
    let radians = f64::from(yaw).to_radians();
    // Vanilla look vector of a yaw: (-sin, cos).
    let forward = [-radians.sin(), radians.cos()];
    let at = [
        pos[0] - forward[0] * SPECTATOR_BACK,
        pos[1] + SPECTATOR_UP,
        pos[2] - forward[1] * SPECTATOR_BACK,
    ];
    teleport(state, viewer, at, yaw, SPECTATOR_PITCH);
    Ok(format!(
        "Battle spectator placed: {name} target={target} at={:.2},{:.2},{:.2} yaw={yaw:.1} gamemode=spectator camera=own",
        at[0], at[1], at[2]
    ))
}

#[cfg(test)]
#[path = "live_players_tests.rs"]
mod tests;
