//! D8 pre-freeze cross-project safety: a parallel site may not place a block
//! through another project's builder, and no builder stands inside another
//! active project's future body (review SOL-4, R3-3/R3-4/R3-5).
//!
//! Every executor case goes through the real `advance` -> `debit` path with two
//! prepared, adjacent sites of two projects of one settlement; only the world
//! fixture is shared, no terrain is generated for the lots.
use super::*;
use crate::settlements::allocator::tests::villager;
use crate::settlements::construction::tests::{bind_project, fixture};
use crate::settlements::{SettlementId, construction};
use mc_data::ItemStack;
use mc_script::ScriptWorldBounds;

/// The persisted structure-ledger domain (`plugin_structures::OWNER`), which is
/// part of the save image; a refusal must leave it byte-identical.
const LEDGER_OWNER: &str = "@native-structures";

fn item(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
        .unwrap()
}

/// The fixture's second Builder, bound to the settlement like its own.
fn second_builder(state: &mut TickState, id: SettlementId, feet: [f64; 3]) -> EntitySlot {
    let slot = villager(state, feet);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(state, id, entity).unwrap();
    let v = state.entities.row_mut(slot).villager.as_mut().unwrap();
    v.data.profession = VillagerProfession::Builder;
    v.activity = Activity::Work;
    slot
}

/// Admit `project` into the settlement's active set. The record is the only
/// authority the safety predicates read; a ledger structure is prepared
/// separately where the path under test resolves one.
fn push_project(
    state: &mut TickState,
    id: SettlementId,
    handle: &str,
    blueprint: &str,
    origin: [i32; 3],
) {
    let record = state.settlements.get_mut(id).unwrap();
    record.projects.push(construction::Project {
        settlement: id,
        blueprint: blueprint.into(),
        level: 1,
        origin,
        rotation: 0,
        building: None,
        structure: handle.into(),
        ordered: false,
        builder: None,
        order_seq: None,
    });
    record.revision = record.revision.saturating_add(1);
}

fn site_project(state: &TickState, id: SettlementId, handle: &str) -> construction::Project {
    construction::project_for_structure(state.settlements.get(id).unwrap(), handle)
        .unwrap_or_else(|| panic!("project {handle}"))
        .clone()
}

/// A prepared project whose canonical site reservation is exactly `site`. The
/// safety predicates read that reservation, so a fixture that only pushes the
/// project into the record leaves no box to protect.
fn prepared_site(
    state: &mut TickState,
    id: SettlementId,
    blueprint: &str,
    site: ScriptWorldBounds,
) -> String {
    let air = state.registry.states.air;
    executor::prepare(
        state,
        id,
        blueprint.into(),
        site,
        site,
        vec![executor::PlannedCell {
            portion: executor::PortionKind::Cut,
            at: site.min,
            state: air,
            resource: None,
            quantity: 0,
        }],
    )
    .unwrap()
}

/// Whether cell `at` falls inside the project's canonical site reservation.
fn inside_site(state: &TickState, id: SettlementId, handle: &str, at: [i32; 3]) -> bool {
    executor::site_bounds(state, id, handle).is_some_and(|site| {
        (0..3).all(|axis| site.min[axis] <= at[axis] && at[axis] <= site.max[axis])
    })
}

/// The state a refusal must leave untouched: the persisted structure ledger, the
/// settlement's storage, the acting builder's carry and the cell the step would
/// have written.
struct Untouched {
    ledger: Vec<(String, serde_json::Value)>,
    storage: Vec<ItemStack>,
    carry: [Option<ItemStack>; 8],
    block: u32,
    at: [i32; 3],
}

fn ledger(state: &TickState) -> Vec<(String, serde_json::Value)> {
    state
        .plugins
        .domain_entries(LEDGER_OWNER)
        .map(|(key, value)| (key.to_owned(), value.clone()))
        .collect()
}

fn snapshot(state: &TickState, slot: EntitySlot, at: [i32; 3]) -> Untouched {
    Untouched {
        ledger: ledger(state),
        storage: state.block_entities[&(1, 201, 1)].slots.clone(),
        carry: state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .expect("builder")
            .inventory
            .clone(),
        block: state
            .world
            .block_at(at[0], at[1], at[2])
            .expect("fixture cell"),
        at,
    }
}

fn assert_untouched(state: &TickState, slot: EntitySlot, before: &Untouched) {
    let after = snapshot(state, slot, before.at);
    assert!(!after.ledger.is_empty(), "the ledger exists to stay put");
    assert_eq!(after.ledger, before.ledger, "the ledger moved");
    assert_eq!(after.storage, before.storage, "storage moved");
    assert_eq!(after.carry, before.carry, "the builder's carry moved");
    assert_eq!(after.block, before.block, "the cell was written");
}

struct TwoSites {
    state: TickState,
    id: SettlementId,
    builder: EntitySlot,
    handle: String,
}

/// Two adjacent prepared sites of one settlement, projected at
/// `[6,201,6]..[10,204,10]` and `[11,201,6]..[15,204,10]`. The first site's
/// next build cell is `cell`; the neighbour's Builder stands wherever `feet`
/// puts it, so a builder that wandered into our lot is the case under test.
fn two_sites(feet: [f64; 3], cell: [i32; 3]) -> TwoSites {
    let (mut state, id, builder) = fixture();
    let stone = state.registry.states.stone;
    let air = state.registry.states.air;
    let mine = ScriptWorldBounds::new([6, 201, 6], [10, 204, 10]).unwrap();
    let handle = executor::prepare(
        &mut state,
        id,
        "house".into(),
        mine,
        mine,
        vec![executor::PlannedCell {
            portion: executor::PortionKind::Build,
            at: cell,
            state: stone,
            resource: Some("minecraft:stone".into()),
            quantity: 1,
        }],
    )
    .unwrap();
    let theirs = ScriptWorldBounds::new([11, 201, 6], [15, 204, 10]).unwrap();
    let neighbour = executor::prepare(
        &mut state,
        id,
        "house".into(),
        theirs,
        theirs,
        vec![executor::PlannedCell {
            portion: executor::PortionKind::Cut,
            at: [12, 201, 7],
            state: air,
            resource: None,
            quantity: 0,
        }],
    )
    .unwrap();
    push_project(&mut state, id, &handle, "house", [6, 201, 6]);
    push_project(&mut state, id, &neighbour, "house", [11, 201, 6]);
    bind_project(&mut state, id, &handle, builder);
    let second = second_builder(&mut state, id, feet);
    bind_project(&mut state, id, &neighbour, second);
    // The acting builder carries the block the refused step would place.
    let stone_item = item(&state, "stone");
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(ItemStack::new(stone_item, 1));
    TwoSites {
        state,
        id,
        builder,
        handle,
    }
}

fn advance(sites: &mut TwoSites) -> Result<bool, mc_script::ScriptOperationFailure> {
    let uuid = sites.state.entities.row(sites.builder).uuid;
    executor::advance(&mut sites.state, sites.id, &sites.handle, uuid)
}

/// R3-3: the body owns the cell above its feet. A solid block there is placed
/// through the neighbour builder's head.
#[test]
fn a_head_height_block_over_a_neighbour_builder_is_refused() {
    // Feet at [8,201,8]: the placement cell is the head cell of that column.
    let mut sites = two_sites([8.5, 201.0, 8.5], [8, 202, 8]);
    let before = snapshot(&sites.state, sites.builder, [8, 202, 8]);
    let refused = advance(&mut sites);
    assert_eq!(
        refused.map_err(|failure| failure.as_str()),
        Err("blocked"),
        "a head-height cell over another project's builder must be refused"
    );
    assert_untouched(&sites.state, sites.builder, &before);
}

/// R3-3: the body is 0.6 wide, so it straddles the cell edge; a block in the
/// next column is still placed through its chest.
#[test]
fn a_side_block_reaching_a_neighbour_builder_is_refused() {
    // Feet at x = 8.95: the body spans 8.65..9.25, into the cell at x = 9.
    let mut sites = two_sites([8.95, 201.0, 8.5], [9, 201, 8]);
    let before = snapshot(&sites.state, sites.builder, [9, 201, 8]);
    let refused = advance(&mut sites);
    assert_eq!(
        refused.map_err(|failure| failure.as_str()),
        Err("blocked"),
        "a cell the body straddles must be refused"
    );
    assert_untouched(&sites.state, sites.builder, &before);
}

/// The guard is not a blanket refusal: the same neighbour builder, one cell
/// away, leaves a free cell buildable and pays for it from the carry.
#[test]
fn a_cell_clear_of_the_neighbour_body_still_places() {
    let mut sites = two_sites([8.95, 201.0, 8.5], [9, 201, 7]);
    let placed = advance(&mut sites).expect("a cell outside the neighbour body is buildable");
    assert!(placed, "the single-stage site completes on its one cell");
    let state = &sites.state;
    assert_eq!(
        state.world.block_at(9, 201, 7).expect("fixture cell"),
        state.registry.states.stone,
        "the free cell was placed"
    );
    let carried = state
        .entities
        .row(sites.builder)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .iter()
        .flatten()
        .filter(|stack| stack.item_id == item(state, "stone"))
        .map(|stack| stack.count)
        .sum::<i32>();
    assert_eq!(carried, 0, "the placement paid from the carry");
}

/// A clearing site whose own stand is available: the neighbour's prepared lot
/// covers the nearest stand cell, so the chosen stand must be the next one
/// outside it.
fn clearing_site() -> (TickState, SettlementId, EntitySlot, String, String) {
    let (mut state, id, slot) = fixture();
    let air = state.registry.states.air;
    let site = ScriptWorldBounds::new([5, 201, 5], [9, 204, 9]).unwrap();
    let handle = executor::prepare(
        &mut state,
        id,
        "house".into(),
        site,
        site,
        vec![executor::PlannedCell {
            portion: executor::PortionKind::Cut,
            at: [5, 201, 5],
            state: air,
            resource: None,
            quantity: 0,
        }],
    )
    .unwrap();
    push_project(&mut state, id, &handle, "house", [5, 201, 5]);
    // The neighbour's authored site x 0..4, z 4..7 is adjacent to this lot and
    // owns the stand cell at [4,201,5] nearest to the builder.
    let neighbour = ScriptWorldBounds::new([0, 201, 4], [4, 203, 7]).unwrap();
    let origin = neighbour.min;
    let neighbour = prepared_site(&mut state, id, "house", neighbour);
    push_project(&mut state, id, &neighbour, "house", origin);
    state.entities.set_pos(slot, [4.5, 201.0, 5.5]);
    (state, id, slot, handle, neighbour)
}

/// R3-4: `preparation_stand` never returns a stand inside another active
/// project's future body, while a stand outside it stays available.
#[test]
fn a_stand_is_never_chosen_inside_a_neighbour_lot_while_clearing() {
    let (state, id, slot, handle, neighbour) = clearing_site();
    let project = site_project(&state, id, &handle);
    assert!(
        inside_site(&state, id, &neighbour, [4, 201, 5]),
        "fixture: the neighbour lot owns the nearest stand cell"
    );
    let base = BlockPos { x: 5, y: 201, z: 5 };
    let stand = preparation_stand(&state, slot, &project, [5, 201, 5], base)
        .expect("a stand outside the neighbour lot is still available");
    assert!(
        !standing_in_other_body(&state, &project, slot, stand),
        "stand {stand:?} is inside the neighbour's future body"
    );
    assert_ne!(
        stand,
        BlockPos { x: 4, y: 201, z: 5 },
        "the stand inside the neighbour lot is not chosen"
    );
}

/// Spain seed 100 (pin 3068dfb1b857): a builder stood on its felling stand at
/// [99.5, 68, 115.25] for a whole stall window. The brain ends a route anywhere
/// within WAYPOINT_REACH of the stand, which left the body inside the
/// neighbour's site (worksite_unreached on its own navigation target). Wherever
/// on the stand it arrives, the builder's next work step leaves it in reach.
#[test]
fn a_builder_arriving_anywhere_on_its_stand_settles_in_reach() {
    let (mut state, id, slot, handle, _neighbour) = clearing_site();
    bind_project(&mut state, id, &handle, slot);
    let project = site_project(&state, id, &handle);
    let base = BlockPos { x: 5, y: 201, z: 5 };
    let stand = preparation_stand(&state, slot, &project, [5, 201, 5], base)
        .expect("a stand outside the neighbour lot is still available");
    let reach = crate::villager::brain::WAYPOINT_REACH - 0.01;
    let mut overlapping = 0;
    for [dx, dz] in [[-reach, 0.0], [reach, 0.0], [0.0, -reach], [0.0, reach]] {
        let arrived = [
            f64::from(stand.x) + 0.5 + dx,
            f64::from(stand.y),
            f64::from(stand.z) + 0.5 + dz,
        ];
        state.entities.set_pos(slot, arrived);
        overlapping += usize::from(!in_reach(&state, slot, &project, [5, 201, 5]));
        let mut v = state.entities.row_mut(slot).villager.take().unwrap();
        let target = work(&mut state, slot, &mut v, &mut ChangeSet::default());
        state.entities.row_mut(slot).villager = Some(v);
        assert!(
            target.is_some(),
            "arrived at {arrived:?}: the builder has work"
        );
        assert!(
            in_reach(&state, slot, &project, [5, 201, 5]),
            "arrived at {arrived:?} on stand {stand:?}, the builder is not in reach"
        );
    }
    assert!(
        overlapping > 0,
        "fixture: an arrival on stand {stand:?} reaches into the neighbour's site"
    );
}

/// Spain seed 100 (pin ae9f992e517b): road segments abut, so a road cell's one
/// stand touches the next segment's site. Stands tested for the whole arrival
/// tolerance left such a cell with no stand: builders bound to road, wall and
/// well projects stood idle, navigation None, for whole stall windows.
#[test]
fn a_road_stand_beside_the_next_segment_stays_a_target() {
    let (state, id, slot) = road_site(ScriptWorldBounds::new([8, 201, 4], [12, 203, 10]).unwrap());
    let project = site_project(&state, id, "road-site");
    assert_eq!(
        site_target(&state, &project, [7, 201, 7], slot, None),
        Some(BlockPos { x: 7, y: 201, z: 7 }),
        "the road cell's own column is its stand beside the abutting site"
    );
}

/// R3-4: the clear branch of `in_reach` refuses a builder whose body stands in
/// a neighbour's future body, however close the trunk it would fell.
#[test]
fn in_reach_refuses_a_builder_inside_a_neighbour_body_while_clearing() {
    let (state, id, slot, handle, _neighbour) = clearing_site();
    let project = site_project(&state, id, &handle);
    let base = BlockPos { x: 5, y: 201, z: 5 };
    assert!(
        crate::villager::mining::near(&state, slot, base, PREPARATION_REACH),
        "fixture: the builder is within felling reach"
    );
    let body = state.entities.body(slot);
    assert!(
        in_other_body(
            &state,
            &project,
            state.entities.pos(slot),
            body.half_width,
            body.height
        ),
        "fixture: the builder's body stands in the neighbour's lot"
    );
    assert!(
        !in_reach(&state, slot, &project, [5, 201, 5]),
        "a body inside the neighbour's lot is not in reach"
    );
}

/// A road project beside a prepared house site, with the builder on the
/// boundary between them.
fn road_site(neighbour: ScriptWorldBounds) -> (TickState, SettlementId, EntitySlot) {
    let (mut state, id, slot) = fixture();
    push_project(&mut state, id, "road-site", "road", [7, 201, 7]);
    let origin = neighbour.min;
    let handle = prepared_site(&mut state, id, "house", neighbour);
    push_project(&mut state, id, &handle, "house", origin);
    state.entities.set_pos(slot, [4.5, 201.0, 5.5]);
    (state, id, slot)
}

/// R3-4: the infrastructure branch refuses a road stand inside a neighbour's lot.
#[test]
fn in_reach_refuses_a_builder_inside_a_neighbour_body_on_infrastructure() {
    let next = BlockPos { x: 7, y: 201, z: 7 };
    let (state, id, slot) = road_site(ScriptWorldBounds::new([0, 201, 4], [4, 203, 7]).unwrap());
    let project = site_project(&state, id, "road-site");
    assert!(layout::infra(&project.blueprint), "fixture: a road project");
    assert!(
        crate::villager::mining::near(&state, slot, next, REACH),
        "fixture: the road cell is within placement reach"
    );
    assert!(
        !in_reach(&state, slot, &project, [7, 201, 7]),
        "a body inside the neighbour's lot is not in reach of a road cell"
    );
    // Control: the same builder and cell with the neighbour lot moved away.
    let (state, id, slot) = road_site(ScriptWorldBounds::new([0, 201, 12], [4, 203, 15]).unwrap());
    let project = site_project(&state, id, "road-site");
    assert!(
        in_reach(&state, slot, &project, [7, 201, 7]),
        "without a neighbour body the road cell is in reach"
    );
}

/// R3-5: `site_target` tests the stand's body, not its feet column, so a
/// neighbour lot one level up still owns the stand the builder's head reaches.
#[test]
fn site_target_never_picks_a_stand_whose_body_reaches_a_neighbour_lot() {
    let (state, id, slot) = road_site(ScriptWorldBounds::new([4, 202, 4], [10, 204, 10]).unwrap());
    let project = site_project(&state, id, "road-site");
    let body = state.entities.body(slot);
    assert_eq!(
        layout::stand(&state, &project, [7, 201, 7]),
        Some([7, 201, 7]),
        "fixture: the road stand is the cell's own column"
    );
    assert!(
        !construction::in_other_footprint(&state, id, "road-site", [7, 201, 7]),
        "fixture: the feet column alone is outside the neighbour's box"
    );
    assert!(
        in_other_body(
            &state,
            &project,
            [7.5, 201.0, 7.5],
            body.half_width,
            body.height
        ),
        "fixture: the 1.95-high body reaches the box one level up"
    );
    assert_eq!(
        site_target(&state, &project, [7, 201, 7], slot, None),
        None,
        "a stand inside the neighbour's body is not a target"
    );
    // Control: the same stand with the neighbour lot moved away.
    let (state, id, slot) = road_site(ScriptWorldBounds::new([9, 202, 4], [15, 204, 10]).unwrap());
    let project = site_project(&state, id, "road-site");
    assert_eq!(
        site_target(&state, &project, [7, 201, 7], slot, None),
        Some(BlockPos { x: 7, y: 201, z: 7 }),
        "without a neighbour body the road stand stays usable"
    );
}

/// R3-5: the blueprint branch of `in_reach` keeps the whole body out of a
/// neighbour's box, including the head above a feet column that is outside it.
#[test]
fn in_reach_uses_the_body_not_the_feet_column_at_a_blueprint_site() {
    let site = |neighbour: ScriptWorldBounds| {
        let (mut state, id, slot) = fixture();
        push_project(&mut state, id, "house-site", "house", [5, 201, 5]);
        let origin = neighbour.min;
        let handle = prepared_site(&mut state, id, "house", neighbour);
        push_project(&mut state, id, &handle, "house", origin);
        state.entities.set_pos(slot, [4.5, 201.0, 5.5]);
        (state, id, slot)
    };
    let (state, id, slot) = site(ScriptWorldBounds::new([0, 202, 0], [6, 204, 6]).unwrap());
    let project = site_project(&state, id, "house-site");
    let body = state.entities.body(slot);
    assert!(
        !construction::in_other_footprint(&state, id, "house-site", [4, 201, 5]),
        "the feet column is outside the neighbour's box"
    );
    assert!(
        in_other_body(
            &state,
            &project,
            state.entities.pos(slot),
            body.half_width,
            body.height
        ),
        "the body reaches the neighbour's box one level up"
    );
    assert!(
        !in_reach(&state, slot, &project, [8, 201, 8]),
        "a body reaching a neighbour's box is not in reach"
    );
    // Control: the same builder and cell with the neighbour lot moved away.
    let (state, id, slot) = site(ScriptWorldBounds::new([0, 201, 10], [4, 203, 12]).unwrap());
    let project = site_project(&state, id, "house-site");
    assert!(
        in_reach(&state, slot, &project, [8, 201, 8]),
        "outside every neighbour body the site is in reach"
    );
}

/// S2-2: the neighbour box comes from the canonical site reservation, so a
/// project whose blueprint is not in the catalog still protects its site. A
/// lamp column on the cell a house builder would stand on is such a case.
#[test]
fn a_neighbour_furnace_or_lamp_site_protects_itself() {
    let site = |lamp: ScriptWorldBounds| {
        let (mut state, id, slot) = fixture();
        // This lot is past its clear stage, so `in_reach` decides on the
        // blueprint perimeter rule, not on felling reach.
        let stone = state.registry.states.stone;
        let mine = ScriptWorldBounds::new([2, 201, 8], [6, 204, 12]).unwrap();
        let handle = executor::prepare(
            &mut state,
            id,
            "house".into(),
            mine,
            mine,
            vec![executor::PlannedCell {
                portion: executor::PortionKind::Build,
                at: [2, 201, 8],
                state: stone,
                resource: None,
                quantity: 0,
            }],
        )
        .unwrap();
        push_project(&mut state, id, &handle, "house", [2, 201, 8]);
        let origin = lamp.min;
        let lamp = prepared_site(&mut state, id, layout::LAMP, lamp);
        push_project(&mut state, id, &lamp, layout::LAMP, origin);
        state.entities.set_pos(slot, [2.5, 201.0, 7.5]);
        (state, id, slot, handle, lamp)
    };
    let (state, id, slot, mine, lamp) =
        site(ScriptWorldBounds::new([2, 201, 7], [2, 203, 7]).unwrap());
    let project = site_project(&state, id, &mine);
    let body = state.entities.body(slot);
    let neighbour = site_project(&state, id, &lamp);
    assert!(
        construction::footprint(&state, &neighbour).is_none(),
        "fixture: a lamp has no catalog footprint"
    );
    assert!(
        executor::site_bounds(&state, id, &lamp).is_some(),
        "fixture: its canonical site exists"
    );
    assert!(
        in_other_body(
            &state,
            &project,
            state.entities.pos(slot),
            body.half_width,
            body.height
        ),
        "fixture: the body stands in the lamp's site"
    );
    assert!(
        !in_reach(&state, slot, &project, [2, 201, 10]),
        "a body inside a lamp site is not in reach"
    );
    // Control: the same builder and cell with the lamp moved away.
    let (state, id, slot, mine, _lamp) =
        site(ScriptWorldBounds::new([8, 201, 7], [8, 203, 7]).unwrap());
    let project = site_project(&state, id, &mine);
    assert!(
        in_reach(&state, slot, &project, [2, 201, 10]),
        "outside every neighbour site the lot is in reach"
    );
}

/// S2-1: the furnace side search applies the veto per side and keeps going, so
/// a neighbour site on the first free side does not cost the furnace its target.
#[test]
fn a_furnace_stand_skips_a_neighbour_side_and_keeps_searching() {
    let (mut state, id, slot) = fixture();
    let next = [2, 201, 8];
    push_project(
        &mut state,
        id,
        "furnace-site",
        construction::furnace::NAME,
        next,
    );
    // The first side `furnace::stand` tries is (+1, 0): this lot owns it.
    let origin = [3, 201, 8];
    let handle = prepared_site(
        &mut state,
        id,
        "house",
        ScriptWorldBounds::new(origin, [9, 204, 14]).unwrap(),
    );
    push_project(&mut state, id, &handle, "house", origin);
    let project = site_project(&state, id, "furnace-site");
    let body = state.entities.body(slot);
    assert!(
        in_other_body(
            &state,
            &project,
            [3.5, 201.0, 8.5],
            body.half_width,
            body.height
        ),
        "fixture: the first free side is inside the neighbour lot"
    );
    assert!(
        !in_other_body(
            &state,
            &project,
            [1.5, 201.0, 8.5],
            body.half_width,
            body.height
        ),
        "fixture: the opposite side is free"
    );
    assert_eq!(
        site_target(&state, &project, next, slot, None),
        Some(BlockPos { x: 1, y: 201, z: 8 }),
        "the vetoed side is skipped and the opposite side is chosen"
    );
}

/// The geometry these regressions assume: a 0.6 x 1.95 body straddles a cell
/// edge and owns the cell above, so a changed villager size cannot silently
/// weaken them.
#[test]
fn the_measured_body_is_a_straddling_box() {
    let (state, _, slot) = fixture();
    let body = state.entities.body(slot);
    // Vanilla villager AABB 0.6 x 1.95, stored from f32 dimensions.
    assert!(
        (body.half_width - 0.3).abs() < 1e-6,
        "villager half width: {}",
        body.half_width
    );
    assert!(
        (body.height - 1.95).abs() < 1e-6,
        "villager height: {}",
        body.height
    );
    assert!(
        body.half_width > 0.0 && body.height > 1.0 + body.half_width,
        "the body spans the cell above its feet"
    );
}

/// Seed 24 (2026-11-23): the street plan follows the terrain, and a road cell
/// more than `SITE_BAND` below the bell's level had no stand at all, so its
/// builder never navigated (`worksite_unreached` with `navigation=None`) and the
/// road stayed at 0/22 for the whole 96 000-tick observation window. A street
/// stand is searched around the cell's own level; the perimeter, a gatehouse and
/// the well keep the anchor's band.
#[test]
fn a_street_cell_below_the_anchor_band_still_has_a_stand() {
    let (mut state, id, _slot) = fixture();
    let stone = state.registry.states.stone;
    let air = state.registry.states.air;
    for x in 5..=9 {
        for z in 5..=9 {
            state.world.set_block(x, 191, z, stone);
            for y in 192..=200 {
                state.world.set_block(x, y, z, air);
            }
        }
    }
    state.world.fold_overlay();
    push_project(&mut state, id, "road-low", "road", [7, 192, 7]);
    let project = site_project(&state, id, "road-low");
    // The bell at [8, 201, 8] and its band cover 193..=209: this column's ground
    // at 191 is invisible to an anchor-anchored search.
    assert!(
        layout::stand(&state, &project, [7, 203, 7]).is_none(),
        "fixture: the anchor's band cannot see this column's ground"
    );
    assert_eq!(
        layout::stand(&state, &project, [7, 192, 7]),
        Some([7, 192, 7]),
        "a street cell below the anchor band must keep its own column as its stand"
    );
}
