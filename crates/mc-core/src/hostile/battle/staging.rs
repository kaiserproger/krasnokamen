//! Operator-owned deterministic battle staging. No scenario file is read on the tick.
use crate::boundary::{ChangeSet, ChunkPos, EntityId};
use crate::commands::CommandError;
use crate::hostile::battle::formation::{Order, RegimentSpec, slot_position};
use crate::tick::TickState;
use mc_data::entity_types::{HORSE, HUSK};
use mc_data::item_nbt::{EQUIPPABLE_COMPONENT, ITEM_MODEL_COMPONENT};
use mc_data::items::{
    BOW, CHAINMAIL_CHESTPLATE, IRON_CHESTPLATE, IRON_HELMET, IRON_HORSE_ARMOR, IRON_SWORD,
    LEATHER_CHESTPLATE, SHIELD, TRIDENT,
};
use mc_nbt::{ListTag, Tag, tag_type};

pub(super) const OWNER: &str = "solaris:battle";
const TICKET_RADIUS: i32 = 12;
const MAX_SIDE: usize = 10_000;

#[derive(Clone, Debug, PartialEq)]
pub enum BattleCommand {
    Setup { seed: u64, count: usize },
    Start,
    Pause,
    Reset,
    Status,
    Roster,
}

pub fn parse(args: &[&str]) -> Result<BattleCommand, CommandError> {
    let usage = || {
        CommandError::Usage(
            "battle setup <seed> <field-100|field-1k|field-10k> [count] | battle start|pause|reset|status|roster",
        )
    };
    match args {
        ["setup", seed, preset, rest @ ..] if rest.len() <= 1 => {
            let seed = seed.parse::<u64>().map_err(|_| usage())?;
            let default = match *preset {
                "field-100" => 100,
                "field-1k" => 1_000,
                "field-10k" => 10_000,
                _ => return Err(usage()),
            };
            let count = match rest.first() {
                Some(n) => n.parse::<usize>().map_err(|_| usage())?,
                None => default,
            };
            if !(1..=MAX_SIDE).contains(&count) {
                return Err(usage());
            }
            Ok(BattleCommand::Setup { seed, count })
        }
        ["start"] => Ok(BattleCommand::Start),
        ["pause"] => Ok(BattleCommand::Pause),
        ["reset"] => Ok(BattleCommand::Reset),
        ["status"] => Ok(BattleCommand::Status),
        ["roster"] => Ok(BattleCommand::Roster),
        _ => Err(usage()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Infantry,
    Spear,
    Archer,
    Cavalry,
}

// Vanilla item identities retain their combat behaviour; components select authored visuals.
fn equipment(kind: Kind) -> &'static [(&'static str, &'static str)] {
    match kind {
        Kind::Infantry => &[
            ("mainhand", IRON_SWORD),
            ("offhand", SHIELD),
            ("head", IRON_HELMET),
            ("chest", IRON_CHESTPLATE),
        ],
        Kind::Spear => &[
            ("mainhand", TRIDENT),
            ("offhand", SHIELD),
            ("head", IRON_HELMET),
            ("chest", CHAINMAIL_CHESTPLATE),
        ],
        Kind::Archer => &[
            ("mainhand", BOW),
            ("head", IRON_HELMET),
            ("chest", LEATHER_CHESTPLATE),
        ],
        Kind::Cavalry => &[
            ("mainhand", IRON_SWORD),
            ("offhand", SHIELD),
            ("head", IRON_HELMET),
            ("chest", IRON_CHESTPLATE),
        ],
    }
}

/// Homogeneous rank-major regiments: foot in front, archers behind, horse on
/// the two flanks. Counts are exact even when the requested side is small.
fn plan(seed: u64, count: usize, origin: [f64; 3], side: u16) -> Vec<(RegimentSpec, Kind, usize)> {
    let cavalry = count / 10;
    let archers = count / 5;
    let spears = count / 5;
    let infantry = count - cavalry - archers - spears;
    let mut front = Vec::new();
    for (kind, mut left) in [(Kind::Infantry, infantry), (Kind::Spear, spears)] {
        while left > 0 {
            let size = left.min(1000);
            front.push((kind, size));
            left -= size;
        }
    }
    if seed & 1 != 0 {
        front.reverse();
    }
    let direction = if side == 1 { 1.0 } else { -1.0 };
    let facing = [0.0, direction];
    let mut result = Vec::new();
    let mut add = |kind: Kind, size: usize, x: f64, back: f64, files: usize| {
        if size == 0 {
            return;
        }
        let id = (u32::from(side) - 1) * 100 + result.len() as u32;
        let spec = RegimentSpec {
            id,
            faction: side,
            origin: [origin[0] + x, origin[1], origin[2] - direction * back],
            facing,
            files: size.min(files) as u16,
            frontage: if kind == Kind::Cavalry { 2.0 } else { 1.0 },
            order: if kind == Kind::Archer {
                Order::Hold
            } else {
                Order::Advance
            },
        };
        result.push((spec, kind, size));
    };
    // Fit 322 front files inside the existing ticket even at a chunk edge;
    // mounted files remain on the flanks, just behind the shallower foot.
    let flank = front.len().saturating_sub(1) as f64 * 23.0 + 4.0;
    add(Kind::Cavalry, cavalry / 2, -flank, 84.0, 20);
    for (index, &(kind, size)) in front.iter().enumerate() {
        let x = (index as f64 - (front.len() - 1) as f64 / 2.0) * 46.0;
        add(kind, size, x, 36.0, 46);
    }
    let archer_regiments = archers.div_ceil(1000);
    for index in 0..archer_regiments {
        let size = (archers - index * 1000).min(1000);
        let x = (index as f64 - (archer_regiments - 1) as f64 / 2.0) * 52.0;
        add(Kind::Archer, size, x, 120.0, 50);
    }
    add(Kind::Cavalry, cavalry - cavalry / 2, flank, 84.0, 20);
    result
}

/// Pursue routed opponents without disabling the pulse-based fight beforehand.
pub(super) fn update_orders(formation: &mut crate::hostile::battle::formation::FormationRuntime) {
    for index in 0..formation.regiments.len() {
        let regiment = &formation.regiments[index];
        let order = match (
            regiment.spec.order,
            regiment.enemy_routing,
            regiment.morale.broken,
        ) {
            (Order::Advance, true, false) => Some(Order::Pursue),
            (Order::Pursue, false, _) | (Order::Pursue, _, true) => Some(Order::Advance),
            _ => None,
        };
        if let Some(order) = order {
            let id = regiment.spec.id;
            formation.set_order(id, order);
        }
    }
}

fn tag_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    let tag = Tag::Compound(vec![(
        "Tags".into(),
        Tag::List(ListTag {
            element_type: tag_type::STRING,
            elements: vec![Tag::String(OWNER.into())],
        }),
    )]);
    mc_nbt::write_named(&mut bytes, "", &tag).expect("writing battle owner tag to memory");
    bytes
}

fn owned(bytes: &[u8]) -> bool {
    let mut input = bytes;
    let Ok((_, Tag::Compound(fields))) = mc_nbt::read_named(&mut input) else {
        return false;
    };
    fields.iter().any(|(name, value)| name == "Tags" && matches!(value, Tag::List(list) if list.elements.iter().any(|tag| matches!(tag, Tag::String(name) if name == OWNER))))
}

fn tagged_ids(state: &TickState) -> Vec<EntityId> {
    state
        .entities
        .slots()
        .filter_map(|slot| {
            let body = state.entities.body(slot);
            (matches!(body.kind, crate::entities::EntityKind::Mob { .. })
                && owned(&state.entities.row(slot).opaque_entity_fields))
            .then_some(body.id)
        })
        .collect()
}

fn reset(state: &mut TickState, changes: &mut ChangeSet) -> usize {
    let ids = tagged_ids(state);
    for id in &ids {
        if let Some(slot) = state.entities.slot_of_any(*id) {
            state.remove_entity(slot);
            changes.entities_removed.push(*id);
        }
    }
    state.tickets.remove(OWNER);
    state.hostile.battle = Default::default();
    ids.len()
}

fn summon_tag(kind: Option<Kind>, side: u16) -> Tag {
    let mut fields = vec![("PersistenceRequired".into(), Tag::Byte(1))];
    let items = kind.map_or(&[("body", IRON_HORSE_ARMOR)][..], equipment);
    let slots = items
        .iter()
        .map(|&(slot, item)| {
            let component = match (kind, slot, side) {
                (Some(Kind::Infantry), "mainhand", _) => {
                    (ITEM_MODEL_COMPONENT, "solaris_battle:iron_longsword")
                }
                (Some(Kind::Spear), "mainhand", _) => {
                    (ITEM_MODEL_COMPONENT, "solaris_battle:iron_spear")
                }
                (Some(Kind::Archer), "mainhand", _) => {
                    (ITEM_MODEL_COMPONENT, "solaris_battle:iron_longbow_standby")
                }
                (Some(Kind::Cavalry), "mainhand", _) => {
                    (ITEM_MODEL_COMPONENT, "solaris_battle:iron_lance")
                }
                (_, "offhand", 1) => (ITEM_MODEL_COMPONENT, "solaris_battle:red_heater_shield"),
                (_, "offhand", _) => (ITEM_MODEL_COMPONENT, "solaris_battle:blue_heater_shield"),
                (Some(Kind::Cavalry), "head", _) => {
                    (EQUIPPABLE_COMPONENT, "solaris_battle:crested_helm")
                }
                (_, "head", _) => (EQUIPPABLE_COMPONENT, "solaris_battle:iron_plate"),
                (_, "chest", 1) => (EQUIPPABLE_COMPONENT, "solaris_battle:red_surcoat"),
                (_, "chest", _) => (EQUIPPABLE_COMPONENT, "solaris_battle:blue_surcoat"),
                (_, "body", 1) => (EQUIPPABLE_COMPONENT, "solaris_battle:red_caparison"),
                (_, "body", _) => (EQUIPPABLE_COMPONENT, "solaris_battle:blue_caparison"),
                _ => unreachable!("battle equipment table contains an unknown slot"),
            };
            let value = if component.0 == EQUIPPABLE_COMPONENT {
                Tag::Compound(vec![
                    ("slot".into(), Tag::String(slot.into())),
                    ("asset_id".into(), Tag::String(component.1.into())),
                ])
            } else {
                Tag::String(component.1.into())
            };
            (
                slot.into(),
                Tag::Compound(vec![
                    ("id".into(), Tag::String(item.into())),
                    (
                        "components".into(),
                        Tag::Compound(vec![(component.0.into(), value)]),
                    ),
                ]),
            )
        })
        .collect();
    fields.push(("equipment".into(), Tag::Compound(slots)));
    Tag::Compound(fields)
}

fn spawn(
    state: &mut TickState,
    changes: &mut ChangeSet,
    name: &str,
    position: [f64; 3],
    unit: Option<Kind>,
    loadout: &Tag,
    tag: &[u8],
) -> Result<EntityId, String> {
    let identifier = mc_data::Identifier::parse(name.to_owned()).map_err(|e| e.to_string())?;
    let facts = state
        .registry
        .entity_types
        .facts_of(&identifier)
        .ok_or_else(|| format!("unknown battle entity {name}"))?;
    let type_id = facts.protocol_id;
    let image = crate::command_summon::prepare(state, type_id, name, position, loadout)
        .map_err(|e| e.message())?;
    let health = state
        .registry
        .blueprint(type_id)
        .map_or(20.0, |b| b.params.max_health);
    let id = state.spawn_mob(type_id, position, health);
    crate::command_summon::apply(state, id, image);
    let slot = state.entities.slot_of(id).expect("spawned battle entity");
    let row = state.entities.row_mut(slot);
    row.opaque_entity_fields = tag.to_vec();
    if unit == Some(Kind::Archer) {
        row.attack_reach = 220.0;
    }
    changes.entities_spawned.push(id);
    Ok(id)
}

fn setup(
    state: &mut TickState,
    changes: &mut ChangeSet,
    origin: [f64; 3],
    seed: u64,
    count: usize,
) -> Result<(), String> {
    if !tagged_ids(state).is_empty() || state.tickets.contains_key(OWNER) {
        return Err("battle already staged; reset it first".into());
    }
    if state.difficulty == crate::survival::Difficulty::Peaceful {
        return Err("battle requires a non-peaceful world: peaceful removes hostile units".into());
    }
    if !origin.iter().all(|n| n.is_finite())
        || origin[0].abs() > 29_999_000.0
        || origin[2].abs() > 29_999_000.0
    {
        return Err("battle origin outside safe world bounds".into());
    }
    let tag = tag_bytes();
    let centre = ChunkPos {
        x: (origin[0].floor() as i32).div_euclid(16),
        z: (origin[2].floor() as i32).div_euclid(16),
    };
    let spawned_before = changes.entities_spawned.len();
    let removed_before = changes.entities_removed.len();
    state.tickets.insert(OWNER.into(), (centre, TICKET_RADIUS));
    let result = (|| {
        for side in 1..=2_u16 {
            let horse_loadout = summon_tag(None, side);
            for (spec, kind, size) in plan(seed, count, origin, side) {
                let loadout = summon_tag(Some(kind), side);
                let mut members = Vec::with_capacity(size);
                for index in 0..size {
                    let pos = slot_position(&spec, index);
                    let id = spawn(state, changes, HUSK, pos, Some(kind), &loadout, &tag)?;
                    let slot = state.entities.slot_of(id).expect("battle unit");
                    state.entities.set_faction(slot, side);
                    if kind == Kind::Cavalry {
                        let horse = spawn(state, changes, HORSE, pos, None, &horse_loadout, &tag)?;
                        crate::hostile::battle::cavalry::mount(state, id, horse)
                            .map_err(|e| format!("battle mount: {e}"))?;
                    }
                    members.push(id);
                }
                state
                    .hostile
                    .battle
                    .formation
                    .add_regiment(spec, &members)
                    .map_err(str::to_owned)?;
            }
        }
        state
            .hostile
            .battle
            .formation
            .set_bounds(
                [
                    f64::from((centre.x - TICKET_RADIUS) * 16 + 16),
                    f64::from((centre.z - TICKET_RADIUS) * 16 + 16),
                ],
                [
                    f64::from((centre.x + TICKET_RADIUS + 1) * 16 - 16),
                    f64::from((centre.z + TICKET_RADIUS + 1) * 16 - 16),
                ],
            )
            .map_err(str::to_owned)?;
        Ok(())
    })();
    if result.is_err() {
        reset(state, changes);
        changes.entities_spawned.truncate(spawned_before);
        changes.entities_removed.truncate(removed_before);
    }
    if result.is_ok() {
        for name in ["mob_drops", "mob_griefing", "spawn_mobs"] {
            if state.gamerules.get(name).map(String::as_str) != Some("false") {
                state.gamerules.insert(name.into(), "false".into());
                state.gamerules_dirty = true;
            }
        }
    }
    result
}

pub(crate) fn execute(
    state: &mut TickState,
    changes: &mut ChangeSet,
    origin: [f64; 3],
    command: &BattleCommand,
) -> Result<String, String> {
    match command {
        BattleCommand::Setup { seed, count } => {
            setup(state, changes, origin, *seed, *count)?;
            Ok(format!("Battle staged: {count} per side, seed {seed}"))
        }
        BattleCommand::Start => {
            if state.hostile.battle.formation.regiments.is_empty() {
                return Err("no staged formation; setup required".into());
            }
            let waiting = tagged_ids(state)
                .into_iter()
                .filter(|id| {
                    let Some(slot) = state.entities.slot_of(*id) else {
                        return false;
                    };
                    let pos = state.entities.pos(slot);
                    let x = (pos[0].floor() as i32).div_euclid(16);
                    let z = (pos[2].floor() as i32).div_euclid(16);
                    !state.world.is_active_position(x, z)
                })
                .count();
            if waiting != 0 {
                return Err(format!(
                    "battle not ready: {waiting} entities outside loaded simulation"
                ));
            }
            state.hostile.battle.running = true;
            Ok(format!(
                "Battle started tick={}",
                crate::command_context::execution_tick(state).saturating_add(1)
            ))
        }
        BattleCommand::Pause => {
            state.hostile.battle.running = false;
            Ok(format!(
                "Battle paused tick={}",
                crate::command_context::execution_tick(state).saturating_add(1)
            ))
        }
        BattleCommand::Reset => Ok(format!(
            "Battle reset: {} entities removed",
            reset(state, changes)
        )),
        BattleCommand::Roster => {
            use std::fmt::Write;
            let ids = tagged_ids(state);
            if ids.is_empty() {
                return Err("no staged battle roster".into());
            }
            let mut result = String::with_capacity(15 + ids.len() * 37);
            result.push_str("Battle roster: ");
            for (index, id) in ids.iter().enumerate() {
                if index != 0 {
                    result.push(',');
                }
                let slot = state.entities.slot_of_any(*id).expect("tagged entity");
                write!(&mut result, "{}", state.entities.row(slot).uuid).expect("write to string");
            }
            Ok(result)
        }
        BattleCommand::Status => {
            let mut sides = [0_usize; 2];
            let (mut resident, mut active, mut moving, mut horses, mut dead) = (0, 0, 0, 0, 0);
            for id in tagged_ids(state) {
                let Some(slot) = state.entities.slot_of_any(id) else {
                    continue;
                };
                let body = state.entities.body(slot);
                let equine = crate::mounts::type_id(body)
                    .and_then(|id| state.registry.blueprint(id))
                    .is_some_and(|blueprint| blueprint.capabilities.taming.equine());
                if equine {
                    horses += 1;
                    continue;
                }
                let faction = body.faction;
                if !(1..=2).contains(&faction) {
                    continue;
                }
                if !state.entities.body(slot).alive || state.entities.row(slot).health <= 0.0 {
                    dead += 1;
                    continue;
                }
                sides[usize::from(faction - 1)] += 1;
                let pos = state.entities.pos(slot);
                let x = (pos[0].floor() as i32).div_euclid(16);
                let z = (pos[2].floor() as i32).div_euclid(16);
                resident += usize::from(state.world.has_chunk(x, z));
                active += usize::from(state.world.is_active_position(x, z));
                moving += usize::from(state.entities.vel(slot).iter().any(|v| *v != 0.0));
            }
            let sample_tick = crate::command_context::execution_tick(state).saturating_add(1);
            Ok(format!(
                "Battle {} tick={} side1={} side2={} resident={} active={} nonzero_velocity={} horses={} regiments={} dead={}",
                if state.hostile.battle.running {
                    "running"
                } else {
                    "paused"
                },
                sample_tick,
                sides[0],
                sides[1],
                resident,
                active,
                moving,
                horses,
                state.hostile.battle.formation.regiments.len(),
                dead
            ))
        }
    }
}

#[cfg(test)]
#[path = "staging_tests.rs"]
mod tests;
