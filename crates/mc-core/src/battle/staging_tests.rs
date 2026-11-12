use super::*;

fn fixture() -> TickState {
    crate::hostile_tests::setup(8.0).0
}

fn layout(seed: u64) -> Vec<([u64; 3], u16, String)> {
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], seed, 100).unwrap();
    state
        .hostile
        .battle
        .formation
        .members
        .iter()
        .map(|member| {
            let slot = state.entities.slot_of(member.id).unwrap();
            let row = state.entities.row(slot);
            let weapon = row.appearance.equipment[0].as_ref().unwrap();
            let name = state
                .registry
                .items
                .name_of(weapon.item_id)
                .unwrap()
                .as_str()
                .to_owned();
            (
                state.entities.pos(slot).map(f64::to_bits),
                state.entities.body(slot).faction,
                name,
            )
        })
        .collect()
}

#[test]
fn seed_reproduces_layout_and_army_mix() {
    let first = layout(123);
    assert_eq!(first.len(), 200);
    assert_eq!(first, layout(123));
    assert_ne!(first, layout(124));
}

#[test]
fn preset_has_exact_homogeneous_mix_and_spaced_horse_flanks() {
    let planned = plan(42, 10_000, [0.0, 80.0, 0.0], 1);
    for (kind, expected) in [
        (Kind::Infantry, 5_000),
        (Kind::Spear, 2_000),
        (Kind::Archer, 2_000),
        (Kind::Cavalry, 1_000),
    ] {
        assert_eq!(
            planned
                .iter()
                .filter(|(_, k, _)| *k == kind)
                .map(|(_, _, n)| *n)
                .sum::<usize>(),
            expected
        );
    }
    for (spec, kind, count) in &planned {
        assert!(*count <= 1000);
        if *kind == Kind::Cavalry {
            assert!(spec.origin[0].abs() > 100.0);
            assert_eq!(
                (slot_position(spec, 1)[0] - slot_position(spec, 0)[0]).abs(),
                2.0
            );
        }
        if *kind == Kind::Archer {
            assert_eq!(spec.origin[2], -120.0);
            assert_eq!(spec.order, Order::Hold);
        }
    }
}

#[test]
fn widened_front_preserves_population_bounds_and_distinct_spawn_slots() {
    for seed in [42, 43] {
        for count in [100, 1_000, 10_000] {
            for origin in [[0.0, 80.0, 0.0], [15.0, 80.0, 15.0]] {
                let mut positions = std::collections::HashSet::new();
                for side in [1, 2] {
                    let planned = plan(seed, count, origin, side);
                    assert_eq!(
                        planned.iter().map(|(_, _, size)| size).sum::<usize>(),
                        count
                    );
                    let front: Vec<_> = planned
                        .iter()
                        .filter(|(_, kind, _)| matches!(kind, Kind::Infantry | Kind::Spear))
                        .collect();
                    if count == 10_000 {
                        assert_eq!(
                            front
                                .iter()
                                .map(|(spec, _, _)| usize::from(spec.files))
                                .sum::<usize>(),
                            322
                        );
                        assert!(front.iter().all(|(spec, _, _)| spec.files == 46));
                    }
                    for (spec, _, size) in planned {
                        for index in 0..size {
                            let position = slot_position(&spec, index);
                            assert!((-176.0..=192.0).contains(&position[0]), "{position:?}");
                            assert!((-176.0..=192.0).contains(&position[2]), "{position:?}");
                            assert!(
                                positions.insert((
                                    (position[0] * 2.0) as i64,
                                    (position[2] * 2.0) as i64
                                )),
                                "overlapping slot {position:?}, side {side}, count {count}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn scenario_pursues_only_routing_enemies_and_resumes_fighting_afterward() {
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], 42, 100).unwrap();
    let formation = &mut state.hostile.battle.formation;
    let melee = formation
        .regiments
        .iter()
        .position(|r| r.spec.order == Order::Advance && r.spec.frontage <= 1.0)
        .unwrap();
    let archers = formation
        .regiments
        .iter()
        .position(|r| r.spec.order == Order::Hold)
        .unwrap();
    update_orders(formation);
    assert_eq!(formation.regiments[melee].spec.order, Order::Advance);
    formation.regiments[melee].sections[0].pulse_until = 100;
    formation.regiments[melee].enemy_routing = true;
    formation.regiments[archers].enemy_routing = true;
    update_orders(formation);
    assert_eq!(formation.regiments[melee].spec.order, Order::Pursue);
    assert_eq!(formation.regiments[melee].sections[0].pulse_until, 0);
    let front = formation.regiments[melee].start;
    assert!(
        formation.can_attack_at(front),
        "pursuit must permit an out-of-pulse strike"
    );
    assert_eq!(formation.regiments[archers].spec.order, Order::Hold);
    formation.regiments[melee].enemy_routing = false;
    update_orders(formation);
    assert_eq!(formation.regiments[melee].spec.order, Order::Advance);
    assert!(
        !formation.can_attack_at(front),
        "ordinary contact must wait for a pulse"
    );
    formation.regiments[melee].enemy_routing = true;
    formation.regiments[melee].morale.broken = true;
    update_orders(formation);
    assert_eq!(formation.regiments[melee].spec.order, Order::Advance);
}

fn components(state: &TickState, stack: &mc_data::ItemStack) -> Vec<(String, Tag)> {
    let fields = mc_data::item_nbt::stack_to_fields(&state.registry.items, stack).unwrap();
    match mc_data::item_nbt::field(&fields, "components") {
        Some(Tag::Compound(components)) => components.clone(),
        _ => Vec::new(),
    }
}

/// The `(slot, asset_id)` of a stack's `minecraft:equippable` component.
fn worn_asset(components: &[(String, Tag)]) -> Option<(String, String)> {
    let Some(Tag::Compound(value)) = mc_data::item_nbt::field(components, "minecraft:equippable")
    else {
        return None;
    };
    match (
        mc_data::item_nbt::field(value, "slot"),
        mc_data::item_nbt::field(value, "asset_id"),
    ) {
        (Some(Tag::String(slot)), Some(Tag::String(asset))) => Some((slot.clone(), asset.clone())),
        _ => None,
    }
}

fn dyed(components: &[(String, Tag)]) -> Option<i32> {
    match mc_data::item_nbt::field(components, "minecraft:dyed_color") {
        Some(Tag::Int(rgb)) => Some(*rgb),
        _ => None,
    }
}

/// Side-colour rule read back from a spawned husk, independent of `Loadout`:
/// a side surcoat or side-dyed leather torso, or the side heater shield.
fn husk_shows_side_colour(state: &TickState, slot: crate::entities::EntitySlot, side: u16) -> bool {
    let row = state.entities.row(slot);
    let (surcoat, heater, shades) = if side == 1 {
        (
            "solaris_battle:red_surcoat",
            "solaris_battle:red_heater_shield",
            RED_SHADES,
        )
    } else {
        (
            "solaris_battle:blue_surcoat",
            "solaris_battle:blue_heater_shield",
            BLUE_SHADES,
        )
    };
    let chest = components(state, row.appearance.equipment[4].as_ref().unwrap());
    let torso = match worn_asset(&chest) {
        Some((slot, asset)) if slot == "chest" && asset == surcoat => true,
        Some((slot, asset)) if slot == "chest" && asset == "minecraft:leather" => {
            dyed(&chest).is_some_and(|rgb| shades.contains(&rgb))
        }
        _ => false,
    };
    torso
        || row.appearance.equipment[1]
            .as_ref()
            .and_then(|shield| shield.item_model.as_deref())
            .is_some_and(|model| model.as_str() == heater)
}

/// No piece carries the other side's colour: no other-side `red_`/`blue_` model or
/// asset, every dye from the own palette, and the helmet plume dyed the own colour.
fn only_own_side(pieces: &[Vec<(String, Tag)>], head: Option<&[(String, Tag)]>, side: u16) -> bool {
    let (foreign, palette, plume) = if side == 1 {
        (
            "solaris_battle:blue_",
            [&RED_SHADES[..], &[RED_PLUME]].concat(),
            RED_PLUME,
        )
    } else {
        (
            "solaris_battle:red_",
            [&BLUE_SHADES[..], &[BLUE_PLUME]].concat(),
            BLUE_PLUME,
        )
    };
    pieces.iter().all(|components| {
        let model = matches!(
            mc_data::item_nbt::field(components, "minecraft:item_model"),
            Some(Tag::String(model)) if model.starts_with(foreign)
        );
        let asset = worn_asset(components).is_some_and(|(_, asset)| asset.starts_with(foreign));
        let dye = dyed(components).is_some_and(|rgb| !palette.contains(&rgb));
        !model && !asset && !dye
    }) && head.is_none_or(|head| dyed(head) == Some(plume))
}

#[test]
fn staged_equipment_varies_appearance_only_and_shows_side_colour_on_every_fighter() {
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], 42, 1_000).unwrap();
    let name = |state: &TickState, stack: &mc_data::ItemStack| {
        state
            .registry
            .items
            .name_of(stack.item_id)
            .unwrap()
            .as_str()
            .to_owned()
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut caparisons = std::collections::BTreeSet::new();
    for id in changes.entities_spawned.clone() {
        let slot = state.entities.slot_of(id).unwrap();
        let side = state.entities.body(slot).faction;
        let row = state.entities.row(slot);
        let Some(weapon) = row.appearance.equipment[0].as_ref() else {
            let armor = components(&state, row.loot.body_armor.as_ref().unwrap());
            assert!(only_own_side(std::slice::from_ref(&armor), None, side));
            caparisons.insert((side, worn_asset(&armor).unwrap()));
            continue;
        };
        assert!(
            husk_shows_side_colour(&state, slot, side),
            "fighter {id:?} hides side {side}"
        );
        let pieces: Vec<_> = row
            .appearance
            .equipment
            .iter()
            .flatten()
            .map(|stack| components(&state, stack))
            .collect();
        let head = components(&state, row.appearance.equipment[5].as_ref().unwrap());
        assert!(
            only_own_side(&pieces, Some(head.as_slice()), side),
            "fighter {id:?} wears the other side's colour on side {side}"
        );
        // Combat identities stay vanilla: only appearance components vary.
        let mounted = row.entity_life.mount.vehicle.is_some();
        let kind = match (name(&state, weapon).as_str(), mounted) {
            ("minecraft:trident", _) => Kind::Spear,
            ("minecraft:bow", _) => Kind::Archer,
            ("minecraft:iron_sword", true) => Kind::Cavalry,
            ("minecraft:iron_sword", false) => Kind::Infantry,
            (other, _) => panic!("unexpected battle weapon {other}"),
        };
        for (index, (slot_name, item)) in equipment(kind).iter().enumerate() {
            let stack = match *slot_name {
                "mainhand" => weapon,
                "offhand" => row.appearance.equipment[1].as_ref().unwrap(),
                "head" => row.appearance.equipment[5].as_ref().unwrap(),
                _ => row.appearance.equipment[4].as_ref().unwrap(),
            };
            assert_eq!(name(&state, stack), *item, "{kind:?} slot {index}");
        }
        if kind == Kind::Archer {
            assert!(row.appearance.equipment[1].is_none());
        }
        seen.insert((
            "weapon",
            weapon.item_model.as_deref().unwrap().as_str().to_owned(),
        ));
        if let Some(shield) = row.appearance.equipment[1].as_ref() {
            seen.insert((
                "shield",
                shield.item_model.as_deref().unwrap().as_str().to_owned(),
            ));
        }
        let head = worn_asset(&components(
            &state,
            row.appearance.equipment[5].as_ref().unwrap(),
        ));
        let (head_slot, helmet) = head.unwrap();
        assert_eq!(head_slot, "head");
        seen.insert(("helmet", helmet));
        let chest = components(&state, row.appearance.equipment[4].as_ref().unwrap());
        seen.insert(("torso", worn_asset(&chest).unwrap().1));
        match row.appearance.equipment[3].as_ref() {
            None => {
                seen.insert(("legs", "bare".into()));
            }
            Some(legs) => {
                assert_eq!(name(&state, legs), LEGS_CARRIER);
                let legs_components = components(&state, legs);
                let (legs_slot, asset) = worn_asset(&legs_components).unwrap();
                assert_eq!(legs_slot, "legs");
                if asset == "minecraft:leather" {
                    assert!(dyed(&legs_components).is_some());
                }
                seen.insert(("legs", asset));
            }
        }
    }
    let expected: std::collections::BTreeSet<_> = [
        ("weapon", "solaris_battle:iron_longsword"),
        ("weapon", "solaris_battle:iron_flanged_mace"),
        ("weapon", "solaris_battle:iron_spear"),
        ("weapon", "solaris_battle:iron_halberd"),
        ("weapon", "solaris_battle:iron_longbow_standby"),
        ("weapon", "solaris_battle:iron_heavy_crossbow_standby"),
        ("weapon", "solaris_battle:iron_lance"),
        ("shield", "solaris_battle:red_heater_shield"),
        ("shield", "solaris_battle:blue_heater_shield"),
        ("shield", "solaris_battle:iron_kite_shield"),
        ("shield", "solaris_battle:iron_tower_shield"),
        ("helmet", "solaris_battle:iron_plate"),
        ("helmet", "solaris_battle:crested_helm"),
        ("helmet", "solaris_battle:holy"),
        ("torso", "solaris_battle:red_surcoat"),
        ("torso", "solaris_battle:blue_surcoat"),
        ("torso", "minecraft:leather"),
        ("torso", "solaris_battle:holy"),
        ("legs", "bare"),
        ("legs", "minecraft:leather"),
        ("legs", "solaris_battle:holy"),
    ]
    .into_iter()
    .map(|(part, value)| (part, value.to_owned()))
    .collect();
    assert_eq!(seen, expected);
    let caparison = |side, asset: &str| (side, ("body".to_owned(), asset.to_owned()));
    assert_eq!(
        caparisons,
        [
            caparison(1, "solaris_battle:red_caparison"),
            caparison(2, "solaris_battle:blue_caparison"),
        ]
        .into_iter()
        .collect()
    );
    // Worn legs add no armour: the carrier has no attribute modifier.
    let rules = state.registry.item_rules();
    let carrier = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(LEGS_CARRIER).unwrap())
        .unwrap();
    let facts = rules
        .facts
        .facts_for_stack(&mc_data::ItemStack::new(carrier, 1), rules.items);
    assert!(facts.is_none_or(|facts| facts.attribute_modifiers.is_empty()));
}

fn fighters(seed: u64, side: u16) -> Vec<Loadout> {
    plan(seed, 10_000, [0.0, 80.0, 0.0], side)
        .into_iter()
        .flat_map(|(spec, kind, size)| {
            (0..size).map(move |index| loadout(seed, side, spec.id, index, kind))
        })
        .collect()
}

#[test]
fn same_seed_repeats_every_loadout_and_staged_appearance() {
    // 42 and 44 share the regiment plan (even seeds), so only the dress differs.
    for side in [1, 2] {
        assert_eq!(fighters(42, side), fighters(42, side));
        assert_ne!(fighters(42, side), fighters(44, side));
    }
    let appearance = |seed| {
        let mut state = fixture();
        let mut changes = ChangeSet::default();
        setup(&mut state, &mut changes, [0.0, 80.0, 0.0], seed, 100).unwrap();
        state
            .hostile
            .battle
            .formation
            .members
            .iter()
            .map(|member| {
                let row = state
                    .entities
                    .row(state.entities.slot_of(member.id).unwrap());
                row.appearance
                    .equipment
                    .iter()
                    .map(|stack| stack.as_ref().map(|stack| components(&state, stack)))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let first = appearance(42);
    assert_eq!(first, appearance(42));
    assert_ne!(first, appearance(44));
}

#[test]
fn ten_thousand_side_spreads_every_variant_near_its_share_and_shows_side_colour() {
    fn share<T: PartialEq>(pool: &[&Loadout], pick: impl Fn(&Loadout) -> T, value: T) -> f64 {
        pool.iter().filter(|l| pick(l) == value).count() as f64 * 100.0 / pool.len() as f64
    }
    fn near(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= 4.0,
            "{what}: {actual:.1}% vs {expected}%"
        );
    }
    for seed in [42, 7] {
        for side in [1, 2] {
            let all = fighters(seed, side);
            assert_eq!(all.len(), 10_000);
            // The rule read back from each fighter's summon tag.
            for unit in &all {
                let tag = summon_tag(Some(unit), side);
                let Tag::Compound(fields) = &tag else {
                    unreachable!()
                };
                let Some(Tag::Compound(slots)) = mc_data::item_nbt::field(fields, "equipment")
                else {
                    panic!("summon tag without equipment")
                };
                let parts = |slot: &str| match mc_data::item_nbt::field(slots, slot) {
                    Some(Tag::Compound(stack)) => {
                        match mc_data::item_nbt::field(stack, "components") {
                            Some(Tag::Compound(components)) => components.clone(),
                            _ => Vec::new(),
                        }
                    }
                    _ => Vec::new(),
                };
                let chest = parts("chest");
                let (surcoat, heater, shades) = if side == 1 {
                    (
                        "solaris_battle:red_surcoat",
                        "solaris_battle:red_heater_shield",
                        RED_SHADES,
                    )
                } else {
                    (
                        "solaris_battle:blue_surcoat",
                        "solaris_battle:blue_heater_shield",
                        BLUE_SHADES,
                    )
                };
                let torso = match worn_asset(&chest) {
                    Some((_, asset)) if asset == surcoat => true,
                    Some((_, asset)) if asset == "minecraft:leather" => {
                        dyed(&chest).is_some_and(|rgb| shades.contains(&rgb))
                    }
                    _ => false,
                };
                let shield = mc_data::item_nbt::field(&parts("offhand"), "minecraft:item_model")
                    == Some(&Tag::String(heater.into()));
                assert!(torso || shield, "{unit:?} hides side {side}");
                let pieces: Vec<_> = ["mainhand", "offhand", "head", "chest", "legs"]
                    .into_iter()
                    .map(&parts)
                    .collect();
                assert!(
                    only_own_side(&pieces, Some(parts("head").as_slice()), side),
                    "{unit:?} wears the other side's colour on side {side}"
                );
            }
            let of = |kind| all.iter().filter(|l| l.kind == kind).collect::<Vec<_>>();
            let (infantry, spears, archers, cavalry) = (
                of(Kind::Infantry),
                of(Kind::Spear),
                of(Kind::Archer),
                of(Kind::Cavalry),
            );
            for (pool, variants) in [
                (
                    &infantry,
                    [
                        ("solaris_battle:iron_longsword", 60.0),
                        ("solaris_battle:iron_flanged_mace", 40.0),
                    ],
                ),
                (
                    &spears,
                    [
                        ("solaris_battle:iron_spear", 60.0),
                        ("solaris_battle:iron_halberd", 40.0),
                    ],
                ),
                (
                    &archers,
                    [
                        ("solaris_battle:iron_longbow_standby", 60.0),
                        ("solaris_battle:iron_heavy_crossbow_standby", 40.0),
                    ],
                ),
            ] {
                for (model, expected) in variants {
                    near(share(pool, |l| l.weapon, model), expected, model);
                }
            }
            assert!(
                cavalry
                    .iter()
                    .all(|l| l.weapon == "solaris_battle:iron_lance")
            );
            assert!(archers.iter().all(|l| l.shield.is_none()));
            let foot: Vec<_> = infantry.iter().chain(&spears).copied().collect();
            for (shield, expected) in [
                (Shield::Heater, 50.0),
                (Shield::Kite, 30.0),
                (Shield::Tower, 20.0),
            ] {
                near(
                    share(&foot, |l| l.shield, Some(shield)),
                    expected,
                    &format!("foot {shield:?}"),
                );
            }
            for (shield, expected) in [(Shield::Heater, 60.0), (Shield::Kite, 40.0)] {
                near(
                    share(&cavalry, |l| l.shield, Some(shield)),
                    expected,
                    &format!("horse {shield:?}"),
                );
            }
            let every: Vec<_> = all.iter().collect();
            for (helmet, expected) in [
                ("solaris_battle:iron_plate", 45.0),
                ("solaris_battle:crested_helm", 30.0),
                ("solaris_battle:holy", 25.0),
            ] {
                near(share(&every, |l| l.helmet, helmet), expected, helmet);
            }
            let (heater, plain): (Vec<&Loadout>, Vec<&Loadout>) = every
                .iter()
                .copied()
                .partition(|l| l.shield == Some(Shield::Heater));
            for (torso, expected) in [
                (Torso::Surcoat, 45.0),
                (Torso::Leather, 35.0),
                (Torso::Holy, 20.0),
            ] {
                near(
                    share(&heater, |l| l.torso, torso),
                    expected,
                    &format!("heater {torso:?}"),
                );
            }
            for (torso, expected) in [(Torso::Surcoat, 55.0), (Torso::Leather, 45.0)] {
                near(
                    share(&plain, |l| l.torso, torso),
                    expected,
                    &format!("plain {torso:?}"),
                );
            }
            for (legs, expected) in [
                (Legs::Bare, 35.0),
                (Legs::Leather, 35.0),
                (Legs::Holy, 30.0),
            ] {
                near(
                    share(&every, |l| l.legs, legs),
                    expected,
                    &format!("{legs:?}"),
                );
            }
            for shade in 0..4 {
                near(
                    share(&every, |l| l.shade, shade),
                    25.0,
                    &format!("shade {shade}"),
                );
            }
        }
    }
}

#[test]
fn reset_removes_battle_units_and_ticket_not_other_mobs() {
    let mut state = fixture();
    let unrelated = state
        .entities
        .slots()
        .map(|slot| state.entities.body(slot).id)
        .collect::<Vec<_>>();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], 9, 100).unwrap();
    let spawned = changes.entities_spawned.clone();
    assert!(spawned.len() >= 200);
    assert!(spawned.len() <= 400, "at most one horse per soldier");
    assert_eq!(state.hostile.battle.formation.members.len(), 200);
    assert_eq!(state.hostile.battle.formation.regiments.len(), 10);
    for name in ["mob_drops", "mob_griefing", "spawn_mobs"] {
        assert_eq!(state.gamerules.get(name).map(String::as_str), Some("false"));
    }
    let husk = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:husk").unwrap())
        .unwrap();
    for regiment in &state.hostile.battle.formation.regiments {
        let members =
            &state.hostile.battle.formation.members[regiment.start..regiment.start + regiment.len];
        let first_slot = state.entities.slot_of(members[0].id).unwrap();
        let first = state.entities.row(first_slot);
        let weapon = first.appearance.equipment[0].as_ref().unwrap().item_id;
        let mounted = first.entity_life.mount.vehicle.is_some();
        for member in members {
            let slot = state.entities.slot_of(member.id).unwrap();
            let row = state.entities.row(slot);
            assert!(
                matches!(state.entities.body(slot).kind, crate::entities::EntityKind::Mob { type_id } if type_id == husk)
            );
            assert_eq!(
                row.appearance.equipment[0].as_ref().unwrap().item_id,
                weapon
            );
            assert_eq!(row.entity_life.mount.vehicle.is_some(), mounted);
        }
    }
    let error = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Start).unwrap_err();
    assert!(error.contains("outside loaded simulation"), "{error}");
    assert!(!state.hostile.battle.running);
    let status = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Status).unwrap();
    assert!(status.contains("side1=100 side2=100"), "{status}");
    assert!(
        status.contains(&format!("horses={} ", spawned.len() - 200)),
        "{status}"
    );
    let roster = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Roster).unwrap();
    let listed: std::collections::HashSet<_> = roster
        .strip_prefix("Battle roster: ")
        .unwrap()
        .split(',')
        .collect();
    assert_eq!(listed.len(), spawned.len());
    for id in &spawned {
        let slot = state.entities.slot_of(*id).unwrap();
        assert!(
            state.entities.row(slot).persistent,
            "battle entity can naturally despawn"
        );
        assert!(listed.contains(state.entities.row(slot).uuid.to_string().as_str()));
    }
    let first = &state.hostile.battle.formation.members[0];
    let (first_id, x, z) = (first.id.0, first.pos[0], first.pos[2]);
    let trace = execute(
        &mut state,
        &mut changes,
        [0.0; 3],
        &BattleCommand::Trace { x, z, radius: 2.0 },
    )
    .unwrap();
    assert!(
        trace.starts_with("Battle trace: ") && trace.contains(&format!(" {first_id} side1 r")),
        "{trace}"
    );
    assert!(trace.contains(" to "), "{trace}");
    assert_eq!(
        parse(&["trace", "1", "2", "8"]).unwrap(),
        BattleCommand::Trace { x: 1.0, z: 2.0, radius: 8.0 }
    );
    assert!(parse(&["trace", "1", "2", "40"]).is_err(), "radius over the cap");
    let want: rustc_hash::FxHashMap<_, _> = spawned
        .iter()
        .map(|id| {
            let slot = state.entities.slot_of(*id).unwrap();
            let pos = state.entities.pos(slot);
            (
                ChunkPos {
                    x: (pos[0].floor() as i32).div_euclid(16),
                    z: (pos[2].floor() as i32).div_euclid(16),
                },
                0,
            )
        })
        .collect();
    state.world.ensure_blocking(&want);
    state.refresh_active_chunks(state.tick);
    execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Start).unwrap();
    assert!(state.hostile.battle.running);
    execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Pause).unwrap();
    assert!(!state.hostile.battle.running);
    assert!(state.tickets.contains_key(OWNER));
    assert_eq!(reset(&mut state, &mut changes), spawned.len());
    assert!(
        spawned
            .iter()
            .all(|id| state.entities.slot_of_any(*id).is_none())
    );
    assert!(
        unrelated
            .iter()
            .all(|id| state.entities.slot_of_any(*id).is_some())
    );
    assert!(!state.tickets.contains_key(OWNER));
    assert!(state.hostile.battle.formation.regiments.is_empty());
}

#[test]
fn status_tick_matches_command_commit_in_local_and_shared_dispatch() {
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    state.tick = 20;
    let local = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Status).unwrap();
    assert!(local.contains("tick=21 "), "{local}");
    state.command_context.shared = true;
    let shared = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Status).unwrap();
    assert!(shared.contains("tick=20 "), "{shared}");
}

#[test]
fn peaceful_world_rejects_setup_without_partial_changes() {
    let mut state = fixture();
    state.difficulty = crate::survival::Difficulty::Peaceful;
    let mut changes = ChangeSet::default();
    let error = setup(&mut state, &mut changes, [0.0, 80.0, 0.0], 9, 100).unwrap_err();
    assert!(error.contains("peaceful"));
    assert!(changes.entities_spawned.is_empty());
    assert!(!state.tickets.contains_key(OWNER));
}

#[test]
fn commands_require_operator_and_enforce_count_bounds() {
    for text in [
        "battle setup 1 field-100",
        "battle start",
        "battle pause",
        "battle reset",
        "battle status",
        "battle roster",
    ] {
        assert!(crate::commands::parse(text, false).is_err(), "{text}");
        assert!(crate::commands::parse(text, true).is_ok(), "{text}");
    }
    assert!(crate::commands::root_permitted("battle", 2));
    assert!(!crate::commands::root_permitted("battle", 1));
    assert!(crate::commands::parse("battle setup 1 field-100 10001", true).is_err());
    assert!(crate::commands::parse("battle setup 1 field-100 0", true).is_err());
    let roster = crate::commands::parse("battle roster", true).unwrap();
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    assert_eq!(
        crate::command_execution::execute(
            &mut state,
            crate::boundary::ConnectionId(1),
            EntityId(1),
            &roster,
            &mut changes
        ),
        crate::command_execution::CommandResult::Failed
    );
}

#[test]
fn status_counts_a_corpse_as_dead_until_its_body_leaves() {
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], 9, 100).unwrap();
    let victim = state.hostile.battle.formation.members[0].id;
    let slot = state.entities.slot_of(victim).unwrap();
    let side = state.entities.body(slot).faction;
    let before = crate::battle::publication_counts(&mut state).unwrap();
    let alive_on = |counts: &crate::boundary::BattleCounts| {
        counts
            .alive
            .iter()
            .find(|(faction, _)| *faction == side)
            .map(|(_, alive)| *alive)
    };
    state.merge_damage.push((
        slot,
        crate::tick::DamagePiece {
            amount: 1000.0,
            kind: crate::survival::DamageKind::GenericKill,
            source: None,
            physical: None,
            custom_armor: false,
        },
    ));
    state.commit_merge_effects();
    assert!(
        state.entities.slot_of_any(victim).is_some(),
        "the body lies 20 ticks"
    );
    let sides = if side == 1 {
        "side1=99 side2=100"
    } else {
        "side1=100 side2=99"
    };
    let status = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Status).unwrap();
    assert!(
        status.contains(sides) && status.contains(" dead=1 grounding="),
        "{status}"
    );
    let counts = crate::battle::publication_counts(&mut state).unwrap();
    assert_eq!(counts.units, before.units, "the corpse is a retained unit");
    assert_eq!(
        alive_on(&counts),
        alive_on(&before).map(|alive| alive - 1),
        "{:?}",
        counts.alive
    );
    state.hostile.battle.running = true;
    crate::battle::step(&mut state);
    let member = state
        .hostile
        .battle
        .formation
        .members
        .iter()
        .find(|member| member.id == victim)
        .unwrap();
    assert!(!member.alive, "a corpse does not keep fighting");
    state.hostile.battle.running = false;
    for _ in 0..20 {
        crate::tick::run_tick(&mut state, &[]);
    }
    assert!(state.entities.slot_of_any(victim).is_none());
    let status = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Status).unwrap();
    assert!(
        status.contains(sides) && status.contains(" dead=0 grounding="),
        "{status}"
    );
}

#[test]
fn failed_setup_rolls_back_owned_rows_changes_ticket_rules_and_journal() {
    use crate::journal::JournalEntry;
    use std::collections::BTreeSet;
    let mut state = fixture();
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    state.gamerules.insert("mob_drops".into(), "true".into());
    state.gamerules.insert("mob_griefing".into(), "true".into());
    state.gamerules_dirty = false;
    let kind = state.registry.entity_type_id("minecraft:cow").unwrap();
    let unrelated = state.spawn_mob(kind, [0.0, 80.0, 0.0], 20.0);
    let removed = state.spawn_mob(kind, [1.0, 80.0, 0.0], 20.0);
    state.remove_entity(state.entities.slot_of(removed).unwrap());
    let mut changes = ChangeSet::default();
    changes.entities_spawned.push(unrelated);
    changes.entities_removed.push(removed);
    let spawned_before = changes.entities_spawned.clone();
    let removed_before = changes.entities_removed.clone();
    let ids_before: BTreeSet<_> = state
        .entities
        .slots()
        .map(|slot| state.entities.body(slot).id)
        .collect();
    let rules_before = state.gamerules.clone();
    let origin = [0.0, 80.0, 0.0];
    let mut collision = plan(9, 100, origin, 1)[0].0;
    collision.files = 1;
    assert_eq!(collision.id, 0);
    state
        .hostile
        .battle
        .formation
        .add_regiment(collision, &[unrelated])
        .unwrap();
    assert!(setup(&mut state, &mut changes, origin, 9, 100).is_err());
    assert!(
        tagged_ids(&state).is_empty(),
        "failed setup left battle-owned rows"
    );
    assert_eq!(
        state
            .entities
            .slots()
            .map(|slot| state.entities.body(slot).id)
            .collect::<BTreeSet<_>>(),
        ids_before
    );
    assert_eq!(changes.entities_spawned, spawned_before);
    assert_eq!(changes.entities_removed, removed_before);
    assert!(!state.tickets.contains_key(OWNER));
    assert_eq!(state.gamerules, rules_before);
    assert!(!state.gamerules_dirty);

    let (mut output, _, _) = crate::tick::run_tick(&mut state, &[]);
    let published = std::sync::Arc::get_mut(&mut output.changes).unwrap();
    published.entities_spawned.extend(changes.entities_spawned);
    published.entities_removed.extend(changes.entities_removed);
    let entries = crate::core::journal_entries_for_tick(&mut state, &output);
    let allowed: BTreeSet<_> = ids_before
        .into_iter()
        .chain([removed])
        .map(|id| id.0)
        .collect();
    for entry in entries {
        if let JournalEntry::Entity { id, .. } | JournalEntry::RemoveEntity { id, .. } = entry {
            assert!(
                allowed.contains(&id),
                "failed setup leaked entity {id} into WAL"
            );
        }
    }
}

#[test]
fn staged_fighters_and_horses_face_the_enemy_line_at_spawn() {
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], 42, 100).unwrap();
    let mut centroid_z = [0.0; 2];
    let mut counts = [0_usize; 2];
    for id in &changes.entities_spawned {
        let slot = state.entities.slot_of(*id).unwrap();
        let side = usize::from(state.entities.body(slot).faction - 1);
        centroid_z[side] += state.entities.pos(slot)[2];
        counts[side] += 1;
    }
    let centroid_z = [0, 1].map(|side| centroid_z[side] / counts[side] as f64);
    let mut horses = 0;
    for id in &changes.entities_spawned {
        let slot = state.entities.slot_of(*id).unwrap();
        let side = usize::from(state.entities.body(slot).faction - 1);
        let rotation = state.entities.rot(slot);
        assert_eq!((rotation[1], rotation[2]), (0.0, rotation[0]), "level gaze, head on body");
        // Vanilla look vector: x = -sin(yaw), z = cos(yaw).
        let yaw = f64::from(rotation[0]).to_radians();
        let enemy = (centroid_z[1 - side] - state.entities.pos(slot)[2]).signum();
        assert!(
            (-yaw.sin()).abs() < 1e-6 && yaw.cos() * enemy > 0.999,
            "side {} body faces yaw {} away from the enemy",
            side + 1,
            yaw.to_degrees()
        );
        horses += usize::from(state.entities.row(slot).entity_life.mount.rider.is_some());
    }
    assert_eq!(horses, 20, "horses are checked too");
}
