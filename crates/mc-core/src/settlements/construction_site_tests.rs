use super::tests::{fixture, fund, open_plan};
use super::*;

/// A storage lot of the street plan and its index in the frontage list the
/// planner's cursor walks.
#[derive(Debug, Clone, Copy)]
struct Site {
    front: u32,
    lot: layout::Lot,
    size: [u8; 3],
}

impl Site {
    fn within(&self, x: i32, z: i32) -> bool {
        let [ox, _, oz] = self.lot.origin;
        (ox..ox + i32::from(self.size[0])).contains(&x)
            && (oz..oz + i32::from(self.size[2])).contains(&z)
    }

    /// Footprints at least one block apart.
    fn apart(&self, other: &Self) -> bool {
        [0, 2].into_iter().any(|axis| {
            let span = |site: &Self| {
                let min = site.lot.origin[axis];
                (min, min + i32::from(site.size[axis]) - 1)
            };
            let ((a0, a1), (b0, b1)) = (span(self), span(other));
            a1 + 1 < b0 || b1 + 1 < a0
        })
    }

    /// Wholly inside the three-by-three chunks `open_plan` loads.
    fn loaded(&self) -> bool {
        let [ox, _, oz] = self.lot.origin;
        ox >= -16
            && oz >= -16
            && ox + i32::from(self.size[0]) <= 32
            && oz + i32::from(self.size[2]) <= 32
    }
}

/// Storage lots of the live street plan, in walking order from the plaza.
fn storage_lots(state: &TickState, id: SettlementId) -> Vec<Site> {
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let mut network = layout::Network::walk(state, record);
    network
        .frontage()
        .into_iter()
        .enumerate()
        .filter_map(|(front, (at, normal))| {
            let lot = network.lot(blueprint, at, normal)?;
            Some(Site {
                front: u32::try_from(front).unwrap(),
                lot,
                size: blueprint.footprint(Rotation::from_quarter_turns(lot.rotation)),
            })
        })
        .collect()
}

/// Solid stone through the terrain-following band on every lot column except
/// the fixture chest and the given sites: roads, verges and the plaza stay
/// walkable, so a site exists only where a test leaves one.
fn street_plateau(state: &mut TickState, id: SettlementId, open: &[Site]) {
    let anchor = state.settlements.get(id).unwrap().anchor;
    for x in -16i32..32 {
        for z in -16i32..32 {
            if layout::cell(anchor, x, z) == layout::Cell::Lot
                && (x, z) != (1, 1)
                && !open.iter().any(|site| site.within(x, z))
            {
                for y in 201..=216 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
}

/// On the flat open plan: the first lot in walking order clear of the
/// fixture chest, and the last loaded lot apart from it.
fn near_and_far(state: &TickState, id: SettlementId) -> (Site, Site) {
    let lots: Vec<Site> = storage_lots(state, id)
        .into_iter()
        .filter(|site| !site.within(1, 1))
        .collect();
    let near = lots[0];
    let far = *lots
        .iter()
        .rev()
        .find(|site| site.loaded() && site.apart(&near))
        .expect("the open plan holds a second lot");
    assert!(near.front < far.front);
    (near, far)
}

/// Seed 87122 claim QA: a storage ordered at the anchor was sited 110 blocks
/// away, and 21 earlier orders were refused. Two free lots exist; the order
/// takes the one first in walking order from the plaza.
#[test]
fn owner_order_takes_the_first_free_lot_in_walking_order() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    let (near, far) = near_and_far(&state, id);
    street_plateau(&mut state, id, &[near, far]);
    request_build(&mut state, id, "storage").unwrap();
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    assert_eq!(
        (project.origin, project.rotation),
        (near.lot.origin, near.lot.rotation),
        "not the lot at {far:?}"
    );
}

/// Every planned lot lies wholly on lot columns, outside roads, verges and
/// the plaza, with the entrance facing the road it fronts across the verge;
/// all four road sides carry lots.
#[test]
fn planned_lots_front_reachable_roads_and_never_cover_streets() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let anchor = state.settlements.get(id).unwrap().anchor;
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let lots = storage_lots(&state, id);
    let sides: std::collections::BTreeSet<u8> = lots.iter().map(|site| site.lot.rotation).collect();
    assert_eq!(sides.len(), 4, "{lots:?}");
    for site in &lots {
        let [ox, _, oz] = site.lot.origin;
        for x in ox..ox + i32::from(site.size[0]) {
            for z in oz..oz + i32::from(site.size[2]) {
                assert_eq!(layout::cell(anchor, x, z), layout::Cell::Lot, "{site:?}");
            }
        }
        let entrance = blueprint
            .level(1)
            .unwrap()
            .rotated(Rotation::from_quarter_turns(site.lot.rotation))
            .entrance;
        let door = [ox + i32::from(entrance[0]), oz + i32::from(entrance[2])];
        // Two steps out of the entrance edge reach a road column.
        assert!(
            [[2, 0], [-2, 0], [0, 2], [0, -2]]
                .into_iter()
                .any(|[dx, dz]| layout::cell(anchor, door[0] + dx, door[1] + dz)
                    == layout::Cell::Road),
            "{site:?} entrance {door:?} faces no road"
        );
    }
}

/// A cluttered village: a pillar every third lot column leaves no
/// footprint-sized gap except one clearing on the avenue south of the plaza.
/// One owner order finds it at once and faces the avenue.
#[test]
fn owner_order_in_a_cluttered_village_finds_the_clearing_at_once() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    let anchor = state.settlements.get(id).unwrap().anchor;
    let clearing = |x: i32, z: i32| (11..=22).contains(&x) && (13..=24).contains(&z);
    for x in -16i32..32 {
        for z in -16i32..32 {
            if layout::cell(anchor, x, z) == layout::Cell::Lot
                && (x, z) != (1, 1)
                && !clearing(x, z)
                && (x.rem_euclid(3) == 0 || z.rem_euclid(3) == 0)
            {
                for y in 201..=216 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    request_build(&mut state, id, "storage").unwrap();
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    let size = blueprints::get(&state.registry, "storage")
        .unwrap()
        .footprint(Rotation::from_quarter_turns(project.rotation));
    for x in project.origin[0]..project.origin[0] + i32::from(size[0]) {
        for z in project.origin[2]..project.origin[2] + i32::from(size[2]) {
            assert!(clearing(x, z), "{:?} leaves the clearing", project.origin);
        }
    }
    assert_eq!(project.rotation, 1, "front faces the avenue to the west");
}

fn profession(state: &TickState, slot: crate::entities::EntitySlot) -> Profession {
    state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .data
        .profession
}

/// Seed 87122 autonomy QA: the storage project was admitted at once, but its
/// builder woke inside a house whose door step was roofed by a tree crown. It
/// stood in the doorway for 7000 daytime ticks, and no resident replaced it
/// because a builder, once chosen, was never re-checked. A builder on shift
/// that cannot walk to storage is released and a resident who can takes over.
#[test]
fn trapped_builder_on_shift_is_replaced_by_a_resident_who_reaches_storage() {
    use crate::settlements::allocator::tests::villager;
    let (mut state, id, trapped) = fixture();
    fund(&mut state, "storage", 1);
    // Shut the fixture's builder in a stone cell at (10, 201..202, 10).
    for x in 9..=11 {
        for z in 9..=11 {
            for y in 201..=203 {
                if (x, z) != (10, 10) || y == 203 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    state.entities.set_pos(trapped, [10.5, 201.0, 10.5]);
    let free = villager(&mut state, [4.5, 201.0, 4.5]);
    let entity = state.entities.body(free).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    assert_eq!(profession(&state, trapped), Profession::Builder);
    assign_builder(&mut state, id);
    assert_eq!(profession(&state, trapped), Profession::None);
    assert_eq!(profession(&state, free), Profession::Builder);
}

/// Seed 24 (CIV-SEED24-STALLED-PROJECT-HOLDS-SLOT): a stationary zombie eight
/// blocks from the only road to the Lumber camp lot made its builder flee and
/// return for 15,000 ticks at 35 of 239 cells, while the funded House waited
/// for the single project slot. A builder that has not advanced the project in
/// 3000 shift ticks is replaced by another resident, and a replaced builder is
/// chosen last for a day.
#[test]
fn builder_that_cannot_advance_the_project_is_replaced_and_chosen_last() {
    use crate::settlements::allocator::tests::villager;
    let (mut state, id, stuck) = fixture();
    fund(&mut state, "storage", 1);
    super::tests::start(&mut state, id, "storage", 1, None);
    let mut recruit = |state: &mut TickState, x: f64| {
        let slot = villager(state, [x, 201.0, 4.5]);
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(state, id, entity).unwrap();
        slot
    };
    let second = recruit(&mut state, 4.5);
    let third = recruit(&mut state, 6.5);
    // The planning cadence: one pass every 20 ticks of shift (day time 3000).
    let pass = |state: &mut TickState, ticks: u64| {
        for _ in 0..ticks / 20 {
            state.tick += 20;
            assign_builder(state, id);
        }
    };
    state.tick = 100;
    assign_builder(&mut state, id);
    assert_eq!(profession(&state, stuck), Profession::Builder);
    assert_eq!(profession(&state, second), Profession::None);
    pass(&mut state, BUILDER_STALL_SHIFT_TICKS - 40);
    assert_eq!(
        profession(&state, stuck),
        Profession::Builder,
        "less than the stall window without progress is not a stall"
    );
    // Night ticks are not shift ticks: nobody is expected to build.
    state.day_time = 15_000;
    pass(&mut state, 2 * BUILDER_STALL_SHIFT_TICKS);
    assert_eq!(profession(&state, stuck), Profession::Builder);
    state.day_time = 3000;
    pass(&mut state, 60);
    assert_eq!(profession(&state, stuck), Profession::None);
    assert_eq!(profession(&state, second), Profession::Builder);
    // The replacement stalls too: the resident not yet tried takes over, not
    // the first builder again.
    pass(&mut state, BUILDER_STALL_SHIFT_TICKS + 20);
    assert_eq!(profession(&state, stuck), Profession::None);
    assert_eq!(profession(&state, second), Profession::None);
    assert_eq!(profession(&state, third), Profession::Builder);
}

/// The re-check never costs a working builder its slot, and an off-shift
/// builder (asleep behind a closed door at night) is left alone.
#[test]
fn reachable_or_off_shift_builder_keeps_the_slot() {
    use crate::settlements::allocator::tests::villager;
    let (mut state, id, builder) = fixture();
    let free = villager(&mut state, [4.5, 201.0, 4.5]);
    let entity = state.entities.body(free).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    assign_builder(&mut state, id);
    assert_eq!(profession(&state, builder), Profession::Builder);
    assert_eq!(profession(&state, free), Profession::None);
    for x in 9..=11 {
        for z in 9..=11 {
            for y in 201..=203 {
                if (x, z) != (10, 10) || y == 203 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    state.entities.set_pos(builder, [10.5, 201.0, 10.5]);
    state.day_time = 15_000;
    assign_builder(&mut state, id);
    assert_eq!(profession(&state, builder), Profession::Builder);
}

/// Deterministic terrain, not an operator-built pad: behind a level front
/// lip, every lot east of the avenue is a 198..=201 diagonal ramp, so each
/// footprint there crosses both low and high columns. Roads, verges, the
/// plaza and the western blocks stay level stone and dirt at 200.
#[test]
fn hillside_village_prepares_and_builds_first_storage() {
    use super::tests::complete;
    use mc_data::ItemStack;
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    // The lot's east and south work sides lie in the neighbouring chunks, as
    // a player near the bell keeps them simulated. A builder that walks into
    // an unsimulated chunk freezes there and loses the builder slot.
    for x in -1..=1 {
        for z in -1..=1 {
            state.tickets.insert(
                format!("hillside-{x}-{z}"),
                (crate::boundary::ChunkPos { x, z }, 0),
            );
        }
    }
    state.refresh_active_chunks(0);
    fund(&mut state, "storage", 1);
    let anchor = state.settlements.get(id).unwrap().anchor;
    let lot = |x: i32, z: i32| layout::cell(anchor, x, z) == layout::Cell::Lot;
    let lip = |x: i32, z: i32| {
        [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .any(|(dx, dz)| layout::cell(anchor, x + dx, z + dz) == layout::Cell::Verge)
    };
    let height = |x: i32, z: i32| {
        if lot(x, z) && x > anchor[0] && !lip(x, z) {
            198 + (x + z).rem_euclid(4)
        } else {
            200
        }
    };
    let dirt = crate::settlements::allocator::tests::block(&state, "minecraft:dirt", &[]);
    for x in -16i32..32 {
        for z in -16i32..32 {
            for y in 197..=201 {
                if (x, y, z) != (1, 201, 1) {
                    let block = if y > height(x, z) {
                        state.registry.states.air
                    } else if lot(x, z) {
                        dirt
                    } else {
                        state.registry.states.stone
                    };
                    state.world.set_block(x, y, z, block);
                }
            }
        }
    }
    let dirt_item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:dirt").unwrap())
        .unwrap();
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[26] = ItemStack::new(dirt_item, 64);
    state.world.fold_overlay();
    for _ in 0..128 {
        plan(&mut state, id);
        if state.settlements.get(id).unwrap().project.is_some() {
            break;
        }
    }
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .clone()
        .expect("hillside admits first storage");
    assert!(!project.ordered);
    let snapshot = executor::status(&state, id, &project.structure).unwrap();
    let foundation = snapshot
        .remaining
        .iter()
        .find(|m| m.resource == "minecraft:dirt")
        .expect("paid foundation");
    let bill = foundation.quantity;
    assert!(
        project_summary(&state, id)
            .iter()
            .any(|line| line == &format!("minecraft:dirt: {bill}"))
    );
    assert!(bill > 0);
    let floor = project.origin[1];
    let size = blueprints::get(&state.registry, "storage")
        .unwrap()
        .footprint(Rotation::from_quarter_turns(project.rotation));
    assert!(
        (project.origin[0]..project.origin[0] + i32::from(size[0])).any(|x| {
            (project.origin[2]..project.origin[2] + i32::from(size[2]))
                .any(|z| height(x, z) < floor - 1)
        }) && (project.origin[0]..project.origin[0] + i32::from(size[0])).any(|x| {
            (project.origin[2]..project.origin[2] + i32::from(size[2]))
                .any(|z| height(x, z) > floor)
        }),
        "the first lot crosses the ramp: {:?}",
        project.origin
    );
    // Real brain/navigation, no teleports, operator commands or terrain edits.
    for now in 0..20_000 {
        state.tick = now;
        state.tick_budget.reset(now);
        let mut changes = crate::boundary::ChangeSet::default();
        let writes_before = state.plugin_block_writes.len();
        crate::settlements::tick(&mut state, &mut changes);
        assert!(
            state.plugin_block_writes.len() - writes_before <= 1,
            "preparation and building must use one-cell admissions"
        );
        if state.settlements.get(id).unwrap().project.is_none() {
            break;
        }
        crate::villager::brain::tick(&mut state, &mut changes);
        state.world.fold_overlay();
    }
    assert!(
        state.settlements.get(id).unwrap().project.is_none(),
        "hillside stalled: {:?}; builder={:?}; villager={:?}",
        project_status(&state, id),
        state.entities.pos(builder),
        state.entities.row(builder).villager
    );
    println!(
        "hillside storage completed at tick {}, origin {:?}, foundation dirt {}",
        state.tick, project.origin, bill
    );
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .buildings
            .values()
            .any(|b| b.kind == BuildingKind::Storage)
    );
    for x in project.origin[0]..project.origin[0] + i32::from(size[0]) {
        for z in project.origin[2]..project.origin[2] + i32::from(size[2]) {
            for y in floor + 1..floor + i32::from(size[1]) {
                assert_ne!(
                    state.world.physics_block_at(x, y, z),
                    Some(dirt),
                    "terrain above the floor must be excavated"
                );
            }
            for y in height(x, z) + 1..floor {
                assert_eq!(state.world.physics_block_at(x, y, z), Some(dirt));
            }
            assert!(
                !state.registry.collision_boxes
                    [state.world.physics_block_at(x, floor, z).unwrap() as usize]
                    .is_empty()
            );
        }
    }
    let stored: i32 = state
        .block_entities
        .values()
        .flat_map(|be| &be.slots)
        .filter(|s| s.item_id == dirt_item)
        .map(|s| s.count)
        .sum();
    let carried: i32 = state
        .entities
        .row(builder)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .iter()
        .flatten()
        .filter(|s| s.item_id == dirt_item)
        .map(|s| s.count)
        .sum();
    assert_eq!(stored + carried, 64 - bill as i32);
    let building = state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .values()
        .find(|b| b.kind == BuildingKind::Storage)
        .unwrap()
        .id;
    assert!(upgrade_available(&state, id, building));
    fund(&mut state, "storage", 2);
    request_upgrade(&mut state, id, building).unwrap();
    complete(&mut state, id, builder);
    assert_eq!(
        state.settlements.get(id).unwrap().buildings[&building].level,
        2
    );
}

#[test]
fn flat_site_keeps_bill_origin_and_same_site_upgrade() {
    use super::tests::{complete, start};
    let (mut state, id, builder) = fixture();
    let materials = fund(&mut state, "storage", 1);
    start(&mut state, id, "storage", 1, None);
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
    let snapshot = executor::status(&state, id, &project.structure).unwrap();
    assert_eq!(snapshot.origin, project.origin);
    assert_eq!(
        cell_bill(&state, &planned_cells(&state, &project, "oak").unwrap().1)
            .unwrap()
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>(),
        materials.into_iter().collect()
    );
    complete(&mut state, id, builder);
    let building = state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .values()
        .find(|b| b.kind == BuildingKind::Storage)
        .unwrap()
        .id;
    assert!(executor::built_at(&state, id, "storage", project.origin));
    assert!(!executor::built_at(
        &state,
        id,
        "storage",
        [project.origin[0], project.origin[1] + 1, project.origin[2]]
    ));
    fund(&mut state, "storage", 2);
    request_upgrade(&mut state, id, building).unwrap();
    complete(&mut state, id, builder);
    assert_eq!(
        state.settlements.get(id).unwrap().buildings[&building].level,
        2
    );
}

#[test]
fn terrain_preparation_rejects_four_block_variance_and_protected_cells() {
    let (mut state, id, _) = fixture();
    let origin = [5, 200, 5];
    let size = [3, 5, 3];
    for y in 201..=204 {
        state.world.set_block(6, y, 6, state.registry.states.stone);
    }
    assert!(!site_free(
        &state,
        state.settlements.get(id).unwrap(),
        origin,
        size
    ));
    state.world.set_block(6, 204, 6, state.registry.states.air);
    assert!(site_free(
        &state,
        state.settlements.get(id).unwrap(),
        origin,
        size
    ));
    for name in [
        "minecraft:oak_planks",
        "minecraft:dirt_path",
        "minecraft:water",
    ] {
        let block = crate::settlements::allocator::tests::block(&state, name, &[]);
        state.world.set_block(6, 203, 6, block);
        assert!(
            !site_free(&state, state.settlements.get(id).unwrap(), origin, size),
            "{name}"
        );
    }
}

/// Two free lots: the first in walking order needs one dirt below its
/// authored floor, away from its door column; the stock funds the complete
/// building but contains no foundation dirt. The later lot is level.
fn unfunded_near_foundation() -> (TickState, SettlementId, Site, Site) {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    let (near, far) = near_and_far(&state, id);
    street_plateau(&mut state, id, &[near, far]);
    let (x, z) = (near.lot.origin[0] + 1, near.lot.origin[2] + 1);
    state
        .world
        .set_block(x, 198, z, state.registry.states.stone);
    for y in 199..=200 {
        state.world.set_block(x, y, z, state.registry.states.air);
    }
    state.world.fold_overlay();
    for site in [near, far] {
        assert!(site_free(
            &state,
            state.settlements.get(id).unwrap(),
            site.lot.origin,
            site.size
        ));
    }
    (state, id, near, far)
}

#[test]
fn hillside_review_autonomous_search_skips_unfunded_foundation_within_budget() {
    let (mut state, id, near, far) = unfunded_near_foundation();
    // Each pass examines SITE_ATTEMPTS frontage candidates from the cursor;
    // skipping the unfunded near lot never resets it.
    let mut cursor = 0;
    while cursor <= near.front {
        plan(&mut state, id);
        cursor += SITE_ATTEMPTS;
        let record = state.settlements.get(id).unwrap();
        assert!(
            record.project.is_none(),
            "no free funded lot before {cursor}"
        );
        assert_eq!(
            record.construction_cursor, cursor,
            "unfunded site cannot reset the cursor"
        );
    }
    assert!(
        far.front >= cursor,
        "far lot lies beyond the skipping passes"
    );
    for _ in 0..32 {
        plan(&mut state, id);
        if state.settlements.get(id).unwrap().project.is_some() {
            break;
        }
    }
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .as_ref()
        .expect("far funded site");
    assert_eq!(
        (project.origin, project.rotation),
        (far.lot.origin, far.lot.rotation)
    );
}

#[test]
fn hillside_review_owner_search_skips_unfunded_foundation() {
    let (mut state, id, _, far) = unfunded_near_foundation();
    request_build(&mut state, id, "storage").expect("far funded site");
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    assert_eq!(
        (project.origin, project.rotation),
        (far.lot.origin, far.lot.rotation)
    );
    assert!(project.ordered);
}

#[test]
fn hillside_review_floor_under_overhang_preserves_fluids_and_protected_blocks() {
    let (mut state, id, _) = fixture();
    let origin = [5, 200, 5];
    let size = [3, 5, 3];
    state
        .world
        .set_block(6, 201, 6, state.registry.states.stone);
    assert!(
        site_free(&state, state.settlements.get(id).unwrap(), origin, size),
        "solid natural ground below the overhang is replaceable"
    );
    for name in [
        "minecraft:water",
        "minecraft:lava",
        "minecraft:dirt_path",
        "minecraft:farmland",
        "minecraft:wheat",
        "minecraft:torch",
    ] {
        let block = crate::settlements::allocator::tests::block(&state, name, &[]);
        state.world.set_block(6, 200, 6, block);
        assert!(
            !site_free(&state, state.settlements.get(id).unwrap(), origin, size),
            "protected floor under overhang: {name}"
        );
    }
}

fn block_name(state: &TickState, x: i32, y: i32, z: i32) -> &str {
    state
        .world
        .block_at(x, y, z)
        .and_then(|block| state.registry.blocks.by_id(mc_world::BlockStateId(block)))
        .map_or("unloaded", |definition| definition.block.id.path())
}

/// A standing building gets its street: the planner paves, for free, the
/// stretch on the walk from the plaza to the road its door faces, then funds
/// that stretch's verge lamp post from storage. Streets nobody uses stay as
/// they are, and nothing is planned before a building stands.
#[test]
fn standing_storage_gets_the_street_its_door_faces_paved_then_lit() {
    use super::tests::complete;
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    let anchor = state.settlements.get(id).unwrap().anchor;
    let grass = crate::settlements::allocator::tests::block(&state, "minecraft:grass_block", &[]);
    for x in -16i32..32 {
        for z in -16i32..32 {
            if layout::cell(anchor, x, z) == layout::Cell::Road {
                state.world.set_block(x, 200, z, grass);
            }
        }
    }
    state.world.fold_overlay();
    plan(&mut state, id);
    assert!(state.settlements.get(id).unwrap().project.is_none());

    fund(&mut state, "storage", 1);
    request_build(&mut state, id, "storage").unwrap();
    let storage = state.settlements.get(id).unwrap().project.clone().unwrap();
    complete(&mut state, id, slot);
    // The first free lot fronts the avenue south of the plaza across its
    // west verge; the door's road is the avenue column two blocks west.
    assert_eq!((storage.origin, storage.rotation), ([11, 200, 13], 1));
    let mut stretches = 0;
    loop {
        plan(&mut state, id);
        let Some(project) = state.settlements.get(id).unwrap().project.clone() else {
            break;
        };
        assert_eq!(project.blueprint, layout::ROAD, "free paving comes first");
        assert!(!project.ordered && project.building.is_none());
        complete(&mut state, id, slot);
        stretches += 1;
        assert!(stretches <= 4, "only the walk to the door is paved");
    }
    assert_eq!(
        stretches, 1,
        "the avenue stretch from the plaza to the door"
    );
    for z in 10..=23 {
        for x in 7..=9 {
            assert_eq!(block_name(&state, x, 200, z), "dirt_path", "{x} {z}");
        }
    }
    for (x, z) in [(8, -10), (-10, 8), (20, 8), (8, 28)] {
        assert_eq!(
            block_name(&state, x, 200, z),
            "grass_block",
            "unused street {x} {z}"
        );
    }
    // A paved road reopens: the restored ledger accepts free paving.
    crate::plugin_structures::validate(&state).expect("committed road reopens");

    // The stretch's lamp post waits for its two fences and torch.
    assert!(state.settlements.get(id).unwrap().project.is_none());
    let item = |name: &str| {
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap()
    };
    let (fence, torch) = (item("minecraft:oak_fence"), item("minecraft:torch"));
    let chest = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    chest[25] = mc_data::ItemStack::new(fence, 2);
    chest[26] = mc_data::ItemStack::new(torch, 1);
    plan(&mut state, id);
    let lamp = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .clone()
        .expect("funded lamp post");
    assert_eq!(lamp.blueprint, layout::LAMP);
    assert_eq!(
        lamp.origin,
        [10, 201, 19],
        "the stretch's verge lamp column"
    );
    // The direct-work helper ended in an unticketed neighbour chunk. Return
    // to the active fixture before admitting the next temporary job.
    state.entities.set_pos(slot, [1.5, 201.0, 2.5]);
    assign_builder(&mut state, id);
    complete(&mut state, id, slot);
    assert_eq!(block_name(&state, 10, 201, 19), "oak_fence");
    assert_eq!(block_name(&state, 10, 202, 19), "oak_fence");
    assert_eq!(block_name(&state, 10, 203, 19), "torch");
    crate::plugin_structures::validate(&state).expect("committed lamp post reopens");
    plan(&mut state, id);
    assert!(state.settlements.get(id).unwrap().project.is_none());
    let report = layout::report(&state, state.settlements.get(id).unwrap());
    assert!(report.contains("Lamps: 1 standing"), "{report}");
    assert!(
        report.contains("Buildings: 1/1 face a reachable road."),
        "{report}"
    );
    assert!(report.contains("Next street work: none."), "{report}");
}

/// Seed 24's generated village obstructs the three road columns, but the
/// adjacent verge gives a legal one-block-step walk to the lots beyond.
/// A street walk must use that shoulder, not discard all distant frontage.
#[test]
fn funded_workshops_reach_lots_past_a_road_obstruction_via_the_verge() {
    for name in ["mine", "mason"] {
        let (mut state, id, _) = fixture();
        open_plan(&mut state);
        let blueprint = blueprints::get(&state.registry, name).unwrap();
        let record = state.settlements.get(id).unwrap();
        let mut network = layout::Network::walk(&state, record);
        let lot = network.lot(blueprint, [9, 200, 22], [1, 0]).unwrap();
        let site = Site {
            front: 0,
            size: blueprint.footprint(Rotation::from_quarter_turns(lot.rotation)),
            lot,
        };
        street_plateau(&mut state, id, &[site]);
        // An authored obstacle spans the road, not the walkable verge.
        for x in 7..=9 {
            for y in 201..=203 {
                state.world.set_block(x, y, 13, state.registry.states.stone);
            }
        }
        state.world.fold_overlay();
        fund(&mut state, name, 1);
        request_build(&mut state, id, name).unwrap();
        let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
        assert_eq!(project.blueprint, name);
        assert_eq!(project.origin, site.lot.origin);
        assert_eq!(project.rotation, site.lot.rotation);
        for x in 7..=9 {
            assert_eq!(
                state.world.block_at(x, 203, 13),
                Some(state.registry.states.stone),
                "the route must go around, never clear the obstacle"
            );
        }
    }
}

fn active_street_fixture() -> (TickState, SettlementId, crate::entities::EntitySlot) {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    let anchor = state.settlements.get(id).unwrap().anchor;
    let grass = crate::settlements::allocator::tests::block(&state, "minecraft:grass_block", &[]);
    for x in -16i32..32 {
        for z in -16i32..32 {
            if layout::cell(anchor, x, z) == layout::Cell::Road {
                state.world.set_block(x, 200, z, grass);
            }
        }
    }
    state.world.fold_overlay();
    fund(&mut state, "storage", 1);
    request_build(&mut state, id, "storage").unwrap();
    super::tests::complete(&mut state, id, slot);
    plan(&mut state, id);
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .blueprint,
        layout::ROAD
    );
    (state, id, slot)
}

#[test]
fn street_work_releases_its_builder_after_completion() {
    let (mut state, id, slot) = active_street_fixture();
    let before = state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .clone();
    super::tests::complete(&mut state, id, slot);
    let worker = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!(
        worker.data.profession,
        Profession::None,
        "finished free street work must not retain the only extraction/shearing worker"
    );
    assert_eq!(
        worker.inventory, before,
        "free paving cannot lose or mint carry"
    );
}

/// Seed-24 civilization runs ufgbv77d and fx4gdz87: the Farm stayed unfunded
/// on 'Wheat Seeds 0/12 — needs a farm' while the bread ran out, so no second
/// birth. A farm's wheat cells cost seeds that only a farm produced. Builders
/// clearing plants now get their vanilla loot, as felled trees do: short grass
/// drops wheat seeds one time in eight into real storage.
#[test]
fn street_work_clearing_short_grass_stores_its_vanilla_wheat_seeds() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    let anchor = state.settlements.get(id).unwrap().anchor;
    let grass = crate::settlements::allocator::tests::block(&state, "minecraft:grass_block", &[]);
    let plant = crate::settlements::allocator::tests::block(&state, "minecraft:short_grass", &[]);
    for x in -16i32..32 {
        for z in -16i32..32 {
            if layout::cell(anchor, x, z) == layout::Cell::Road {
                state.world.set_block(x, 200, z, grass);
                state.world.set_block(x, 201, z, plant);
            }
        }
    }
    state.world.fold_overlay();
    fund(&mut state, "storage", 1);
    request_build(&mut state, id, "storage").unwrap();
    super::tests::complete(&mut state, id, slot);
    plan(&mut state, id);
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .blueprint,
        layout::ROAD
    );
    let seeds = |state: &TickState| -> i32 {
        let item = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:wheat_seeds").unwrap())
            .unwrap();
        state
            .block_entities
            .values()
            .flat_map(|block| &block.slots)
            .filter(|stack| stack.item_id == item)
            .map(|stack| stack.count)
            .sum()
    };
    let before = seeds(&state);
    super::tests::complete(&mut state, id, slot);
    let cleared = (-16i32..32)
        .flat_map(|x| (-16i32..32).map(move |z| (x, z)))
        .filter(|&(x, z)| {
            layout::cell(anchor, x, z) == layout::Cell::Road
                && state.world.block_at(x, 201, z) == Some(state.registry.states.air)
        })
        .count();
    assert!(
        cleared >= 16,
        "the street cleared its short grass ({cleared} columns)"
    );
    assert!(
        seeds(&state) > before,
        "clearing {cleared} short grass stored no wheat seeds"
    );
}

#[test]
fn waiting_house_wool_demand_survives_an_active_street_project() {
    let (state, id, _) = active_street_fixture();
    assert!(
        waiting(&state, id)
            .iter()
            .any(|kind| kind.blueprint == "house")
    );
    assert!(
        wool_demand(&state, id),
        "the published unmet house bill still needs wool while a road occupies the project slot"
    );
}

#[test]
fn funded_mine_without_a_site_does_not_starve_a_legal_mason() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    buildings::certify(
        &mut state,
        id,
        BuildingKind::Storage,
        ScriptWorldBounds::new([0, 200, 0], [2, 203, 2]).unwrap(),
    )
    .unwrap();
    let mason = blueprints::get(&state.registry, "mason").unwrap();
    let mut network = layout::Network::walk(&state, state.settlements.get(id).unwrap());
    let lot = network.lot(mason, [9, 200, 22], [1, 0]).unwrap();
    let site = Site {
        front: 0,
        size: mason.footprint(Rotation::from_quarter_turns(lot.rotation)),
        lot,
    };
    street_plateau(&mut state, id, &[site]);
    let mut bill = std::collections::BTreeMap::<u32, u32>::new();
    for name in ["mine", "mason"] {
        for &(item, count) in &blueprints::get(&state.registry, name)
            .unwrap()
            .level(1)
            .unwrap()
            .materials
        {
            let total = bill.entry(item).or_default();
            *total = (*total).max(count);
        }
    }
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    let mut index = 0;
    for (item, mut count) in bill {
        while count > 0 {
            let moving = count.min(64);
            slots[index] = mc_data::ItemStack::new(item, moving as i32);
            index += 1;
            count -= moving;
        }
    }
    state.world.fold_overlay();
    let images = executor::source_images(&state, id);
    for name in ["mine", "mason"] {
        assert!(
            select_species(
                &state,
                &blueprints::get(&state.registry, name)
                    .unwrap()
                    .level(1)
                    .unwrap()
                    .materials,
                &images,
                false,
            )
            .is_some()
        );
    }
    for _ in 0..256 {
        plan(&mut state, id);
        if state.settlements.get(id).unwrap().project.is_some() {
            break;
        }
    }
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .as_ref()
        .expect("later funded kind with a legal site must be admitted");
    assert_eq!(project.blueprint, "mason");
    assert_eq!(project.origin, site.lot.origin);
}
