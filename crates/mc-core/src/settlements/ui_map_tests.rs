use super::*;
use crate::boundary::{PlayerChange, PlayerInput};
use crate::settlements::allocator::tests::test_state;
use crate::settlements::buildings::Building;
use crate::settlements::ui_text::Lang;
use crate::tick::run_tick;
use mc_script::ScriptWorldBounds;

const CONN: ConnectionId = ConnectionId(7);
const ANCHOR: [i32; 3] = [8, 201, 8];
const HOUSE: u32 = 0xE3C08D;

/// One joined player who owns `oak` (radius 48) but stands far outside it.
fn town() -> (TickState, SettlementId) {
    let mut state = test_state();
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: CONN,
            uuid: uuid::Uuid::nil(),
            name: "Mayor".into(),
            permission_level: 0,
        }],
    );
    let id = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "oak".into(),
            uuid::Uuid::nil().to_string(),
            ANCHOR,
            48,
        )
        .unwrap();
    stand(&mut state, [1000, 201, 1000]);
    (state, id)
}

fn stand(state: &mut TickState, at: [i32; 3]) {
    let entity = state.connections.entity_of(CONN).unwrap();
    let slot = state.entities.slot_of(entity).unwrap();
    state
        .entities
        .set_pos(slot, at.map(|value| f64::from(value) + 0.5));
}

fn ctx(id: SettlementId) -> Ctx {
    Ctx {
        id,
        manage: true,
        lang: Lang::En,
    }
}

fn render(state: &TickState, id: SettlementId) -> Grid {
    let viewer = player_pos(state, CONN);
    grid(&ctx(id), state, state.settlements.get(id).unwrap(), viewer)
}

fn add(
    state: &mut TickState,
    id: SettlementId,
    kind: BuildingKind,
    min: [i32; 3],
    max: [i32; 3],
) -> BuildingId {
    let record = state.settlements.get_mut(id).unwrap();
    record.building_sequence += 1;
    let building = BuildingId((id.0 << 32) | record.building_sequence);
    record.buildings.insert(
        building,
        Building {
            id: building,
            settlement: id,
            kind,
            level: 1,
            bounds: ScriptWorldBounds::new(min, max).unwrap(),
            work_points: Vec::new(),
            revision: 1,
            natural_zone: false,
        },
    );
    building
}

/// The cell holding block column (x, z), per the documented origin.
fn at(grid: &Grid, x: i32, z: i32) -> &Cell {
    let half = (SIZE / 2) as i32;
    let origin = [ANCHOR[0], ANCHOR[2]].map(|centre| centre - half * grid.scale - grid.scale / 2);
    let column = usize::try_from((x - origin[0]).div_euclid(grid.scale)).unwrap();
    let row = usize::try_from((z - origin[1]).div_euclid(grid.scale)).unwrap();
    assert!(column < SIZE && row < SIZE, "({x}, {z}) is off the map");
    &grid.cells[row * SIZE + column]
}

fn field<'a>(tag: &'a Tag, key: &str) -> Option<&'a Tag> {
    match tag {
        Tag::Compound(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, v)| v),
        _ => None,
    }
}

fn elements(tag: Option<&Tag>) -> &[Tag] {
    match tag {
        Some(Tag::List(list)) => &list.elements,
        _ => &[],
    }
}

fn string(tag: Option<&Tag>) -> &str {
    match tag {
        Some(Tag::String(value)) => value,
        _ => "",
    }
}

/// Plain text of a component: its `text` then every `extra` child, in order.
fn plain(tag: &Tag) -> String {
    let mut out = string(field(tag, "text")).to_owned();
    for child in elements(field(tag, "extra")) {
        out.push_str(&plain(child));
    }
    out
}

fn last_dialog(changes: &[PlayerChange]) -> Tag {
    changes
        .iter()
        .rev()
        .find_map(|change| match change {
            PlayerChange::ShowDialog { dialog } => Some(dialog.clone()),
            _ => None,
        })
        .expect("a dialog was shown")
}

fn run(state: &mut TickState, input: PlayerInput) -> Tag {
    let (_, outbox, _) = run_tick(state, &[input]);
    let changes: Vec<PlayerChange> = outbox
        .into_iter()
        .filter(|(connection, _)| *connection == CONN)
        .flat_map(|(_, changes)| changes)
        .collect();
    last_dialog(&changes)
}

fn command(state: &mut TickState, line: &str) -> Tag {
    run(
        state,
        PlayerInput::Command {
            connection: CONN,
            command: line.into(),
            permission_level: 0,
        },
    )
}

fn click(state: &mut TickState, id: SettlementId, action: &str) -> Tag {
    run(
        state,
        PlayerInput::DialogClick {
            connection: CONN,
            id: format!("solaris:settlement/{action}"),
            payload: Some(Tag::Compound(vec![(
                "s".to_owned(),
                Tag::String(id.0.to_string()),
            )])),
        },
    )
}

fn actions(dialog: &Tag) -> Vec<String> {
    elements(field(dialog, "actions"))
        .iter()
        .map(|button| string(field(button, "action").and_then(|a| field(a, "id"))).to_owned())
        .collect()
}

/// The map paragraph's runs (one compound per stretch of equal cells).
fn runs(dialog: &Tag) -> &[Tag] {
    let body = elements(field(dialog, "body"));
    elements(field(field(&body[0], "contents").unwrap(), "extra"))
}

#[test]
fn empty_village_shows_the_territory_disk_on_square_cells() {
    let (mut state, id) = town();
    let grid = render(&state, id);
    // 97 blocks across into 13 cells.
    assert_eq!(grid.scale, 8);
    assert_eq!(grid.cells.len(), SIZE * SIZE);
    assert_eq!(at(&grid, 8, 8).color, TERRITORY);
    assert_eq!(at(&grid, 8 + 45, 8).color, TERRITORY);
    for (x, z) in [(-43, -43), (-43, 58), (58, -43), (58, 58)] {
        assert_eq!(at(&grid, x, z).color, OUTSIDE, "corner ({x}, {z})");
    }
    assert!(grid.cells.iter().all(|cell| cell.click.is_none()));
    assert_eq!(
        grid.legend,
        vec![(TERRITORY, "Territory, radius 48".to_owned())]
    );

    let dialog = command(&mut state, "settlement menu map");
    assert_eq!(string(field(&dialog, "title")), "Map - oak");
    let body = elements(field(&dialog, "body"));
    let map = plain(field(&body[0], "contents").unwrap());
    let rows: Vec<&str> = map.split('\n').collect();
    assert_eq!(rows.len(), SIZE);
    assert!(
        rows.iter()
            .all(|row| row.chars().count() == SIZE && row.chars().all(|c| c == GLYPH))
    );
    let legend = plain(field(&body[1], "contents").unwrap());
    assert!(legend.contains("Territory, radius 48"), "{legend}");
    assert!(legend.contains("1 cell = 8×8 blocks"), "{legend}");
    assert!(legend.contains("No buildings yet."), "{legend}");
    assert_eq!(
        actions(&dialog),
        [
            "solaris:settlement/map",
            "solaris:settlement/buildings",
            "solaris:settlement/main"
        ]
    );
    // Equal neighbours share one run, and every run carries a hover.
    assert!(runs(&dialog).len() < SIZE * SIZE);
    assert!(
        runs(&dialog)
            .iter()
            .all(|run| plain(run) == "\n" || field(run, "hover_event").is_some())
    );
    // The claimed main page links the map; every settlement's Buildings page
    // does too (the compact unclaimed main page keeps its four buttons).
    let main = command(&mut state, "settlement menu main");
    assert!(actions(&main).contains(&"solaris:settlement/map".to_owned()));
    state.settlements.get_mut(id).unwrap().owner = None;
    let buildings = click(&mut state, id, "buildings");
    assert!(actions(&buildings).contains(&"solaris:settlement/map".to_owned()));
    // Reads stay open on an unclaimed village.
    let map = click(&mut state, id, "map");
    assert_eq!(string(field(&map, "title")), "Map - oak");
}

#[test]
fn buildings_sit_on_their_footprint_cells_with_hover_and_click() {
    let (mut state, id) = town();
    let house = add(
        &mut state,
        id,
        BuildingKind::House,
        [20, 201, 20],
        [24, 205, 24],
    );
    let grid = render(&state, id);
    for (x, z) in [(20, 20), (24, 24), (20, 24), (24, 20)] {
        let cell = at(&grid, x, z);
        assert_eq!(cell.color, HOUSE);
        assert_eq!(cell.hover, "House #1 L1\n20 201 20 .. 24 205 24");
        assert_eq!(cell.click, Some(Click::Building(house)));
    }
    assert_eq!(at(&grid, 14, 14).color, TERRITORY);
    assert_eq!(at(&grid, 30, 20).color, TERRITORY);
    assert!(grid.legend.contains(&(HOUSE, "House 1".to_owned())));
    assert_eq!(grid.buildings, 1);

    // The wire run is clickable: `buildings` with the settlement and building ids.
    let dialog = command(&mut state, "settlement menu map");
    let run = runs(&dialog)
        .iter()
        .find(|run| string(field(run, "color")) == "#E3C08D")
        .expect("a house run");
    let event = field(run, "click_event").unwrap();
    assert_eq!(string(field(event, "action")), "custom");
    assert_eq!(string(field(event, "id")), "solaris:settlement/buildings");
    let payload = field(event, "payload").unwrap();
    assert_eq!(string(field(payload, "s")), id.0.to_string());
    assert_eq!(string(field(payload, "b")), house.0.to_string());
    let legend = plain(field(&elements(field(&dialog, "body"))[1], "contents").unwrap());
    assert!(!legend.contains("No buildings yet."), "{legend}");
}

#[test]
fn refresh_redraws_live_state_and_tracks_the_viewer() {
    let (mut state, id) = town();
    let before = command(&mut state, "settlement menu map");
    assert!(
        runs(&before)
            .iter()
            .all(|run| string(field(run, "color")) != "#E3C08D")
    );
    add(
        &mut state,
        id,
        BuildingKind::House,
        [-20, 200, -20],
        [-16, 204, -16],
    );
    stand(&mut state, ANCHOR);
    let after = click(&mut state, id, "map");
    assert!(
        runs(&after)
            .iter()
            .any(|run| string(field(run, "color")) == "#E3C08D")
    );
    let grid = render(&state, id);
    let you = at(&grid, 8, 8);
    assert_eq!(you.color, YOU);
    assert_eq!(you.hover, "You are here: 8 8\nTerritory, radius 48");
    assert!(grid.legend.contains(&(YOU, "You".to_owned())));
    // Off the map: no marker and no legend entry.
    stand(&mut state, [1000, 201, 1000]);
    let grid = render(&state, id);
    assert!(grid.cells.iter().all(|cell| cell.color != YOU));
    assert!(!grid.legend.iter().any(|(color, _)| *color == YOU));
}

/// The site is the box the active project reserved in the structure ledger
/// (buildings, roads and lamp posts alike): here the storage the planner put
/// on the first street lot, turned to face the avenue.
#[test]
fn construction_site_covers_its_reserved_rotated_footprint() {
    use crate::settlements::construction::tests::{fixture, fund, open_plan};
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    crate::settlements::construction::request_build(&mut state, id, "storage").unwrap();
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
    assert_eq!((project.origin, project.rotation), ([11, 200, 13], 1));
    let grid = grid(&ctx(id), &state, state.settlements.get(id).unwrap(), None);
    // Storage is 9 wide and 5 deep; turned once it covers x 11..15, z 13..21.
    for (x, z) in [(11, 13), (15, 21)] {
        let cell = at(&grid, x, z);
        assert_eq!(cell.color, SITE);
        assert!(
            cell.hover.starts_with("Construction: Storage L1\n11 "),
            "{}",
            cell.hover
        );
        assert!(
            cell.hover.contains(" 13 .. 15 ") && cell.hover.ends_with(" 21"),
            "{}",
            cell.hover
        );
        assert_eq!(cell.click, Some(Click::Site));
    }
    assert!(grid.legend.contains(&(SITE, "Construction".to_owned())));
    let map = cells_component(&grid, id);
    let site = elements(field(&map, "extra"))
        .iter()
        .find(|run| string(field(run, "color")) == "#FF8800")
        .expect("a site run");
    assert_eq!(
        string(field(field(site, "click_event").unwrap(), "id")),
        "solaris:settlement/project_status"
    );
}

/// Street work the planner owes on the walk to a standing building is drawn
/// under the buildings, with a hover and no click; none without a building.
/// A lamp post wins the road cell it stands in (a point beside the road would
/// vanish into it).
#[test]
fn street_plan_work_is_drawn_under_buildings_without_clicks() {
    use crate::settlements::construction::tests::{fixture, open_plan};
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let grass = crate::settlements::allocator::tests::block(&state, "minecraft:grass_block", &[]);
    for x in 7..=9 {
        for z in 13..=23 {
            state.world.set_block(x, 200, z, grass);
        }
    }
    state.world.fold_overlay();
    let view = |state: &TickState| grid(&ctx(id), state, state.settlements.get(id).unwrap(), None);
    let empty = view(&state);
    assert!(
        empty
            .cells
            .iter()
            .all(|cell| cell.color != PLANNED_ROAD && cell.color != PLANNED_LAMP)
    );
    add(
        &mut state,
        id,
        BuildingKind::House,
        [11, 200, 25],
        [15, 204, 30],
    );
    let grid = view(&state);
    // The road and its nearby lamp share this 8×8 cell; both appear in the hover.
    let road = at(&grid, 8, 13);
    assert_eq!(road.color, PLANNED_LAMP);
    assert!(
        road.hover
            .contains("Street plan: unpaved road\n7 200 13 .. 9 200 23"),
        "{}",
        road.hover
    );
    assert_eq!(road.click, None);
    let post = at(&grid, 10, 19);
    assert_eq!(post.color, PLANNED_LAMP, "{}", post.hover);
    assert_eq!(post.click, None);
    let lamp = at(&grid, 10, 28);
    assert!(
        lamp.hover
            .contains("Street plan: lamp post site\n10 201 28"),
        "{}",
        lamp.hover
    );
    assert_eq!(at(&grid, 13, 27).color, HOUSE, "the house wins its cells");
    assert!(
        grid.legend
            .contains(&(PLANNED_ROAD, "Road (planned)".to_owned()))
    );
    assert!(
        grid.legend
            .contains(&(PLANNED_LAMP, "Lamp (planned)".to_owned()))
    );
}

/// A finished road (dirt path columns on the street plan) is drawn with its own
/// colour, hover and legend entry, with or without buildings, and is no click target.
#[test]
fn paved_road_is_drawn_and_named_in_the_legend() {
    use crate::settlements::construction::tests::{fixture, open_plan};
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let path = crate::settlements::allocator::tests::block(&state, "minecraft:dirt_path", &[]);
    for x in 7..=9 {
        for z in 13..=23 {
            state.world.set_block(x, 200, z, path);
        }
    }
    state.world.fold_overlay();
    let grid = grid(&ctx(id), &state, state.settlements.get(id).unwrap(), None);
    let road = at(&grid, 8, 15);
    assert_eq!(road.color, PAVED_ROAD);
    assert_eq!(road.hover, "Paved road\n7 200 13 .. 9 200 23");
    assert_eq!(road.click, None);
    assert!(
        grid.legend
            .contains(&(PAVED_ROAD, "Road (paved)".to_owned()))
    );
    assert!(grid.legend.iter().all(|(color, _)| *color != PLANNED_ROAD));
}

/// Every colour on the map is told apart at a glance, and the legend that names
/// them all - every kind, the site, the viewer and the whole street plan at once,
/// English and Russian - lays out in the lines that fit under the map, an entry
/// never split from its swatch.
#[test]
fn legend_names_every_colour_in_the_lines_that_fit_under_the_map() {
    let mut colours = vec![
        OUTSIDE,
        TERRITORY,
        SITE,
        YOU,
        PAVED_ROAD,
        PLANNED_ROAD,
        PLANNED_LAMP,
    ];
    let names = [
        "house",
        "storage",
        "armorer",
        "mason",
        "mine",
        "lumber_camp",
        "barracks",
        "wall",
        "tower",
        "gate",
    ];
    for name in names {
        colours.push(style(BuildingKind::parse(name).unwrap()).0);
    }
    colours.sort_unstable();
    colours.dedup();
    assert_eq!(
        colours.len(),
        7 + 9,
        "one colour per kind (workshops share one)"
    );
    for (index, a) in colours.iter().enumerate() {
        for b in &colours[index + 1..] {
            let distance: u32 = [16, 8, 0]
                .into_iter()
                .map(|shift| ((a >> shift) & 0xFF).abs_diff((b >> shift) & 0xFF))
                .sum();
            assert!(distance >= 80, "{a:06X} and {b:06X} are too close");
        }
    }

    let (mut state, id) = town();
    for (index, name) in names.into_iter().enumerate() {
        let at = -20 + 4 * index as i32;
        add(
            &mut state,
            id,
            BuildingKind::parse(name).unwrap(),
            [at, 201, 0],
            [at + 3, 205, 3],
        );
    }
    stand(&mut state, ANCHOR);
    for lang in [Lang::En, Lang::Ru] {
        let ctx = Ctx {
            id,
            manage: true,
            lang,
        };
        let record = state.settlements.get(id).unwrap();
        let mut grid = grid(&ctx, &state, record, player_pos(&state, CONN));
        for (colour, key) in [
            (SITE, Key::MapSite),
            (PAVED_ROAD, Key::MapPavedRoads),
            (PLANNED_ROAD, Key::MapUnpavedRoads),
            (PLANNED_LAMP, Key::MapLampSites),
        ] {
            if !grid.legend.iter().any(|(existing, _)| *existing == colour) {
                grid.legend.push((colour, ctx.t(key).to_owned()));
            }
        }
        assert_eq!(grid.legend.len(), 15, "{:?}", grid.legend);
        let text = plain(&legend_component(&ctx, &grid));
        let lines: Vec<&str> = text.split('\n').collect();
        assert!(
            lines.len() <= LEGEND_LINES,
            "{lang:?}: {} lines\n{text}",
            lines.len()
        );
        let (scale, entries) = lines.split_last().unwrap();
        assert!(scale.contains("8×8"), "{text}");
        for line in entries {
            assert!(
                line.starts_with(GLYPH) && !line.ends_with(GLYPH),
                "{line:?}"
            );
            assert!(!line.ends_with(' '), "{line:?}");
        }
        assert_eq!(text.matches(GLYPH).count(), 15);
    }
}

#[test]
fn every_radius_fits_the_grid_around_the_anchor() {
    let (mut state, id) = town();
    for radius in 0..=MAX_RADIUS {
        state.settlements.get_mut(id).unwrap().radius = radius;
        let grid = render(&state, id);
        let r = radius as i32;
        // Smallest scale whose SIZE cells span the whole 2r+1 diameter.
        assert!(grid.scale * SIZE as i32 > 2 * r, "radius {radius}");
        assert!(grid.scale == 1 || (grid.scale - 1) * (SIZE as i32) < 2 * r + 1);
        for x in [ANCHOR[0] - r, ANCHOR[0] + r] {
            at(&grid, x, ANCHOR[2]);
            at(&grid, ANCHOR[0], x - ANCHOR[0] + ANCHOR[2]);
        }
        assert_eq!(at(&grid, ANCHOR[0], ANCHOR[2]).color, TERRITORY);
    }
}

#[test]
fn russian_client_gets_russian_map_text() {
    let (state, id) = town();
    let record = state.settlements.get(id).unwrap();
    let ru = Ctx {
        id,
        manage: true,
        lang: Lang::Ru,
    };
    let grid = grid(&ru, &state, record, None);
    assert_eq!(grid.legend[0].1, "Территория, радиус 48");
    assert_eq!(ru.f(Key::TitleMap, &[("name", "oak")]), "Карта — oak");
}
