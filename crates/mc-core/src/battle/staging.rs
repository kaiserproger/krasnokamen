//! Operator-owned deterministic battle staging. No scenario file is read on the tick.
use crate::battle::formation::{MESH_INTERVAL, Order, RegimentSpec, slot_position};
use crate::battle::ground::{self, Placement};
use crate::battle::live_players;
use crate::boundary::{ChangeSet, ChunkPos, EntityId};
use crate::commands::CommandError;
use crate::tick::TickState;
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
    /// Live network players (live_players.rs).
    Watch { names: Vec<String> },
    Live { name: String, side: u16 },
    Spectate { name: String, target: String },
}

pub fn parse(args: &[&str]) -> Result<BattleCommand, CommandError> {
    let usage = || {
        CommandError::Usage(
            "battle setup <seed> <field-100|field-1k|field-10k> [count] | battle start|pause|reset|status|roster | battle watch <player>... | battle live <player> <red|blue> | battle spectate <player> <target>",
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
        ["watch", names @ ..] => {
            let names: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
            if !live_players::watch_count_ok(&names)
                || !names.iter().all(|n| live_players::valid_name(n))
            {
                return Err(usage());
            }
            Ok(BattleCommand::Watch { names })
        }
        ["live", name, side] if live_players::valid_name(name) => Ok(BattleCommand::Live {
            name: (*name).to_owned(),
            side: live_players::parse_side(side).ok_or_else(usage)?,
        }),
        ["spectate", name, target]
            if live_players::valid_name(name) && live_players::valid_name(target) =>
        {
            Ok(BattleCommand::Spectate {
                name: (*name).to_owned(),
                target: (*target).to_owned(),
            })
        }
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

// Vanilla item identities retain their combat behaviour (weapon class, shield
// blocking, armour points); components select authored visuals.
fn equipment(kind: Kind) -> &'static [(&'static str, &'static str)] {
    match kind {
        Kind::Infantry => &[
            ("mainhand", "minecraft:iron_sword"),
            ("offhand", "minecraft:shield"),
            ("head", "minecraft:iron_helmet"),
            ("chest", "minecraft:iron_chestplate"),
        ],
        Kind::Spear => &[
            ("mainhand", "minecraft:trident"),
            ("offhand", "minecraft:shield"),
            ("head", "minecraft:iron_helmet"),
            ("chest", "minecraft:chainmail_chestplate"),
        ],
        Kind::Archer => &[
            ("mainhand", "minecraft:bow"),
            ("head", "minecraft:iron_helmet"),
            ("chest", "minecraft:leather_chestplate"),
        ],
        Kind::Cavalry => &[
            ("mainhand", "minecraft:iron_sword"),
            ("offhand", "minecraft:shield"),
            ("head", "minecraft:iron_helmet"),
            ("chest", "minecraft:iron_chestplate"),
        ],
    }
}

/// Worn leggings are cosmetic: `minecraft:leather` has no armour modifier, so
/// adding legs never changes the damage a fighter takes.
const LEGS_CARRIER: &str = "minecraft:leather";
/// Dyed leather shades of each side colour (`minecraft:dyed_color` RGB).
const RED_SHADES: [i32; 4] = [0x00B0_2E26, 0x008E_1B1B, 0x00C8_452E, 0x006E_1414];
const BLUE_SHADES: [i32; 4] = [0x003C_44AA, 0x0024_397F, 0x004F_6BC4, 0x001E_2A66];
/// Helmet plume tint: the pack's crested_helm and iron_plate carry their crest or
/// plume as a dyeable layer (undyed red), so side 2 must dye it blue.
const RED_PLUME: i32 = 0x00B9_2529;
const BLUE_PLUME: i32 = 0x0029_4DB7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shield {
    /// Side-coloured red or blue heater.
    Heater,
    Kite,
    Tower,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Torso {
    Surcoat,
    /// Vanilla leather dyed in a shade of the side colour.
    Leather,
    /// Neutral; only worn behind a side-coloured heater shield.
    Holy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Legs {
    Bare,
    Leather,
    Holy,
}

/// One fighter's appearance. Side colour stays readable on a wide shot: the
/// torso is a side surcoat or side-dyed leather, or the off hand carries the
/// side heater shield (holy torso only behind a heater).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Loadout {
    kind: Kind,
    weapon: &'static str,
    shield: Option<Shield>,
    helmet: &'static str,
    torso: Torso,
    legs: Legs,
    shade: usize,
}

/// SplitMix64 finaliser: a fixed, platform-independent mix so retakes repeat.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Pick the variant whose cumulative percentage band contains the next draw.
fn band<T: Copy>(roll: &mut u64, table: &[(u64, T)]) -> T {
    let draw = *roll % 100;
    *roll /= 100;
    let mut upper = 0;
    for &(share, value) in table {
        upper += share;
        if draw < upper {
            return value;
        }
    }
    unreachable!("loadout table shares must sum to 100")
}

/// Deterministic per-fighter loadout from (seed, side, regiment, index).
fn loadout(seed: u64, side: u16, regiment: u32, index: usize, kind: Kind) -> Loadout {
    let mut roll =
        mix(mix(seed) ^ (u64::from(side) << 56) ^ (u64::from(regiment) << 24) ^ index as u64);
    let weapon = match kind {
        Kind::Infantry => band(
            &mut roll,
            &[
                (60, "solaris_battle:iron_longsword"),
                (40, "solaris_battle:iron_flanged_mace"),
            ],
        ),
        Kind::Spear => band(
            &mut roll,
            &[
                (60, "solaris_battle:iron_spear"),
                (40, "solaris_battle:iron_halberd"),
            ],
        ),
        Kind::Archer => band(
            &mut roll,
            &[
                (60, "solaris_battle:iron_longbow_standby"),
                (40, "solaris_battle:iron_heavy_crossbow_standby"),
            ],
        ),
        Kind::Cavalry => "solaris_battle:iron_lance",
    };
    let shield = match kind {
        Kind::Archer => None,
        // A tower shield does not fit a rider.
        Kind::Cavalry => Some(band(&mut roll, &[(60, Shield::Heater), (40, Shield::Kite)])),
        Kind::Infantry | Kind::Spear => Some(band(
            &mut roll,
            &[
                (50, Shield::Heater),
                (30, Shield::Kite),
                (20, Shield::Tower),
            ],
        )),
    };
    let helmet = band(
        &mut roll,
        &[
            (45, "solaris_battle:iron_plate"),
            (30, "solaris_battle:crested_helm"),
            (25, "solaris_battle:holy"),
        ],
    );
    let torso = if shield == Some(Shield::Heater) {
        band(
            &mut roll,
            &[
                (45, Torso::Surcoat),
                (35, Torso::Leather),
                (20, Torso::Holy),
            ],
        )
    } else {
        band(&mut roll, &[(55, Torso::Surcoat), (45, Torso::Leather)])
    };
    let legs = band(
        &mut roll,
        &[(35, Legs::Bare), (35, Legs::Leather), (30, Legs::Holy)],
    );
    Loadout {
        kind,
        weapon,
        shield,
        helmet,
        torso,
        legs,
        shade: (roll % 4) as usize,
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
    // Seven 36-file foot regiments (28 ranks) one fighting width apart: at the
    // mesh interval the line is one uniform 252-file lattice (353 blocks,
    // inside the existing ticket even at a chunk edge). The second side stands
    // half an interval to the side, so the two lines mesh file between file.
    // Mounted files remain on the flanks, behind the deeper foot.
    let width = 36.0 * MESH_INTERVAL;
    let flank = front.len().saturating_sub(1) as f64 * width / 2.0 + 4.0;
    let mesh = if side == 2 { MESH_INTERVAL / 2.0 } else { 0.0 };
    add(Kind::Cavalry, cavalry / 2, -flank, 96.0, 20);
    for (index, &(kind, size)) in front.iter().enumerate() {
        let x = (index as f64 - (front.len() - 1) as f64 / 2.0) * width + mesh;
        add(kind, size, x, 36.0, 36);
    }
    let archer_regiments = archers.div_ceil(1000);
    for index in 0..archer_regiments {
        let size = (archers - index * 1000).min(1000);
        let x = (index as f64 - (archer_regiments - 1) as f64 / 2.0) * 52.0;
        add(Kind::Archer, size, x, 120.0, 50);
    }
    add(Kind::Cavalry, cavalry - cavalry / 2, flank, 96.0, 20);
    result
}

/// Pursue routed opponents without disabling the pulse-based fight beforehand.
pub(super) fn update_orders(formation: &mut crate::battle::formation::FormationRuntime) {
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

fn worn(slot: &str, asset: &str) -> (String, Tag) {
    (
        "minecraft:equippable".into(),
        Tag::Compound(vec![
            ("slot".into(), Tag::String(slot.into())),
            ("asset_id".into(), Tag::String(asset.into())),
        ]),
    )
}

fn item_model(model: &str) -> (String, Tag) {
    ("minecraft:item_model".into(), Tag::String(model.into()))
}

/// `None` is a horse's caparison; otherwise one fighter's loadout.
fn summon_tag(unit: Option<&Loadout>, side: u16) -> Tag {
    let red = side == 1;
    let coloured = |r: &'static str, b: &'static str| if red { r } else { b };
    let mut slots: Vec<(&str, &str, Vec<(String, Tag)>)> = Vec::new();
    match unit {
        None => slots.push((
            "body",
            "minecraft:iron_horse_armor",
            vec![worn(
                "body",
                coloured(
                    "solaris_battle:red_caparison",
                    "solaris_battle:blue_caparison",
                ),
            )],
        )),
        Some(unit) => {
            let dyed = (
                "minecraft:dyed_color".to_owned(),
                Tag::Int(coloured_shade(red, unit.shade)),
            );
            for &(slot, item) in equipment(unit.kind) {
                let components = match slot {
                    "mainhand" => vec![item_model(unit.weapon)],
                    "offhand" => vec![item_model(match unit.shield {
                        Some(Shield::Heater) | None => coloured(
                            "solaris_battle:red_heater_shield",
                            "solaris_battle:blue_heater_shield",
                        ),
                        Some(Shield::Kite) => "solaris_battle:iron_kite_shield",
                        Some(Shield::Tower) => "solaris_battle:iron_tower_shield",
                    })],
                    "head" => vec![
                        worn("head", unit.helmet),
                        (
                            "minecraft:dyed_color".to_owned(),
                            Tag::Int(if red { RED_PLUME } else { BLUE_PLUME }),
                        ),
                    ],
                    "chest" => match unit.torso {
                        Torso::Surcoat => vec![worn(
                            "chest",
                            coloured("solaris_battle:red_surcoat", "solaris_battle:blue_surcoat"),
                        )],
                        Torso::Leather => vec![worn("chest", "minecraft:leather"), dyed.clone()],
                        Torso::Holy => vec![worn("chest", "solaris_battle:holy")],
                    },
                    _ => unreachable!("battle equipment table contains an unknown slot"),
                };
                slots.push((slot, item, components));
            }
            match unit.legs {
                Legs::Bare => {}
                Legs::Leather => slots.push((
                    "legs",
                    LEGS_CARRIER,
                    vec![worn("legs", "minecraft:leather"), dyed],
                )),
                Legs::Holy => slots.push((
                    "legs",
                    LEGS_CARRIER,
                    vec![worn("legs", "solaris_battle:holy")],
                )),
            }
        }
    }
    let slots = slots
        .into_iter()
        .map(|(slot, item, components)| {
            (
                slot.into(),
                Tag::Compound(vec![
                    ("id".into(), Tag::String(item.into())),
                    ("components".into(), Tag::Compound(components)),
                ]),
            )
        })
        .collect();
    Tag::Compound(vec![
        ("PersistenceRequired".into(), Tag::Byte(1)),
        ("equipment".into(), Tag::Compound(slots)),
    ])
}

fn coloured_shade(red: bool, shade: usize) -> i32 {
    if red {
        RED_SHADES[shade]
    } else {
        BLUE_SHADES[shade]
    }
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
                let yaw = facing_yaw(spec.facing);
                let mut members = Vec::with_capacity(size);
                for index in 0..size {
                    let unit = loadout(seed, side, spec.id, index, kind);
                    let pos = slot_position(&spec, index);
                    let id = spawn(
                        state,
                        changes,
                        "minecraft:husk",
                        pos,
                        Some(kind),
                        &summon_tag(Some(&unit), side),
                        &tag,
                    )?;
                    let slot = state.entities.slot_of(id).expect("battle unit");
                    state.entities.set_faction(slot, side);
                    state.entities.set_rot(slot, yaw, 0.0);
                    // Staged ranks already stand guard: shields go up with the
                    // spawn, so `battle start` publishes no mass pose change.
                    if unit.shield.is_some() {
                        super::show_shield(state, id, true);
                    }
                    // A horse carries its rider: settle the pair through the horse.
                    let mut grounded = id;
                    if kind == Kind::Cavalry {
                        let horse = spawn(
                            state,
                            changes,
                            "minecraft:horse",
                            pos,
                            None,
                            &horse_loadout,
                            &tag,
                        )?;
                        crate::battle::cavalry::mount(state, id, horse)
                            .map_err(|e| format!("battle mount: {e}"))?;
                        let horse_slot = state.entities.slot_of(horse).expect("battle horse");
                        state.entities.set_rot(horse_slot, yaw, 0.0);
                        grounded = horse;
                    }
                    state.hostile.battle.ground_pending.push(grounded);
                    members.push(id);
                }
                let foot = matches!(kind, Kind::Infantry | Kind::Spear);
                let formation = &mut state.hostile.battle.formation;
                formation
                    .add_regiment(spec, &members)
                    .map_err(str::to_owned)?;
                if foot {
                    formation
                        .set_fighting_interval(spec.id, MESH_INTERVAL)
                        .map_err(str::to_owned)?;
                }
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
        settle(state);
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

/// Vanilla yaw of a horizontal facing (0 = +z, 90 = -x), as mob look uses.
fn facing_yaw(facing: [f64; 2]) -> f32 {
    -(mc_physics::mth::atan2(facing[0], facing[1]) as f32) * mc_physics::mth::RAD_TO_DEG
}

/// Stand pending staged bodies on the ground of their footprint, normally
/// once every chunk their search ring can read is resident, so placement
/// depends on terrain, not chunk arrival order. A body whose own chunk is
/// already simulated is placed at once from the resident columns: physics
/// (and suffocation) must never see its provisional height.
pub(super) fn settle(state: &mut TickState) {
    let pending = std::mem::take(&mut state.hostile.battle.ground_pending);
    let mut waiting = Vec::new();
    for id in pending {
        let Some(slot) = state.entities.slot_of(id) else {
            continue;
        };
        let pos = state.entities.pos(slot);
        let reach = f64::from(ground::SEARCH_RADIUS) + 2.0;
        let resident = [-reach, reach].iter().all(|dx| {
            [-reach, reach].iter().all(|dz| {
                state.world.has_chunk(
                    ((pos[0] + dx).floor() as i32).div_euclid(16),
                    ((pos[2] + dz).floor() as i32).div_euclid(16),
                )
            })
        });
        let own = (
            (pos[0].floor() as i32).div_euclid(16),
            (pos[2].floor() as i32).div_euclid(16),
        );
        if !resident && !state.world.is_active_position(own.0, own.1) {
            waiting.push(id);
            continue;
        }
        let body = *state.entities.body(slot);
        let horse = state.entities.row(slot).entity_life.mount.rider.is_some();
        let height = if horse {
            ground::MOUNTED_CLEARANCE
        } else {
            body.height
        };
        match ground::place(&state.world, &state.registry, pos, body.half_width, height) {
            Placement::At(at) => {
                state.entities.set_pos(slot, at);
                if horse {
                    crate::mounts::sync_rider(state, slot);
                }
            }
            Placement::Wait => waiting.push(id),
            Placement::Nowhere => state.hostile.battle.ground_refused.push(id),
        }
    }
    state.hostile.battle.ground_pending = waiting;
}

/// Standing staged bodies (riders sit on horses) against their terrain.
fn ground_census(state: &TickState) -> ground::Census {
    let mut census = ground::Census::default();
    for id in tagged_ids(state) {
        let Some(slot) = state.entities.slot_of(id) else {
            continue;
        };
        let body = state.entities.body(slot);
        if !body.alive || state.entities.row(slot).entity_life.mount.vehicle.is_some() {
            continue;
        }
        ground::audit(
            &state.world,
            &state.registry,
            state.entities.pos(slot),
            body.half_width,
            body.height,
            &mut census,
        );
    }
    census
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
            settle(state);
            let battle = &state.hostile.battle;
            if !battle.ground_pending.is_empty() || !battle.ground_refused.is_empty() {
                return Err(format!(
                    "battle not ready: {} staged bodies await ground, {} have no dry clear ground within {} blocks of their slot; stage on drier ground",
                    battle.ground_pending.len(),
                    battle.ground_refused.len(),
                    ground::SEARCH_RADIUS
                ));
            }
            let census = ground_census(state);
            state.hostile.battle.running = true;
            Ok(format!(
                "Battle started tick={} ground: bodies={} inside={} floating={} wet={}",
                crate::command_context::execution_tick(state).saturating_add(1),
                census.bodies,
                census.inside,
                census.floating,
                census.wet
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
            let battle = &state.hostile.battle;
            // Melee fronts within contact range of their enemy front.
            let engaged = battle
                .formation
                .regiments
                .iter()
                .filter(|r| {
                    r.spec.order != Order::Hold
                        && r.enemy.is_some()
                        && r.gap <= 6.0
                        && r.live_count > 0
                })
                .count();
            Ok(format!(
                "Battle {} tick={} side1={} side2={} resident={} active={} nonzero_velocity={} horses={} regiments={} dead={} grounding={} stalled={} advancing={} engaged={}",
                if battle.running { "running" } else { "paused" },
                sample_tick,
                sides[0],
                sides[1],
                resident,
                active,
                moving,
                horses,
                battle.formation.regiments.len(),
                dead,
                battle.ground_pending.len() + battle.ground_refused.len(),
                battle.formation.stalled,
                battle.formation.advancing,
                engaged
            ))
        }
        BattleCommand::Watch { names } => Ok(live_players::watch(state, names)),
        BattleCommand::Live { name, side } => live_players::live(state, name, *side),
        BattleCommand::Spectate { name, target } => {
            live_players::spectate(state, changes, name, target)
        }
    }
}

#[cfg(test)]
#[path = "staging_tests.rs"]
mod tests;
