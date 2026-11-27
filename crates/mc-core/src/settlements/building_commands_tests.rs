//! Shared `/settlement building` selection/authorization regression gates.
//!
//! Native-only: deterministic closest-anchor selection, owner-or-operator
//! refusals, global upgrade lookup with home-settlement membership, and the
//! public list shape (id, kind, level, bounds).
use super::*;
use crate::settlements::SettlementId;
use crate::settlements::allocator::tests::{block, test_state};
use crate::settlements::buildings::BuildingKind;

fn town(state: &mut TickState, name: &str, owner: &str, anchor: [i32; 3]) -> SettlementId {
    state
        .settlements
        .create(
            &mut state.settlement_sequence,
            name.into(),
            owner.into(),
            anchor,
            48,
        )
        .unwrap()
}

fn player(state: &mut TickState, pos: [f64; 3]) -> EntityId {
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:player").unwrap())
        .unwrap();
    state.spawn_mob(type_id, pos, 20.0)
}

fn station(state: &mut TickState, at: [i32; 3], name: &str) {
    let id = block(state, name, &[]);
    state.world.set_block(at[0], at[1], at[2], id);
    state.world.fold_overlay();
}

/// A certifiable 5x4x5 farmer workshop at y=201: one composter, solid stone
/// shell, air gap inside (see parent buildings tests for the shape).
fn workshop_bounds() -> mc_script::ScriptWorldBounds {
    mc_script::ScriptWorldBounds::new([3, 200, 3], [7, 204, 7]).unwrap()
}

fn raise_workshop(state: &mut TickState) {
    for x in 3..=7 {
        for z in 3..=7 {
            for y in 200..=204 {
                let edge = x == 3 || x == 7 || z == 3 || z == 7;
                let name = if y == 200 || y == 204 || (edge && (201..=203).contains(&y)) {
                    "minecraft:stone"
                } else {
                    "minecraft:air"
                };
                station(state, [x, y, z], name);
            }
        }
    }
    station(state, [4, 201, 4], "minecraft:composter");
}

#[test]
fn containing_prefers_closest_anchor_then_smallest_id() {
    let mut state = test_state();
    let near = town(&mut state, "near", "owner", [0, 201, 0]);
    let far = town(&mut state, "far", "owner", [30, 201, 0]);
    let solo = player(&mut state, [25.0, 201.0, 0.0]);
    // Both spheres contain x=25 (near: 25, far: 5) — closer anchor wins.
    assert_eq!(containing(&state, solo), Some(far));
    let tie = player(&mut state, [15.0, 201.0, 0.0]);
    // Equidistant anchors (15 each): smallest settlement id wins.
    assert_eq!(containing(&state, tie), Some(near));
    let anchor = player(&mut state, [0.0, 201.0, 0.0]);
    // Exactly on the near anchor: distance 0 beats everything.
    assert_eq!(containing(&state, anchor), Some(near));
    let outside = player(&mut state, [200.0, 201.0, 0.0]);
    assert_eq!(containing(&state, outside), None);
    let ghost = mc_domain::EntityId(-1);
    assert_eq!(containing(&state, ghost), None);
}

#[test]
fn certify_needs_owner_or_operator_and_known_kind() {
    let mut state = test_state();
    raise_workshop(&mut state);
    town(&mut state, "town", "owner", [8, 201, 8]);
    let executor = player(&mut state, [5.0, 201.0, 5.0]);
    assert!(certify_here(&mut state, executor, None, false, "farmer").is_err());
    assert_eq!(
        certify_here(&mut state, executor, Some("stranger"), false, "farmer").unwrap_err(),
        "Only the owner or an operator certifies buildings.",
    );
    assert_eq!(
        certify_here(&mut state, executor, Some("owner"), false, "castle").unwrap_err(),
        "Unknown building kind 'castle'.",
    );
    let reply = certify_here(&mut state, executor, Some("owner"), false, "farmer").unwrap();
    assert!(reply.starts_with("Certified farmer #"), "{reply}");
    // A stranger with operator rights recertifies the same kind over the same
    // bounds (same-kind expansion path): must succeed.
    let recert = certify_here(&mut state, executor, Some("stranger"), true, "farmer").unwrap();
    assert!(recert.starts_with("Certified farmer #"), "{recert}");
}

#[test]
fn upgrade_authorizes_home_owner_from_anywhere_and_resolves_globally() {
    let mut state = test_state();
    raise_workshop(&mut state);
    town(&mut state, "home", "owner", [8, 201, 8]);
    town(&mut state, "away", "owner", [100, 201, 100]);
    let bounds = workshop_bounds();
    let home_executor = player(&mut state, [8.0, 201.0, 8.0]);
    let reply = certify(
        &mut state,
        home_executor,
        Some("owner"),
        true,
        "farmer",
        bounds.min,
        bounds.max,
    )
    .unwrap();
    let id: u64 = reply
        .split('#')
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .unwrap();
    // Global id: upgrading needs no executor position, only the home authority.
    station(&mut state, [6, 201, 4], "minecraft:composter");
    let upgraded = upgrade(&mut state, Some("owner"), false, id).unwrap();
    assert!(
        upgraded.starts_with(&format!("Upgraded building #{id}")),
        "{upgraded}"
    );
    assert_eq!(
        upgrade(&mut state, Some("owner"), false, 999_999).unwrap_err(),
        "Building not found.",
    );
    assert_eq!(
        upgrade(&mut state, Some("stranger"), false, id).unwrap_err(),
        "Only the owner or an operator upgrades buildings.",
    );
    assert_eq!(
        upgrade(&mut state, None, false, id).unwrap_err(),
        ADMISSION_UNAVAILABLE,
    );
    station(&mut state, [4, 201, 6], "minecraft:composter");
    assert!(
        upgrade(&mut state, Some("stranger"), true, id)
            .unwrap()
            .contains("level 3"),
    );
    assert_eq!(
        BuildingKind::parse("farmer"),
        Some(BuildingKind::Workshop(
            crate::villager::VillagerProfession::Farmer
        ))
    );
}

#[test]
fn list_is_public_and_shows_id_kind_level_bounds() {
    let mut state = test_state();
    raise_workshop(&mut state);
    town(&mut state, "town", "owner", [8, 201, 8]);
    let executor = player(&mut state, [8.0, 201.0, 8.0]);
    assert_eq!(list(&state, executor).unwrap(), "No buildings.");
    let bounds = workshop_bounds();
    certify(
        &mut state,
        executor,
        Some("owner"),
        true,
        "farmer",
        bounds.min,
        bounds.max,
    )
    .unwrap();
    let reply = list(&state, executor).unwrap();
    assert!(reply.starts_with("Buildings: #"), "{reply}");
    assert!(reply.contains("farmer level 1"), "{reply}");
    assert!(reply.contains("[3, 200, 3]-[7, 204, 7]"), "{reply}");
    assert!(!reply.contains("natural"), "{reply}");
    let id = state.settlements.records().next().unwrap().id;
    let record = state.settlements.get_mut(id).unwrap();
    record.buildings.values_mut().next().unwrap().natural_zone = true;
    let reply = list(&state, executor).unwrap();
    assert!(reply.contains("[3, 200, 3]-[7, 204, 7] natural"), "{reply}");
    let outsider = player(&mut state, [200.0, 201.0, 0.0]);
    assert_eq!(list(&state, outsider).unwrap_err(), NO_SETTLEMENT);
}

#[test]
fn overlapping_village_resolution_refusal_names_the_claim_requirement() {
    // Real-client QA receipt (20261020T095116-buildings-qa): a town founded
    // inside a generated village sphere resolved the nearer, autonomous
    // village for certify while the founder stood mid-way, and the refusal
    // claimed owner/operator rights that an unclaimed settlement never grants.
    let mut state = test_state();
    raise_workshop(&mut state);
    let village = state
        .settlements
        .adopt_village(
            &mut state.settlement_sequence,
            "minecraft:overworld:4:201:4",
            [4, 201, 4],
        )
        .unwrap();
    town(&mut state, "bqa_barracks", "alice", [20, 201, 20]);
    let midway = player(&mut state, [8.0, 201.0, 8.0]);
    assert_eq!(
        containing(&state, midway),
        Some(village),
        "the autonomous village anchor is nearer"
    );
    let bounds = workshop_bounds();
    let name = state.settlements.get(village).unwrap().name.clone();
    assert_eq!(
        certify(
            &mut state,
            midway,
            Some("alice"),
            true,
            "farmer",
            bounds.min,
            bounds.max,
        )
        .unwrap_err(),
        format!("{name} is an autonomous settlement; claim it before managing it."),
    );
    // Standing on the town's own anchor resolves the town and admits its
    // founder; certification permission itself is unchanged.
    let founder = player(&mut state, [20.0, 201.0, 20.0]);
    let reply = certify(
        &mut state,
        founder,
        Some("alice"),
        true,
        "farmer",
        bounds.min,
        bounds.max,
    )
    .unwrap();
    assert!(reply.starts_with("Certified farmer #"), "{reply}");
}

#[test]
fn autonomous_village_refuses_certify_until_claimed() {
    let mut state = test_state();
    raise_workshop(&mut state);
    let village = state
        .settlements
        .adopt_village(
            &mut state.settlement_sequence,
            "minecraft:overworld:8:201:8",
            [8, 201, 8],
        )
        .unwrap();
    let executor = player(&mut state, [8.0, 201.0, 8.0]);
    let bounds = workshop_bounds();
    let name = state.settlements.get(village).unwrap().name.clone();
    let refusal = format!("{name} is an autonomous settlement; claim it before managing it.");
    assert_eq!(
        certify(
            &mut state,
            executor,
            Some("carol"),
            true,
            "farmer",
            bounds.min,
            bounds.max,
        )
        .unwrap_err(),
        refusal,
    );
    assert_eq!(
        certify(
            &mut state,
            executor,
            Some("carol"),
            true,
            "farmer",
            bounds.min,
            bounds.max,
        )
        .unwrap_err(),
        refusal,
        "operators cannot manage an unclaimed settlement either"
    );
    state
        .settlements
        .claim(village, "carol".to_owned(), [8, 201, 8])
        .unwrap();
    let reply = certify(
        &mut state,
        executor,
        Some("carol"),
        true,
        "farmer",
        bounds.min,
        bounds.max,
    )
    .unwrap();
    assert!(reply.starts_with("Certified farmer #"), "{reply}");
}

#[test]
fn explicit_bounds_require_an_operator_even_when_the_actor_owns_the_town() {
    let mut state = test_state();
    raise_workshop(&mut state);
    town(&mut state, "town", "owner", [8, 201, 8]);
    let executor = player(&mut state, [8.0, 201.0, 8.0]);
    let bounds = workshop_bounds();
    let refusal = certify(
        &mut state,
        executor,
        Some("owner"),
        false,
        "farmer",
        bounds.min,
        bounds.max,
    )
    .unwrap_err();
    assert!(refusal.contains("operator level 2") && refusal.contains("Проверить здесь"));
    assert!(
        state
            .settlements
            .records()
            .next()
            .unwrap()
            .buildings
            .is_empty()
    );
    certify(
        &mut state,
        executor,
        Some("owner"),
        true,
        "farmer",
        bounds.min,
        bounds.max,
    )
    .unwrap();
    assert_eq!(
        state.settlements.records().next().unwrap().buildings.len(),
        1
    );
}

#[test]
fn non_operator_owner_certifies_open_storage_and_rechecks_the_same_certificate() {
    let mut state = test_state();
    let id = town(&mut state, "town", "owner", [8, 201, 8]);
    let executor = player(&mut state, [5.5, 201.0, 5.5]);
    station(&mut state, [4, 201, 4], "minecraft:chest");
    certify_here(&mut state, executor, Some("owner"), false, "storage").unwrap();
    let before = state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .values()
        .next()
        .unwrap()
        .clone();
    certify_here(&mut state, executor, Some("owner"), false, "storage").unwrap();
    let buildings = &state.settlements.get(id).unwrap().buildings;
    assert_eq!(buildings.len(), 1);
    assert_eq!(buildings[&before.id].bounds, before.bounds);
    assert_eq!(buildings[&before.id].work_points, [[4, 201, 4]]);
}

#[test]
fn non_operator_owner_certifies_open_mine_camp_and_every_workshop() {
    let mut state = test_state();
    let id = town(&mut state, "town", "owner", [8, 201, 8]);
    let executor = player(&mut state, [5.5, 201.0, 5.5]);
    for (kind, name) in [
        ("mine", "smithing_table"),
        ("lumber_camp", "fletching_table"),
        ("builder", "crafting_table"),
        ("armorer", "blast_furnace"),
        ("butcher", "smoker"),
        ("cartographer", "cartography_table"),
        ("cleric", "brewing_stand"),
        ("farmer", "composter"),
        ("fisherman", "barrel"),
        ("fletcher", "fletching_table"),
        ("leatherworker", "cauldron"),
        ("librarian", "lectern"),
        ("mason", "stonecutter"),
        ("shepherd", "loom"),
        ("toolsmith", "smithing_table"),
        ("weaponsmith", "grindstone"),
    ] {
        station(&mut state, [4, 201, 4], &format!("minecraft:{name}"));
        certify_here(&mut state, executor, Some("owner"), false, kind)
            .unwrap_or_else(|error| panic!("{kind}: {error}"));
        assert_eq!(
            state
                .settlements
                .get(id)
                .unwrap()
                .buildings
                .values()
                .next()
                .unwrap()
                .work_points,
            [[4, 201, 4]]
        );
        state.settlements.get_mut(id).unwrap().buildings.clear();
    }
}

#[test]
fn non_operator_owner_certifies_the_straight_wall_beside_them() {
    let mut state = test_state();
    let id = town(&mut state, "town", "owner", [8, 201, 8]);
    let executor = player(&mut state, [5.5, 201.0, 4.5]);
    for x in 3..=7 {
        for y in 201..=203 {
            state.world.set_block(x, y, 3, state.registry.states.stone);
        }
    }
    state.world.fold_overlay();
    certify_here(&mut state, executor, Some("owner"), false, "wall").unwrap();
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .buildings
            .values()
            .next()
            .unwrap()
            .bounds,
        mc_script::ScriptWorldBounds::new([3, 200, 3], [7, 203, 3]).unwrap()
    );
}

#[test]
fn non_operator_owner_certifies_the_gate_frame_beside_its_door() {
    let mut state = test_state();
    let id = town(&mut state, "town", "owner", [8, 201, 8]);
    let executor = player(&mut state, [5.5, 201.0, 4.5]);
    for x in 3..=7 {
        for z in 5..=7 {
            for y in 201..=203 {
                if ((x == 3 || x == 7) && (z == 5 || z == 7))
                    || (y == 203 && (x == 3 || x == 7 || z == 5 || z == 7))
                {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    for (y, half) in [(201, "lower"), (202, "upper")] {
        let door = block(&state, "minecraft:oak_door", &[("half", half)]);
        state.world.set_block(5, y, 5, door);
    }
    state.world.fold_overlay();
    certify_here(&mut state, executor, Some("owner"), false, "gate").unwrap();
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .buildings
            .values()
            .next()
            .unwrap()
            .bounds,
        mc_script::ScriptWorldBounds::new([3, 200, 5], [7, 203, 7]).unwrap()
    );
}

#[test]
fn open_work_area_does_not_borrow_another_certificates_workstation() {
    let mut state = test_state();
    let id = town(&mut state, "town", "owner", [8, 201, 8]);
    let executor = player(&mut state, [5.5, 201.0, 5.5]);
    station(&mut state, [4, 201, 4], "minecraft:chest");
    certify_here(&mut state, executor, Some("owner"), false, "storage").unwrap();
    station(&mut state, [6, 201, 4], "minecraft:smithing_table");
    let slot = state.entities.slot_of(executor).unwrap();
    state.entities.set_pos(slot, [8.5, 201.0, 4.5]);
    let refusal = certify_here(&mut state, executor, Some("owner"), false, "mine").unwrap_err();
    assert_eq!(
        refusal,
        crate::settlements::ui_text::text(
            crate::settlements::ui_text::Lang::En,
            crate::settlements::ui_text::Key::InspectNoWorkPoint
        )
    );
    station(&mut state, [10, 201, 4], "minecraft:smithing_table");
    certify_here(&mut state, executor, Some("owner"), false, "mine").unwrap();
    let mine = state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .values()
        .find(|building| building.kind == BuildingKind::Mine)
        .unwrap();
    assert_eq!(mine.work_points, [[10, 201, 4]]);
}

#[test]
fn non_operator_wall_selection_inside_a_room_does_not_claim_the_whole_room() {
    let mut state = test_state();
    let id = town(&mut state, "town", "owner", [8, 201, 8]);
    crate::settlements::building_discovery::tests::room(&mut state, [3, 3], [7, 7], true);
    let executor = player(&mut state, [5.5, 201.0, 5.5]);
    certify_here(&mut state, executor, Some("owner"), false, "wall").unwrap();
    let wall = state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .values()
        .next()
        .unwrap();
    assert_eq!(
        wall.bounds,
        mc_script::ScriptWorldBounds::new([3, 200, 3], [7, 204, 3]).unwrap()
    );
}

#[test]
fn non_operator_gate_selection_inside_a_room_finds_the_nearby_frame_not_the_room() {
    let mut state = test_state();
    let id = town(&mut state, "town", "owner", [8, 201, 8]);
    crate::settlements::building_discovery::tests::room(&mut state, [3, 9], [7, 13], true);
    for y in 201..=202 {
        state.world.set_block(3, y, 11, state.registry.states.stone);
    }
    for x in 3..=7 {
        for z in 5..=7 {
            for y in 201..=203 {
                if ((x == 3 || x == 7) && (z == 5 || z == 7))
                    || (y == 203 && (x == 3 || x == 7 || z == 5 || z == 7))
                {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    for (y, half) in [(201, "lower"), (202, "upper")] {
        let door = block(&state, "minecraft:oak_door", &[("half", half)]);
        state.world.set_block(5, y, 7, door);
    }
    state.world.fold_overlay();
    let executor = player(&mut state, [5.5, 201.0, 10.5]);
    certify_here(&mut state, executor, Some("owner"), false, "gate").unwrap();
    let gate = state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .values()
        .next()
        .unwrap();
    assert_eq!(
        gate.bounds,
        mc_script::ScriptWorldBounds::new([3, 200, 5], [7, 203, 7]).unwrap()
    );
}

fn certify_house_with_floor_surface(surface: &str, feet_y: f64, floor_y: i32) {
    let mut state = test_state();
    let id = town(&mut state, "town", "owner", [8, 201, 8]);
    crate::settlements::building_discovery::tests::room(&mut state, [3, 3], [7, 7], true);
    crate::settlements::allocator::tests::bed(&mut state, 5, 5);
    if surface == "carpet" {
        let carpet = block(&state, "minecraft:red_carpet", &[]);
        state.world.set_block(4, 201, 4, carpet);
    } else {
        let slab = block(&state, "minecraft:stone_slab", &[("type", surface)]);
        for x in 3..=7 {
            for z in 3..=7 {
                state.world.set_block(x, 200, z, slab);
                for y in 193..=199 {
                    state.world.set_block(
                        x,
                        y,
                        z,
                        if surface == "top" {
                            state.registry.states.air
                        } else {
                            state.registry.states.stone
                        },
                    );
                }
            }
        }
    }
    state.world.fold_overlay();
    let expected = mc_script::ScriptWorldBounds::new([3, floor_y, 3], [7, 204, 7]).unwrap();
    // The unchanged native validator accepts the actual structural floor.
    crate::settlements::buildings::inspect(&state, BuildingKind::House, 1, expected).unwrap();
    let executor = player(&mut state, [4.5, feet_y, 4.5]);
    certify_here(&mut state, executor, Some("owner"), false, "house").unwrap();
    let buildings = &state.settlements.get(id).unwrap().buildings;
    assert_eq!(buildings.len(), 1);
    assert_eq!(buildings.values().next().unwrap().bounds, expected);
}

#[test]
fn non_operator_house_upper_slab_floor_over_air_certifies() {
    certify_house_with_floor_surface("top", 201.0, 200);
}

#[test]
fn non_operator_house_bottom_slab_floor_on_ground_still_certifies() {
    certify_house_with_floor_surface("bottom", 200.5, 199);
}

#[test]
fn non_operator_house_carpet_above_solid_floor_still_certifies() {
    certify_house_with_floor_surface("carpet", 201.0625, 200);
}
