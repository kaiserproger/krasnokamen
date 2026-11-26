use super::tests::{fixture, fund, open_plan, request_started, start};
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

/// Solid stone above the territory's whole vertical span (the bell's elevation
/// plus its radius, 48) on every lot column except the fixture chest and the
/// given sites: roads, verges and the plaza stay walkable, so a site exists
/// only where a test leaves one. Anything lower would be a hill the builder
/// levels (owner 2026-11-17), no longer a wall.
pub(crate) fn street_plateau(state: &mut TickState, id: SettlementId, open: &[Site]) {
    let anchor = state.settlements.get(id).unwrap().anchor;
    for x in -16i32..32 {
        for z in -16i32..32 {
            if layout::cell(anchor, x, z) == layout::Cell::Lot
                && (x, z) != (1, 1)
                && !open.iter().any(|site| site.within(x, z))
            {
                for y in 201..=250 {
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
/// lot reaches, gets a torch the survey refuses as `ProtectedFloor` (a dirt path
/// is village work now and would not refuse the lot, owner D10).
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
    let path = crate::settlements::allocator::tests::block(state, "minecraft:torch", &[]);
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
/// Agreement guard for the site search: a lot the sweep hands on must be one
/// the plan can actually prepare. `check` cannot model the tree plan, so a lot
/// whose own trees have no walking route used to win the cheapest ranking and be
/// dropped in `materialize` every walk (HANDOFF-51: seed-24 House lot
/// `[246,69,-176]`, `check` true, plan `no_route_with_trees`). The sweep now
/// refuses that lot itself; this guard fails if any admitted lot is one the
/// plan refuses.
#[test]
fn every_lot_the_sweep_admits_can_be_planned() {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    fund(&mut state, "spanish_house", 1);
    start(&mut state, id, "spanish_house", 1, None);
    super::tests::complete(&mut state, id, builder);
    super::growth_tests::stock_buildings(
        &mut state,
        &[("spanish_house", 1), ("farm", 1), ("storage", 1)],
    );
    let record = state.settlements.get(id).unwrap().clone();
    let images = executor::source_images(&state, id);
    let mut checked = 0;
    for name in ["spanish_house", "farm", "storage"] {
        let blueprint = blueprints::get(&state.registry, name).unwrap();
        let Some(site) = site_project(
            &state,
            &record,
            std::iter::once(blueprint),
            &mut site_refusals::Sweeps::default(),
            0,
            u32::MAX,
            &images,
        )
        .site
        else {
            // A kind the plan refuses everywhere simply admits nothing.
            continue;
        };
        checked += 1;
        let mut clearance = super::super::forestry::Clearance::prepared();
        let planned =
            super::preparation::planned_cells(&state, &site.project, "oak", &mut clearance);
        assert!(
            planned.is_ok(),
            "the sweep admitted a {name} lot at {:?} the plan refuses ({:?})",
            site.bounds.min,
            planned.err().map(|_| super::preparation::take_last_stage())
        );
    }
    assert!(checked > 0, "the open plan must admit at least one kind");
}

#[test]
fn owner_order_takes_the_cheapest_free_lot() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    let (near, far) = near_and_far(&state, id);
    street_plateau(&mut state, id, &[near, far]);
    let expected = ranking_lot(&state, id, "storage");
    request_started(&mut state, id, "storage");
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
                for y in 201..=250 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    request_started(&mut state, id, "storage");
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
        if !state.settlements.get(id).unwrap().projects.is_empty() {
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
    tests::upgrade_started(&mut state, id, building);
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
    tests::upgrade_started(&mut state, id, building);
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
    // A dirt path is village work now, not a refusal: the lot re-lays it
    // around the footprint (owner D10, `relayout::plan`). Authored blocks and
    // fluids stay protected floors.
    for name in ["minecraft:oak_planks", "minecraft:water", "minecraft:lava"] {
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
        if !state.settlements.get(id).unwrap().projects.is_empty() {
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
    // The owner order's sweep covers the whole plan and takes the level lot.
    request_started(&mut state, id, "storage");
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
    for name in ["minecraft:water", "minecraft:lava", "minecraft:torch"] {
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
    request_started(&mut state, id, "storage");
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
    // The direct-work helper ended in an unticketed neighbour chunk, where
    // construction cannot hire the only resident: no autonomous site opens
    // until it is back on the active fixture.
    plan(&mut state, id);
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
    state.entities.set_pos(slot, [1.5, 201.0, 2.5]);
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
        request_started(&mut state, id, name);
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
    request_started(&mut state, id, "storage");
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
    request_started(&mut state, id, "storage");
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
    request_started(&mut state, id, "storage");
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
        if !state.settlements.get(id).unwrap().projects.is_empty() {
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
    // The proofs' shared budget, refilled per tick as the server does: a
    // proof whose crown takes a free lot finishes its scan over several.
    let mut budget = site_turn::SiteBudget::default();
    let mut settled =
        |state: &TickState, verdict: &dyn Fn(&TickState, &mut site_turn::SiteBudget) -> Replant| {
            (0..32)
                .map(|_| {
                    budget.refill();
                    verdict(state, &mut budget)
                })
                .find(|verdict| *verdict != Replant::NotYet)
                .expect("the proof settles")
        };
    let misses = state.settlements.forest.lock().unwrap().misses();
    assert_eq!(
        settled(&state, &|state, budget| replant_check(state, root, budget)),
        Replant::Plant,
        "far B cannot veto A's safe renewal"
    );
    assert_eq!(
        state.settlements.forest.lock().unwrap().misses(),
        misses,
        "A's admission must not request evidence for B's cold grove"
    );
    // Prove the cold foreign town would refuse this unrelated planting if
    // queried: it has no proved house lot and needs fresh forest evidence.
    assert_ne!(
        layout::planting_verdict(
            &state,
            state.settlements.get(b).unwrap(),
            crown,
            &mut site_turn::SiteBudget::default()
        ),
        Replant::Plant
    );
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
    assert_eq!(
        settled(&state, &|state, budget| layout::planting_verdict(
            state,
            state.settlements.get(a).unwrap(),
            crown,
            budget
        )),
        Replant::Plant
    );
    assert_eq!(
        layout::planting_verdict(
            &state,
            state.settlements.get(b).unwrap(),
            crown,
            &mut site_turn::SiteBudget::default()
        ),
        Replant::Refused,
        "the adjacent crown covers B's plaza and cuts its building off"
    );
    assert_eq!(
        settled(&state, &|state, budget| replant_check(state, root, budget)),
        Replant::Refused,
        "nearby B's access still vetoes A's planting"
    );
}

/// Spain seed 100 (pin 3068dfb1b857): lots sit up to SITE_BAND below the
/// bell, while the access corridor's columns are sampled around the bell. A
/// corridor tree on ground 74 above a lot floor of 63 had no footing in the
/// lot's band, so its builder got no stand and idled at the store
/// (worksite_unreached, navigation=None) for 22,000 ticks.
#[test]
fn a_corridor_tree_on_ground_above_a_low_lot_gets_a_felling_stand() {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    // The bell six below the street keeps a pit lot ten below the street and
    // the street itself inside the bell's band, as on the seed-100 hillside.
    state.settlements.get_mut(id).unwrap().anchor[1] = 194;
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let (lot, size) = {
        let record = state.settlements.get(id).unwrap();
        let mut network = layout::Network::walk(&state, record);
        network
            .frontage()
            .into_iter()
            .filter_map(|(at, normal)| {
                let lot = network.lot(blueprint, at, normal)?;
                let size = blueprint.footprint(Rotation::from_quarter_turns(lot.rotation));
                let [ox, _, oz] = lot.origin;
                (ox >= -14
                    && oz >= -14
                    && ox + i32::from(size[0]) <= 30
                    && oz + i32::from(size[2]) <= 30)
                    .then_some((lot, size))
            })
            .max_by_key(|(lot, _)| (lot.origin[0] - 8).abs() + (lot.origin[2] - 8).abs())
            .expect("a loaded storage lot on the street plan")
    };
    let floor = 190;
    let mut project = Project {
        settlement: id,
        blueprint: "storage".into(),
        level: 1,
        origin: [lot.origin[0], floor, lot.origin[2]],
        rotation: lot.rotation,
        building: None,
        structure: String::new(),
        ordered: true,
        builder: None,
        order_seq: None,
    };
    // The corridor column farthest from the lot, beyond felling reach of a
    // builder waiting beside it, as the plaza-side trees were from the far lots.
    let route = {
        let record = state.settlements.get(id).unwrap();
        layout::Network::walk(&state, record)
            .construction_access(&project)
            .expect("a walk from the plaza to the lot")
    };
    let distance = |at: [i32; 3]| {
        let gap = |axis: usize| {
            let min = lot.origin[axis];
            let max = min + i32::from(size[axis]) - 1;
            (min - at[axis]).max(at[axis] - max).max(0)
        };
        gap(0).max(gap(2))
    };
    let [x, ground, z] = route
        .iter()
        .copied()
        .filter(|&[x, _, z]| (x, z) != (1, 1))
        .max_by_key(|&at| distance(at))
        .unwrap();
    assert_eq!(ground, 200, "setup: the street stands on the flat plan");
    assert!(
        distance([x, ground, z]) >= 6,
        "setup: the tree is beyond felling reach from beside the lot"
    );
    let root = [x, ground + 1, z];
    super::super::forestry::tests::grown_world(&mut state, vec![root], "birch");
    for x in lot.origin[0]..lot.origin[0] + i32::from(size[0]) {
        for z in lot.origin[2]..lot.origin[2] + i32::from(size[2]) {
            state
                .world
                .set_block(x, floor, z, state.registry.states.stone);
            for y in floor + 1..=200 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    state.world.fold_overlay();
    assert!(
        surface_y_for_lot(
            &state,
            floor,
            x,
            z,
            None,
            &mut super::super::forestry::Clearance::prepared()
        )
        .is_none(),
        "setup: the corridor ground is beyond the lot floor's band"
    );
    assert!(begin(&mut state, project.clone(), "oak", 0));
    project = state.settlements.get(id).unwrap().projects[0].clone();
    super::tests::bind_project(&mut state, id, &project.structure, builder);
    let next = executor::next_cell(&state, id, &project.structure)
        .unwrap()
        .unwrap();
    assert_eq!(
        [next[0], next[2]],
        [x, z],
        "setup: the corridor tree is the first cell"
    );
    // The builder waits on the street plan out of placement reach of the tree,
    // as the seed-100 builder stood at the store 19 blocks from it.
    let [ox, _, oz] = lot.origin;
    let (far, gap) = (-15..31)
        .flat_map(|x| (-15..31).map(move |z| [x, z]))
        .filter(|&[x, z]| {
            !(ox - 1..=ox + i32::from(size[0])).contains(&x)
                || !(oz - 1..=oz + i32::from(size[2])).contains(&z)
        })
        .map(|at| (at, (at[0] - x).abs().max((at[1] - z).abs())))
        .max_by_key(|&(_, gap)| gap)
        .unwrap();
    assert!(
        gap > 13,
        "setup: the builder is out of placement reach: {gap}"
    );
    state.entities.set_pos(
        builder,
        [f64::from(far[0]) + 0.5, 201.0, f64::from(far[1]) + 0.5],
    );
    let mut v = state.entities.row_mut(builder).villager.take().unwrap();
    let target = crate::villager::building::work(
        &mut state,
        builder,
        &mut v,
        &mut crate::boundary::ChangeSet::default(),
    );
    state.entities.row_mut(builder).villager = Some(v);
    let target = target.expect("the corridor tree has a felling stand");
    assert!(target.exact);
    assert_eq!(
        target.pos.y,
        ground + 1,
        "the stand is on the corridor's ground"
    );
    assert!(
        (target.pos.x - x).abs() <= 3 && (target.pos.z - z).abs() <= 3,
        "the stand is beside the tree: {:?}, tree at {root:?}",
        target.pos
    );
    for _ in 0..64 {
        if state.world.block_at(root[0], root[1], root[2]) == Some(state.registry.states.air) {
            return;
        }
        super::tests::attempt(&mut state, id, builder);
    }
    panic!(
        "the corridor tree was never felled: next={:?}",
        executor::next_cell(&state, id, &project.structure)
    );
}

/// Diagnostic (not a regression): measure the retained seed-24 village's site
/// supply with the planner's own lot check, and print which reason refuses each
/// frontage. Ignored by default; run with `D123_CIV_ARTIFACT=<artifact dir>`:
///
/// ```text
/// D123_CIV_ARTIFACT=/path/to/20261125T213859-... cargo test -p mc-core --lib \
///   settlements::construction_site_tests::probe_retained_village_site_supply -- --ignored --nocapture
/// ```
#[test]
#[ignore = "diagnostic probe: needs D123_CIV_ARTIFACT pointing at a civilization-seed-qa artifact"]
fn probe_retained_village_site_supply() {
    let root = std::env::var("D123_CIV_ARTIFACT").expect("D123_CIV_ARTIFACT=<artifact dir>");
    let artifact = std::path::PathBuf::from(&root);
    let directory = tempfile::tempdir().unwrap();
    let mut config = crate::config::CoreConfig::new(directory.path().to_owned(), 24);
    config.vanilla_data_dir =
        Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    let registry = std::sync::Arc::new(
        crate::registry::CoreRegistry::load(config.vanilla_data_dir.as_deref()).unwrap(),
    );
    let store = std::sync::Arc::new(
        mc_world::native::NativeStore::open_learning_identity(
            &artifact.join("world/native/state.redb"),
            64 * 1024 * 1024,
        )
        .expect("retained store")
        .0,
    );
    // The settlements and their certificates live in the checkpoint globals, not in
    // the dimension replay: recover them exactly as `Core::open` does, otherwise the
    // state has no settlement and the probe has nothing to measure.
    let reader = store.checkpoint_reader().expect("checkpoint reader");
    let mut recovered =
        crate::native_checkpoint::recover_with_reader(&config, registry.clone(), reader)
            .expect("checkpoint recovery");
    let replay = recovered
        .dimensions
        .remove(&crate::dimensions::Dimension::Overworld)
        .expect("overworld replay");
    let mut state = crate::core::load_dimension_state(
        &config,
        registry,
        crate::dimensions::Dimension::Overworld,
        &recovered.manifest,
        replay,
        store,
        &recovered.checkpoint.globals,
        None,
    )
    .expect("retained dimension state");
    state.native_globals = recovered.checkpoint.globals;
    crate::settlements::persistence::finish_recovery(&mut state).expect("settlement recovery");
    let want: rustc_hash::FxHashMap<crate::boundary::ChunkPos, i32> = (-12..=12)
        .flat_map(|dx| {
            (-12..=12).map(move |dz| {
                (
                    crate::boundary::ChunkPos {
                        x: 8 + dx,
                        z: -14 + dz,
                    },
                    0,
                )
            })
        })
        .collect();
    state.world.ensure_blocking(&want);
    state.refresh_active_chunks(state.tick);
    let id = state
        .settlements
        .records()
        .next()
        .map(|record| record.id)
        .expect("a settlement in the retained world");
    let record = state.settlements.get(id).expect("record").clone();
    println!(
        "PROBE settlement anchor={:?} radius={} buildings={}",
        record.anchor,
        record.radius,
        record.buildings.len()
    );
    for building in record.buildings.values() {
        println!(
            "PROBE building id={} kind={:?} min={:?} max={:?} natural={}",
            building.id.0,
            building.kind,
            building.bounds.min,
            building.bounds.max,
            building.natural_zone
        );
    }
    // Direct check of one authored lot (the old run's House footprint):
    // D123_CIV_SITE="x,y,z,sx,sy,sz".
    if let Ok(spec) = std::env::var("D123_CIV_SITE") {
        let values: Vec<i32> = spec
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        if values.len() == 6 {
            let origin = [values[0], values[1], values[2]];
            let size = [values[3] as u8, values[4] as u8, values[5] as u8];
            let mut survey = site_refusals::Survey::default();
            let outcome = site_refusals::check(
                &state,
                &record,
                origin,
                size,
                record.founding.as_ref(),
                &mut survey,
                None,
            );
            println!("PROBE site origin={origin:?} size={size:?} -> {outcome:?}");
        }
    }
    // R1 diagnosis (HANDOFF-51): a lot the sweep ranks can be refused later by
    // `planned_cells`, which `check` does not model. Plan one authored lot
    // directly: D123_CIV_PLAN="x,y,z,rotation,blueprint,species".
    if let Ok(spec) = std::env::var("D123_CIV_PLAN") {
        let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
        if parts.len() == 6 {
            let origin = [
                parts[0].parse().expect("x"),
                parts[1].parse().expect("y"),
                parts[2].parse().expect("z"),
            ];
            let rotation: u8 = parts[3].parse().expect("rotation");
            let project = super::Project {
                settlement: id,
                blueprint: parts[4].to_owned(),
                level: 1,
                origin,
                rotation,
                building: None,
                structure: String::new(),
                ordered: false,
                builder: None,
                order_seq: None,
            };
            let mut survey = site_refusals::Survey::default();
            let size = blueprints::get(&state.registry, parts[4])
                .map(|blueprint| {
                    blueprint.footprint(blueprints::Rotation::from_quarter_turns(rotation))
                })
                .expect("blueprint");
            let checked = site_refusals::check(
                &state,
                &record,
                origin,
                size,
                record.founding.as_ref(),
                &mut survey,
                Some(&site_refusals::floor_columns(
                    blueprints::get(&state.registry, parts[4]).expect("blueprint"),
                    rotation,
                )),
            );
            super::preparation::take_last_stage();
            let mut clearance = crate::settlements::forestry::Clearance::prepared();
            let planned =
                super::preparation::planned_cells(&state, &project, parts[5], &mut clearance);
            println!(
                "PROBE plan origin={origin:?} rotation={rotation} blueprint={} check={:?} trees_without_route={} planned={:?}",
                parts[4],
                checked.is_ok(),
                {
                    let record = state.settlements.get(id).unwrap();
                    let network = layout::Network::for_construction(&state, record);
                    let mut clearance = crate::settlements::forestry::Clearance::prepared();
                    super::super::forestry::lot_trees_without_route(
                        &state,
                        &network,
                        &project,
                        size,
                        &mut clearance,
                    )
                },
                planned
                    .as_ref()
                    .map(|(bounds, cells)| (bounds.min, cells.len()))
                    .map_err(|_| {
                        (
                            super::preparation::take_last_stage(),
                            super::super::forestry::take_last_tree_stage(),
                        )
                    }),
            );
        }
    }
    let blueprint_name = std::env::var("D123_CIV_BLUEPRINT").unwrap_or_else(|_| "storage".into());
    let blueprint = blueprints::get(&state.registry, &blueprint_name)
        .unwrap_or_else(|| panic!("blueprint {blueprint_name:?} is not in the catalogue"));
    if std::env::var_os("D123_CIV_STEP").is_some() {
        // Review round 3 C6: one tick's site cost in release on this world.
        // An off-cadence step is the pass prologue's storage images, one
        // construction network and one 64-lot slice (`step_sites` takes one of
        // the tick's two slices and runs one scan). The admission pass is the
        // same prologue with two networks and two 64-lot slices. Eight samples;
        // the median is the number to read.
        let house = blueprints::get(&state.registry, "spanish_house")
            .expect("spanish_house is in the catalogue");
        let mut step_ms: Vec<f64> = Vec::new();
        let mut pass_ms: Vec<f64> = Vec::new();
        let mut network_ms: Vec<f64> = Vec::new();
        let mut examined_step = 0u32;
        let mut examined_pass = 0u32;
        let scan = |state: &TickState, kind: &'static blueprints::Blueprint| {
            let mut network = layout::Network::for_construction(state, &record);
            let mut sweeps = site_refusals::Sweeps::default();
            let mut survey = site_refusals::Survey::default();
            let mut report = site_refusals::Report::default();
            let mut budget = 64u32;
            let floors: [std::collections::BTreeSet<[i32; 2]>; 4] = std::array::from_fn(|turns| {
                site_refusals::floor_columns(kind, u8::try_from(turns).unwrap())
            });
            let _ = site_refusals::advance(
                state,
                &record,
                &mut network,
                kind,
                &floors,
                &mut survey,
                &mut sweeps,
                record.construction_cursor,
                &mut budget,
                &mut report,
                true,
            );
            report.examined
        };
        for _ in 0..8 {
            let started = std::time::Instant::now();
            let images =
                crate::plugin_structures::settlement_builds::source_images(&state, record.id);
            examined_step = scan(&state, house);
            step_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(images.len());
            let started = std::time::Instant::now();
            let images =
                crate::plugin_structures::settlement_builds::source_images(&state, record.id);
            examined_pass = scan(&state, house) + scan(&state, blueprint);
            pass_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(images.len());
        }
        for _ in 0..8 {
            let started = std::time::Instant::now();
            let network = layout::Network::for_construction(&state, &record);
            network_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(network.frontage().len());
        }
        let median = |samples: &[f64]| {
            let mut sorted = samples.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
            sorted[sorted.len() / 2]
        };
        let step_median = median(&step_ms);
        let pass_median = median(&pass_ms);
        let network_median = median(&network_ms);
        println!(
            "PROBE step house=spanish_house funded={blueprint_name} \
             step_examined={examined_step} pass_examined={examined_pass} \
             step_median_ms={step_median:.4} step_samples_ms={step_ms:?} \
             pass_median_ms={pass_median:.4} pass_samples_ms={pass_ms:?} \
             network_median_ms={network_median:.4}",
        );
        // The real path for comparison: arm the village's own flag, simulate
        // its chunk, and time `step_sites` itself, which also runs
        // `funded_kinds` and the funding proofs the modelled shapes omit.
        let chunk = crate::boundary::ChunkPos {
            x: record.anchor[0].div_euclid(16),
            z: record.anchor[2].div_euclid(16),
        };
        state
            .tickets
            .insert("d123-civ-step-probe".into(), (chunk, 8));
        state.refresh_active_chunks(state.tick);
        if !state.world.is_active(chunk.x, chunk.z) {
            println!("PROBE step real=unavailable:chunk-not-active chunk={chunk:?}");
            return;
        }
        let mut real_ms: Vec<f64> = Vec::new();
        let mut real_examined = 0u32;
        for _ in 0..8 {
            state
                .settlements
                .site_sweep
                .entry(record.id)
                .or_default()
                .stepping = true;
            site_refusals::EXAMINED.with(|count| count.set(0));
            let mut budget = super::site_turn::SiteBudget::default();
            let started = std::time::Instant::now();
            super::step_sites(&mut state, &mut budget);
            real_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            real_examined = site_refusals::EXAMINED.with(std::cell::Cell::get);
        }
        println!(
            "PROBE step real_examined={real_examined} real_median_ms={:.4} real_samples_ms={real_ms:?}",
            median(&real_ms),
        );
        return;
    }
    let mut network = layout::Network::for_construction(&state, &record);
    let fronts = network.frontage();
    println!("PROBE frontages={}", fronts.len());
    let mut counts: std::collections::BTreeMap<String, (u32, Option<[i32; 3]>)> =
        Default::default();
    let mut legal = Vec::new();
    let mut examined = 0u32;
    let mut survey = site_refusals::Survey::default();
    // The per-frontage cost is what sizes the planner's per-pass site budget:
    // a full sweep of a real village must finish in a handful of passes, so the
    // fix needs this number rather than a guess (HANDOFF-50, D123CivH).
    let timed = std::env::var_os("D123_CIV_TIME").is_some();
    let total_frontages = fronts.len();
    let started = std::time::Instant::now();
    for (front, (at, normal)) in fronts.into_iter().enumerate() {
        let Some(lot) = network.lot(blueprint, at, normal) else {
            continue;
        };
        examined += 1;
        let size = blueprint.footprint(blueprints::Rotation::from_quarter_turns(lot.rotation));
        match site_refusals::check(
            &state,
            &record,
            lot.origin,
            size,
            record.founding.as_ref(),
            &mut survey,
            Some(&site_refusals::floor_columns(blueprint, lot.rotation)),
        ) {
            Ok(_) => legal.push((front, lot.origin, lot.rotation, size)),
            Err(reason) => {
                let entry = counts.entry(format!("{reason:?}")).or_insert((0, None));
                entry.0 += 1;
                entry.1.get_or_insert(lot.origin);
            }
        }
    }
    if std::env::var_os("D123_CIV_HORIZON").is_some() {
        // Early-termination evidence in the units the walk counts: frontages
        // EXAMINED (every frontage, legal or not), not legal lots.
        let blueprint = blueprints::get(&state.registry, &blueprint_name).expect("blueprint");
        let mut network = layout::Network::for_construction(&state, &record);
        let fronts = network.frontage();
        type Key = ((u32, u32, u32), i64, [i32; 3], u8);
        let mut best_by_horizon: Vec<(usize, Option<Key>)> = [256usize, 512, 1024, 2048, 4096]
            .into_iter()
            .map(|h| (h, None))
            .collect();
        let mut best: Option<Key> = None;
        let mut examined = 0usize;
        let mut legal = 0usize;
        let mut zero_work = 0usize;
        let mut distances: Vec<i64> = Vec::new();
        for (at, normal) in fronts {
            examined += 1;
            let Some(lot) = network.lot(blueprint, at, normal) else {
                continue;
            };
            let size = blueprint.footprint(blueprints::Rotation::from_quarter_turns(lot.rotation));
            let mut survey = site_refusals::Survey::default();
            let Ok(cost) = site_refusals::check(
                &state,
                &record,
                lot.origin,
                size,
                record.founding.as_ref(),
                &mut survey,
                Some(&site_refusals::floor_columns(blueprint, lot.rotation)),
            ) else {
                continue;
            };
            legal += 1;
            let distance = (0..3)
                .map(|axis| (i64::from(lot.origin[axis]) - i64::from(record.anchor[axis])).pow(2))
                .sum::<i64>();
            distances.push(distance);
            let key = (cost.rank(), distance, lot.origin, lot.rotation);
            if cost.rank() == (0, 0, 0) {
                zero_work += 1;
            }
            if best.is_none_or(|known| key < known) {
                best = Some(key);
            }
            for (horizon, slot) in &mut best_by_horizon {
                if examined == *horizon {
                    *slot = best;
                }
            }
        }
        let winner = best.expect("a legal lot");
        for (horizon, snapshot) in &best_by_horizon {
            let Some(found) = snapshot else { continue };
            println!(
                "PROBE horizon blueprint={blueprint_name} frontages={examined} legal={legal} h={horizon} best_rank={:?} best_dist={} winner_rank={:?} winner_dist={} same={}",
                found.0,
                found.1,
                winner.0,
                winner.1,
                *found == winner
            );
        }
        // The exact bound: a zero-work best whose every remaining frontage is
        // farther from the bell. Suffix-min over the legal lots' distances.
        let mut suffix = vec![i64::MAX; distances.len().saturating_add(1)];
        for index in (0..distances.len()).rev() {
            suffix[index] = suffix[index + 1].min(distances[index]);
        }
        println!(
            "PROBE horizon_bound blueprint={blueprint_name} zero_work_lots={zero_work} winner_rank={:?} suffix_min_first={}",
            winner.0, suffix[0]
        );
    }
    if std::env::var_os("D123_CIV_PLAN_COST").is_some() {
        // The structural option: a candidate becomes `best` only if the real
        // plan accepts it, run only on best improvements. Measure how many
        // improvements a real sweep has and what each plan costs.
        let blueprint = blueprints::get(&state.registry, &blueprint_name).expect("blueprint");
        let fronts = layout::Network::for_construction(&state, &record).frontage();
        let mut network = layout::Network::for_construction(&state, &record);
        let mut best: Option<(u32, i64, [i32; 3], u8)> = None;
        let mut improvements = 0u32;
        let mut plan_ms = 0u128;
        let mut plans_ok = 0u32;
        for (at, normal) in fronts {
            let Some(lot) = network.lot(blueprint, at, normal) else {
                continue;
            };
            let size = blueprint.footprint(blueprints::Rotation::from_quarter_turns(lot.rotation));
            let mut survey = site_refusals::Survey::default();
            let Ok(cost) = site_refusals::check(
                &state,
                &record,
                lot.origin,
                size,
                record.founding.as_ref(),
                &mut survey,
                Some(&site_refusals::floor_columns(blueprint, lot.rotation)),
            ) else {
                continue;
            };
            let key = (
                cost.total(),
                (0..3)
                    .map(|axis| {
                        (i64::from(lot.origin[axis]) - i64::from(record.anchor[axis])).pow(2)
                    })
                    .sum::<i64>(),
                lot.origin,
                lot.rotation,
            );
            if best.is_none_or(|known| key < known) {
                best = Some(key);
                improvements += 1;
                let project = super::Project {
                    settlement: id,
                    blueprint: blueprint.id.clone(),
                    level: 1,
                    origin: lot.origin,
                    rotation: lot.rotation,
                    building: None,
                    structure: String::new(),
                    ordered: false,
                    builder: None,
                    order_seq: None,
                };
                let before = std::time::Instant::now();
                let mut clearance = crate::settlements::forestry::Clearance::prepared();
                let planned =
                    super::preparation::planned_cells(&state, &project, "oak", &mut clearance);
                plan_ms += before.elapsed().as_millis();
                plans_ok += u32::from(planned.is_ok());
            }
        }
        println!(
            "PROBE plan_cost blueprint={blueprint_name} frontages={} improvements={improvements} plans_ok={plans_ok} plan_ms_total={plan_ms}",
            total_frontages
        );
    }
    if timed {
        let elapsed = started.elapsed();
        let per_frontage =
            elapsed.as_nanos() / u128::from(u32::try_from(total_frontages).unwrap_or(1));
        let per_examined = elapsed.as_nanos() / u128::from(examined.max(1));
        println!(
            "PROBE time frontages={total_frontages} examined={examined} total_ms={} ns_per_frontage={per_frontage} ns_per_lot={per_examined}",
            elapsed.as_millis()
        );
    }
    println!("PROBE examined={examined} legal={}", legal.len());
    for (reason, (count, example)) in &counts {
        println!("PROBE refused {reason} x{count} first={example:?}");
    }
    for (front, origin, rotation, size) in legal.iter().take(5) {
        println!("PROBE legal front={front} origin={origin:?} rotation={rotation} size={size:?}");
    }
    // The planner's own pass loop on a static world: if `site_project` admits here
    // within frontages/64 passes, the live gate's failure is churn (topology
    // restarts or kind interleaving); if it never admits although the sweep
    // finishes, the admission path itself is the defect (HANDOFF-49 section 13).
    if std::env::var("D123_CIV_SIMULATE").is_ok() {
        let house = blueprints::get(&state.registry, "house").expect("house blueprint");
        let images = crate::plugin_structures::settlement_builds::source_images(&state, id);
        let mut sweeps = site_refusals::Sweeps::default();
        let mut cursor = 0u32;
        let mut admitted = None;
        for pass in 0..200u32 {
            let outcome = super::site_project(
                &state,
                &record,
                std::iter::once(house),
                &mut sweeps,
                cursor,
                64,
                &images,
            );
            println!(
                "PROBE pass={pass} site={} cursor={} examined={} running={} waiting={} sweeps={:?}",
                outcome.site.is_some(),
                outcome.cursor,
                outcome.report.examined,
                outcome.running,
                outcome.waiting,
                sweeps.summary()
            );
            if outcome.site.is_some() {
                admitted = Some(pass);
                break;
            }
            cursor = outcome.cursor;
        }
        println!("PROBE simulated_admitted_at_pass={admitted:?}");
    }
}
