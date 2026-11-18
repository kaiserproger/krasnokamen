use super::tests::{fixture, fund, open_plan, start};
use super::*;

// The acceptance window is part of the seed-24 regression, not its implementation.
const BUILDER_STALL_SHIFT_TICKS: u64 = 3_000;

/// A storage lot of the street plan and its index in the frontage list the
/// planner's cursor walks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Site {
    pub(crate) front: u32,
    pub(crate) lot: layout::Lot,
    pub(crate) size: [u8; 3],
}

impl Site {
    pub(crate) fn within(&self, x: i32, z: i32) -> bool {
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
pub(crate) fn storage_lots(state: &TickState, id: SettlementId) -> Vec<Site> {
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
pub(crate) fn street_plateau(state: &mut TickState, id: SettlementId, open: &[Site]) {
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

/// The planner's ranking key: cost, anchor distance, origin, rotation.
type RankKey = (u32, i64, [i32; 3], u8);

/// The lot the planner must pick: cheapest cut+fill; ties go to the lot whose
/// origin is nearest the anchor, then to block position and rotation.
pub(super) fn ranking_lot(state: &TickState, id: SettlementId, blueprint_name: &str) -> Site {
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, blueprint_name).unwrap();
    let mut network = layout::Network::for_construction(state, record);
    let fronts = network.frontage();
    let near = |origin: [i32; 3]| -> i64 {
        (0..3)
            .map(|axis| (i64::from(origin[axis]) - i64::from(record.anchor[axis])).pow(2))
            .sum()
    };
    let mut best: Option<(RankKey, Site)> = None;
    for (front, (at, normal)) in fronts.into_iter().enumerate() {
        let Some(lot) = network.lot(blueprint, at, normal) else {
            continue;
        };
        let size = blueprint.footprint(Rotation::from_quarter_turns(lot.rotation));
        let floors = site_refusals::floor_columns(blueprint, lot.rotation);
        let Ok(cost) = site_refusals::check(
            state,
            record,
            lot.origin,
            size,
            record.founding.as_ref(),
            &mut Default::default(),
            Some(&floors),
        ) else {
            continue;
        };
        let key = (cost.total(), near(lot.origin), lot.origin, lot.rotation);
        if best.as_ref().is_none_or(|(known, _)| key < *known) {
            best = Some((
                key,
                Site {
                    front: u32::try_from(front).unwrap(),
                    lot,
                    size,
                },
            ));
        }
    }
    best.expect("a legal lot").1
}

/// The first legal lot of `blueprint_name` in walking order from the plaza:
/// the lot a test authored its fixture around before lots were ranked.
pub(super) fn first_legal_lot(state: &TickState, id: SettlementId, blueprint_name: &str) -> Site {
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, blueprint_name).unwrap();
    let mut network = layout::Network::for_construction(state, record);
    let fronts = network.frontage();
    fronts
        .into_iter()
        .enumerate()
        .find_map(|(front, (at, normal))| {
            let lot = network.lot(blueprint, at, normal)?;
            let size = blueprint.footprint(Rotation::from_quarter_turns(lot.rotation));
            site_refusals::check(
                state,
                record,
                lot.origin,
                size,
                record.founding.as_ref(),
                &mut Default::default(),
                Some(&site_refusals::floor_columns(blueprint, lot.rotation)),
            )
            .is_ok()
            .then(|| Site {
                front: u32::try_from(front).unwrap(),
                lot,
                size,
            })
        })
        .expect("a legal lot in walking order")
}

/// Leave `keep` the only legal lot of `blueprint_name`: one column of every
/// other frontage lot, outside `keep`'s footprint and as far from it as that
/// lot reaches, gets a dirt-path floor the survey refuses as `ProtectedFloor`.
/// The ground stays walkable and the test's own terrain is untouched; the
/// ranked sweep must then agree that `keep` is the winner.
pub(super) fn pin_lot(state: &mut TickState, id: SettlementId, blueprint_name: &str, keep: &Site) {
    let record = state.settlements.get(id).unwrap().clone();
    let blueprint = blueprints::get(&state.registry, blueprint_name).unwrap();
    let mut network = layout::Network::for_construction(state, &record);
    let keep_at = (keep.lot.origin, keep.lot.rotation);
    let [kx, _, kz] = keep.lot.origin;
    let (kx1, kz1) = (
        kx + i32::from(keep.size[0]) - 1,
        kz + i32::from(keep.size[2]) - 1,
    );
    let mut marks = Vec::new();
    for (at, normal) in network.frontage() {
        let Some(lot) = network.lot(blueprint, at, normal) else {
            continue;
        };
        if (lot.origin, lot.rotation) == keep_at {
            continue;
        }
        let size = blueprint.footprint(Rotation::from_quarter_turns(lot.rotation));
        let [ox, floor, oz] = lot.origin;
        let mark = (ox..ox + i32::from(size[0]))
            .flat_map(|x| (oz..oz + i32::from(size[2])).map(move |z| (x, z)))
            .filter(|&(x, z)| !keep.within(x, z))
            .max_by_key(|&(x, z)| (kx - x).max(x - kx1).max(kz - z).max(z - kz1))
            .expect("a lot of the same blueprint is never inside the kept one");
        marks.push([mark.0, floor, mark.1]);
    }
    drop(network);
    let path = crate::settlements::allocator::tests::block(state, "minecraft:dirt_path", &[]);
    for [x, y, z] in marks {
        state.world.set_block(x, y, z, path);
    }
    state.world.fold_overlay();
    let winner = ranking_lot(state, id, blueprint_name);
    assert_eq!(
        (winner.lot.origin, winner.lot.rotation),
        keep_at,
        "the ranked sweep must take the pinned lot"
    );
}

/// Seed 87122 claim QA: a storage ordered at the anchor was sited 110 blocks
/// away, and 21 earlier orders were refused. Two free lots exist; the order
/// scans the loaded plan once and takes one of them: as both cost nothing, the
/// one the stable tie key ranks first.
#[test]
fn owner_order_takes_the_cheapest_free_lot() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    let (near, far) = near_and_far(&state, id);
    street_plateau(&mut state, id, &[near, far]);
    let expected = ranking_lot(&state, id, "storage");
    request_build(&mut state, id, "storage").unwrap();
    let project = &state.settlements.get(id).unwrap().projects[0];
    assert_eq!(
        (project.origin, project.rotation),
        (expected.lot.origin, expected.lot.rotation),
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
    let project = &state.settlements.get(id).unwrap().projects[0];
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
    start(&mut state, id, "storage", 1, None);
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
    // An unbound resident is nobody's incumbent (D8): the planner binds the one
    // who can reach storage, and the walled one holds no project.
    assert_eq!(profession(&state, free), Profession::Builder);
    assert!(!access::bound_builder(
        &state.settlements,
        state.entities.row(trapped).uuid
    ));
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
    // Builder rotation continues only for paid work: seed 24's unpaid clear
    // stage now gives back its slot after one failed rotation, which lets the
    // waiting House start (SETTLEMENT-ROAD-STALL-HOLDS-SLOT-TRIAL4).
    let structure = state
        .settlements
        .get(id)
        .unwrap()
        .projects
        .first()
        .cloned()
        .unwrap()
        .structure;
    let mut ledger = state
        .plugins
        .get_domain("@native-structures", &format!("structure/{structure}"))
        .unwrap()
        .clone();
    ledger["snapshot"]["consumed"] =
        serde_json::json!([{"resource": "minecraft:oak_log", "quantity": 1}]);
    state.plugins.set_domain(
        "@native-structures",
        &format!("structure/{structure}"),
        ledger,
    );
    let recruit = |state: &mut TickState, x: f64| {
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
    // The first ramp lot in walking order is the one this hillside is about;
    // every other lot is refused, so the ranked sweep has to take it.
    let site = first_legal_lot(&state, id, "storage");
    pin_lot(&mut state, id, "storage", &site);
    for _ in 0..128 {
        plan(&mut state, id);
        if state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .is_some()
        {
            break;
        }
    }
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .projects
        .first()
        .cloned()
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
    // Terrain cuts credit their own mining drop, so cutting grass or dirt pays
    // dirt back into storage: count that before the build changes the world.
    let credited: i32 = {
        let ledger = state
            .plugins
            .get_domain(
                "@native-structures",
                &format!("structure/{}", project.structure),
            )
            .unwrap();
        ledger["cells"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|stage| stage.as_array().unwrap())
            .filter(|cell| {
                u32::try_from(cell["state"].as_u64().unwrap()).unwrap() == state.registry.states.air
            })
            .filter(|cell| {
                let at: [i32; 3] = serde_json::from_value(cell["at"].clone()).unwrap();
                matches!(
                    block_name(&state, at[0], at[1], at[2]),
                    "grass_block" | "dirt"
                )
            })
            .count() as i32
    };
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
        if state.settlements.get(id).unwrap().projects.is_empty() {
            break;
        }
        crate::villager::brain::tick(&mut state, &mut changes);
        state.world.fold_overlay();
    }
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
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
    assert_eq!(
        stored + carried + bill as i32,
        64 + credited,
        "foundation dirt is bought exactly once; cuts only credit their own drop"
    );
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
    use super::tests::complete;
    let (mut state, id, builder) = fixture();
    let materials = fund(&mut state, "storage", 1);
    start(&mut state, id, "storage", 1, None);
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    let snapshot = executor::status(&state, id, &project.structure).unwrap();
    assert_eq!(snapshot.origin, project.origin);
    assert_eq!(
        cell_bill(
            &state,
            &planned_cells(&state, &project, "oak", &mut Default::default())
                .unwrap()
                .1
        )
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
fn terrain_preparation_accepts_a_bounded_pillar_and_rejects_protected_cells() {
    let (mut state, id, _) = fixture();
    let origin = [5, 200, 5];
    let size = [3, 5, 3];
    // A pillar inside the authored volume is a cut this window owns.
    for y in 201..=204 {
        state.world.set_block(6, y, 6, state.registry.states.stone);
    }
    assert!(site_free(
        &state,
        state.settlements.get(id).unwrap(),
        origin,
        size
    ));
    // One block higher than the roof is not: the window never cuts above it.
    state
        .world
        .set_block(6, 205, 6, state.registry.states.stone);
    assert!(!site_free(
        &state,
        state.settlements.get(id).unwrap(),
        origin,
        size
    ));
    for y in 201..=205 {
        state.world.set_block(6, y, 6, state.registry.states.air);
    }
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

/// B-min replaces the founding-material skip: the near lot is admitted on its
/// authored bill and its foundation is the project's own waiting shortfall.
#[test]
fn autonomous_admission_takes_the_level_lot_over_a_foundation_lot() {
    let (mut state, id, near, far) = unfunded_near_foundation();
    for _ in 0..32 {
        plan(&mut state, id);
        if state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .is_some()
        {
            break;
        }
    }
    let project = &state.settlements.get(id).unwrap().projects[0];
    assert_eq!(
        (project.origin, project.rotation),
        (far.lot.origin, far.lot.rotation),
        "the level lot is cheaper; the near lot at {:?} needs foundation dirt",
        near.lot.origin
    );
}

#[test]
fn owner_order_takes_the_cheaper_level_lot_over_a_foundation_lot() {
    let (mut state, id, _, far) = unfunded_near_foundation();
    // The owner order scans the whole plan once and takes the level lot.
    request_build(&mut state, id, "storage").expect("a whole sweep admits the level lot");
    let project = &state.settlements.get(id).unwrap().projects[0];
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
    assert!(state.settlements.get(id).unwrap().projects.is_empty());

    fund(&mut state, "storage", 1);
    // The street below is authored around the first free lot in walking
    // order: it fronts the avenue south of the plaza across its west verge,
    // and the door's road is the avenue column two blocks west. Every other
    // lot is refused, so the ranked sweep has to take this one.
    let site = first_legal_lot(&state, id, "storage");
    assert_eq!((site.lot.origin, site.lot.rotation), ([11, 200, 13], 1));
    pin_lot(&mut state, id, "storage", &site);
    request_build(&mut state, id, "storage").unwrap();
    let storage = state.settlements.get(id).unwrap().projects[0].clone();
    complete(&mut state, id, slot);
    assert_eq!((storage.origin, storage.rotation), ([11, 200, 13], 1));
    let mut stretches = 0;
    loop {
        plan(&mut state, id);
        let Some(project) = state.settlements.get(id).unwrap().projects.first().cloned() else {
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
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
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
        .projects
        .first()
        .cloned()
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
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
    let report = layout::report(&state, state.settlements.get(id).unwrap());
    assert!(report.contains("Lamps: 1 standing"), "{report}");
    assert!(
        report.contains("Buildings: 1/1 face a reachable road."),
        "{report}"
    );
    assert!(report.contains("Next street work: none."), "{report}");

    // Seed-100 trial 2: a committed free-paving envelope still contained
    // unpaid soil after the reachable street columns changed.
    state.world.set_block(8, 200, 12, grass);
    state.world.fold_overlay();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let images = executor::source_images(&state, id);
    let site = street_site(&state, state.settlements.get(id).unwrap(), &images)
        .expect("spent free paving must not reserve the road against later paving");
    assert_eq!(site.project.blueprint, layout::ROAD);
    assert!(begin_cells(
        &mut state,
        site.project,
        site.bounds,
        site.cells
    ));
    state.entities.set_pos(slot, [1.5, 201.0, 2.5]);
    assign_builder(&mut state, id);
    complete(&mut state, id, slot);
    assert_eq!(block_name(&state, 8, 200, 12), "dirt_path");
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
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
        let project = &state.settlements.get(id).unwrap().projects[0];
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
        state.settlements.get(id).unwrap().projects[0].blueprint,
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
    stock_seeds(&mut state, 12);
    request_build(&mut state, id, "storage").unwrap();
    super::tests::complete(&mut state, id, slot);
    plan(&mut state, id);
    assert_eq!(
        state.settlements.get(id).unwrap().projects[0].blueprint,
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

/// Grass and short grass on every fixture column but the stocked chest's.
fn meadow_fixture() -> (TickState, SettlementId, crate::entities::EntitySlot) {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    let grass = crate::settlements::allocator::tests::block(&state, "minecraft:grass_block", &[]);
    let plant = crate::settlements::allocator::tests::block(&state, "minecraft:short_grass", &[]);
    for x in -16i32..32 {
        for z in -16i32..32 {
            if (x, z) != (1, 1) {
                state.world.set_block(x, 200, z, grass);
                state.world.set_block(x, 201, z, plant);
            }
        }
    }
    state.world.fold_overlay();
    fund(&mut state, "storage", 1);
    request_build(&mut state, id, "storage").unwrap();
    super::tests::complete(&mut state, id, slot);
    (state, id, slot)
}

/// Seed-24 civilization-seed-qa (ISSUE-1-17 births): the first Farm needs 12
/// wheat seeds and nothing but a farm produced them; the few seeds that
/// street and site clearing happened to drop never reached the bill. A town
/// short of the seeds now spends its free Builder time clearing seed-bearing
/// grass near the bell, and the vanilla drops fill the stock.
#[test]
fn a_town_short_of_wheat_seeds_clears_a_meadow_for_its_first_farm() {
    let (mut state, id, slot) = meadow_fixture();
    plan(&mut state, id);
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .map(|project| project.blueprint.as_str()),
        Some(layout::MEADOW),
        "no building is funded, so the free time gathers seeds"
    );
    let before = stored_seeds(&state);
    let mut cleared = 0;
    for _ in 0..16 {
        super::tests::complete(&mut state, id, slot);
        plan(&mut state, id);
        cleared += 1;
        if stored_seeds(&state) >= 12 {
            break;
        }
    }
    assert!(
        stored_seeds(&state) > before,
        "{cleared} meadow clearings stored no wheat seeds"
    );
}

#[test]
fn a_town_with_the_farms_wheat_seeds_in_stock_clears_no_meadow() {
    let (mut state, id, _) = meadow_fixture();
    stock_seeds(&mut state, 12);
    plan(&mut state, id);
    assert_ne!(
        state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .map(|project| project.blueprint.as_str()),
        Some(layout::MEADOW)
    );
}

fn wheat_seeds(state: &TickState) -> u32 {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:wheat_seeds").unwrap())
        .unwrap()
}

/// Wheat seeds in every block-entity inventory.
fn stored_seeds(state: &TickState) -> i32 {
    let item = wheat_seeds(state);
    state
        .block_entities
        .values()
        .flat_map(|block| &block.slots)
        .filter(|stack| stack.item_id == item)
        .map(|stack| stack.count)
        .sum()
}

/// Put `count` seeds in the fixture chest's last slot.
fn stock_seeds(state: &mut TickState, count: i32) {
    let item = wheat_seeds(state);
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    let last = slots.len() - 1;
    slots[last] = mc_data::ItemStack::new(item, count);
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
        if state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .is_some()
        {
            break;
        }
    }
    let project = &state.settlements.get(id).unwrap().projects[0];
    assert_eq!(project.blueprint, "mason");
    assert_eq!(project.origin, site.lot.origin);
}

#[test]
fn replant_ignores_a_distant_cold_town_but_preserves_an_adjacent_towns_corridor() {
    use crate::settlements::forestry;
    let (mut state, a, _) = fixture();
    forestry::tests::grown_world(&mut state, vec![[168, 201, 8]], "oak");
    state
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter((9..=11).flat_map(|x| {
            (-1..=1).map(move |z| (crate::boundary::ChunkPos { x, z }, 0))
        })));
    let b = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "neighbour".into(),
            "other".into(),
            [168, 201, 8],
            12,
        )
        .unwrap();
    let root = [-8, 201, 8];
    let radius = forestry::PLANTED_CROWN_RADIUS as i32;
    let crown = ScriptWorldBounds::new(
        [root[0] - radius, root[1], root[2] - radius],
        [root[0] + radius, root[1] + 64, root[2] + radius],
    )
    .unwrap();
    let misses = state.settlements.forest.lock().unwrap().misses();
    assert!(
        !replant_conflicts(&state, root),
        "far B cannot veto A's safe renewal"
    );
    assert_eq!(
        state.settlements.forest.lock().unwrap().misses(),
        misses,
        "A's admission must not request evidence for B's cold grove"
    );
    // Prove the cold foreign town would refuse this unrelated planting if
    // queried: it has no proved house lot and needs fresh forest evidence.
    assert!(layout::planting_blocks_access(
        &state,
        state.settlements.get(b).unwrap(),
        crown
    ));
    assert!(state.settlements.forest.lock().unwrap().misses() > misses);

    let town = state.settlements.get_mut(b).unwrap();
    town.anchor = root;
    let building = BuildingId((b.0 << 32) | 1);
    town.buildings.insert(
        building,
        buildings::Building {
            id: building,
            settlement: b,
            kind: BuildingKind::Storage,
            level: 1,
            bounds: ScriptWorldBounds::new([-2, 200, 7], [0, 204, 9]).unwrap(),
            work_points: vec![[-1, 201, 8]],
            revision: 1,
            natural_zone: false,
        },
    );
    assert!(
        !layout::Network::walk(&state, state.settlements.get(b).unwrap())
            .served()
            .is_empty(),
        "B has a reachable standing-building corridor before the proposed crown"
    );
    assert!(!layout::planting_blocks_access(
        &state,
        state.settlements.get(a).unwrap(),
        crown
    ));
    assert!(
        layout::planting_blocks_access(&state, state.settlements.get(b).unwrap(), crown),
        "the adjacent crown covers B's plaza and cuts its building off"
    );
    assert!(
        replant_conflicts(&state, root),
        "nearby B's access still vetoes A's planting"
    );
}
