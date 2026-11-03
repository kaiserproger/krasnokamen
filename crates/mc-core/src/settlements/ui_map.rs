//! Settlement map page (`/settlement menu map`, the main page's **Map** button).
//!
//! A top-down grid of the live record: the territory disk (`anchor` ± `radius`
//! in X/Z, the widest cut of the settlement sphere), every certified building's
//! footprint, the active construction site (a building, road or lamp project,
//! as reserved in the structure ledger), the street plan ([`layout::street_features`]:
//! the finished road and the road and lamp work the planner owes on the walks to
//! standing buildings) and the viewer. North (-Z) is up, east (+X) right; each cell
//! covers `scale`×`scale` blocks, the smallest scale that fits the whole territory
//! into [`SIZE`] cells.
//!
//! The grid is one 26.1.2 dialog `plain_message` of `█` runs: the default font
//! draws `█` as an 8×8 bitmap with a 9-pixel advance and the line height is 9,
//! so the rows form square cells. Vanilla dialog text shows `show_text` hovers
//! and runs `custom` click events (`DialogBodyHandlers.PlainMessageHandler`:
//! a `FocusableTextWidget` with a component click handler), so hovering a cell
//! names what is there, clicking a building sends `buildings` with `b` = its id
//! and clicking the construction site opens Project status. Those clicks are
//! untrusted like every button: `dispatch` re-resolves `s` and permissions.
//!
//! The page is a snapshot of live state and **Refresh** rebuilds it. A vanilla
//! client does not report closing a dialog, so the server never re-sends the map
//! on its own: that would reopen a dialog the player already closed.

use mc_nbt::{ListTag, Tag};

use super::ui_text::Key;
use super::{
    Ctx, OWNER, blueprint_label, button, corner_text, kind_label, player_pos, present, short,
};
use crate::boundary::ConnectionId;
use crate::dialog::{Button, Dialog};
use crate::settlements::buildings::{BuildingId, BuildingKind};
use crate::settlements::layout;
use crate::settlements::registry::MAX_RADIUS;
use crate::settlements::{SettlementId, SettlementRecord};
use crate::tick::TickState;

/// Cells per side; odd, so the anchor sits in the centre cell. The dialog body
/// of a 720p window at GUI scale 3 is 174 GUI pixels tall and does not scroll in a
/// screenshot: 13 rows of 9 (117) + the 10-pixel gap leave 47 for the legend, i.e.
/// [`LEGEND_LINES`] lines of 9. 13 glyphs fit the default 200-pixel `plain_message`
/// width without wrapping.
const SIZE: usize = 13;
/// Legend `plain_message` width (the dialog's content is 410 wide at that size).
const LEGEND_WIDTH: u32 = 408;
/// Estimated pixel budget of one legend line, under [`LEGEND_WIDTH`] so the client
/// never re-wraps a line the packer laid out (a swatch stranded from its label).
const LEGEND_LINE: usize = 396;
/// Legend lines that fit under the map: the entries (at most four for every kind
/// and feature at once) and the scale line.
#[cfg(test)]
const LEGEND_LINES: usize = 5;
const GLYPH: char = '█';
/// Labels listed in one cell's hover before "(+n more)".
const MAX_HOVER: usize = 4;

const OUTSIDE: u32 = 0x2B2B2B;
const TERRITORY: u32 = 0x4C7A34;
const SITE: u32 = 0xFF8800;
const YOU: u32 = 0xFF55FF;
/// Street plan: finished road (a dirt path), a road stretch still to pave, a lamp
/// post still to set. No building kind shares a colour with them.
const PAVED_ROAD: u32 = 0x8A8D91;
const PLANNED_ROAD: u32 = 0xD7CCC8;
const PLANNED_LAMP: u32 = 0xB3E5FC;

/// Map colour and legend label of a building kind; all workshops share one.
fn style(kind: BuildingKind) -> (u32, Key) {
    match kind {
        BuildingKind::House => (0xE3C08D, Key::KindHouse),
        BuildingKind::Storage => (0x9C6B30, Key::KindStorage),
        BuildingKind::Workshop(_) => (0x3B82F6, Key::MapWorkshops),
        BuildingKind::Mine => (0x00BFA5, Key::KindMine),
        BuildingKind::LumberCamp => (0x8FD14F, Key::KindLumberCamp),
        BuildingKind::Barracks => (0xC0392B, Key::KindBarracks),
        BuildingKind::Wall => (0x34495E, Key::KindWall),
        BuildingKind::Tower => (0x7D3C98, Key::KindTower),
        BuildingKind::Gate => (0xF1C40F, Key::KindGate),
    }
}

/// What clicking a cell opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Click {
    Building(BuildingId),
    Site,
}

/// One drawn footprint, in X/Z block coordinates (inclusive). Planned street
/// work has no page to open.
struct Feature {
    min: [i32; 2],
    max: [i32; 2],
    color: u32,
    label: String,
    click: Option<Click>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    color: u32,
    hover: String,
    click: Option<Click>,
}

/// The rendered page: `cells[row * SIZE + column]`, row 0 north, column 0 west;
/// cell (0, 0) starts at `anchor - (SIZE / 2) * scale - scale / 2` on X and Z.
struct Grid {
    /// Blocks per cell side.
    scale: i32,
    cells: Vec<Cell>,
    /// Legend entries: colour and label.
    legend: Vec<(u32, String)>,
    buildings: usize,
}

/// `map` action: the grid, its legend, and Refresh / Buildings / Back. The grid
/// and the legend (every colour on the map, named) are what a 720p window at GUI
/// scale 3 shows without scrolling; the buttons share one row below them (reached
/// with the wheel), Close is the footer.
pub(super) fn screen(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    let viewer = player_pos(state, connection);
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let grid = grid(ctx, state, record, viewer);
    let title = ctx.f(Key::TitleMap, &[("name", &record.name)]);
    let buttons = [
        (Key::Refresh, "map"),
        (Key::Buildings, "buildings"),
        (Key::Back, "main"),
    ]
    .map(|(key, action)| button(ctx.t(key), action, ctx.id).width(100));
    let dialog = Dialog::multi_action(title, buttons.into())
        .columns(3)
        .text(cells_component(&grid, ctx.id))
        .text_width(legend_component(ctx, &grid), LEGEND_WIDTH)
        .exit(Button::plain(ctx.t(Key::Close)));
    present(state, connection, dialog);
}

fn grid(ctx: &Ctx, state: &TickState, record: &SettlementRecord, viewer: Option<[i32; 3]>) -> Grid {
    let radius = record.radius.min(MAX_RADIUS);
    let scale = (2 * radius + 1).div_ceil(SIZE as u32) as i32;
    let radius = radius as i32;
    let size = SIZE as i32;
    let half = size / 2;
    let anchor = [record.anchor[0], record.anchor[2]];
    let origin = anchor.map(|centre| centre - half * scale - scale / 2);
    let features = features(ctx, state, record);
    let you = viewer.map(|at| [at[0], at[2]]);
    let mut cells = Vec::with_capacity(SIZE * SIZE);
    let mut you_shown = false;
    for row in 0..size {
        for column in 0..size {
            let min = [origin[0] + column * scale, origin[1] + row * scale];
            let max = [min[0] + scale - 1, min[1] + scale - 1];
            let here: Vec<&Feature> = features
                .iter()
                .filter(|feature| {
                    (0..2).all(|axis| {
                        feature.min[axis] <= max[axis] && min[axis] <= feature.max[axis]
                    })
                })
                .collect();
            let mine = you
                .is_some_and(|at| (0..2).all(|axis| (min[axis]..=max[axis]).contains(&at[axis])));
            you_shown |= mine;
            // Territory by cell centre, doubled to stay in integers.
            let inside = (0..2)
                .map(|axis| i64::from(min[axis] + max[axis] - 2 * anchor[axis]).pow(2))
                .sum::<i64>()
                <= (2 * i64::from(radius)).pow(2);
            let mut lines = Vec::new();
            if let (true, Some(at)) = (mine, viewer) {
                lines.push(ctx.f(
                    Key::MapYouAt,
                    &[("x", &at[0].to_string()), ("z", &at[2].to_string())],
                ));
            }
            lines.extend(
                here.iter()
                    .take(MAX_HOVER)
                    .map(|feature| feature.label.clone()),
            );
            if here.len() > MAX_HOVER {
                lines.push(ctx.f(Key::More, &[("n", &(here.len() - MAX_HOVER).to_string())]));
            }
            if here.is_empty() {
                lines.push(if inside {
                    ctx.f(Key::MapTerritory, &[("radius", &radius.to_string())])
                } else {
                    ctx.t(Key::MapOutside).to_owned()
                });
            }
            let color = match (mine, here.first()) {
                (true, _) => YOU,
                (false, Some(feature)) => feature.color,
                (false, None) if inside => TERRITORY,
                (false, None) => OUTSIDE,
            };
            cells.push(Cell {
                color,
                hover: lines.join("\n"),
                click: here.first().and_then(|feature| feature.click),
            });
        }
    }
    let mut legend = vec![(
        TERRITORY,
        ctx.f(Key::MapTerritory, &[("radius", &radius.to_string())]),
    )];
    if you_shown {
        legend.push((YOU, ctx.t(Key::MapYou).to_owned()));
    }
    if features
        .iter()
        .any(|feature| feature.click == Some(Click::Site))
    {
        legend.push((SITE, ctx.t(Key::MapSite).to_owned()));
    }
    for (color, key) in [
        (PAVED_ROAD, Key::MapPavedRoads),
        (PLANNED_ROAD, Key::MapUnpavedRoads),
        (PLANNED_LAMP, Key::MapLampSites),
    ] {
        if features.iter().any(|feature| feature.color == color) {
            legend.push((color, ctx.t(key).to_owned()));
        }
    }
    let mut kinds: Vec<(u32, Key, usize)> = Vec::new();
    for building in record.buildings.values() {
        let (color, key) = style(building.kind);
        match kinds.iter_mut().find(|entry| entry.1 == key) {
            Some(entry) => entry.2 += 1,
            None => kinds.push((color, key, 1)),
        }
    }
    legend.extend(
        kinds
            .into_iter()
            .map(|(color, key, count)| (color, format!("{} {count}", ctx.t(key)))),
    );
    Grid {
        scale,
        cells,
        legend,
        buildings: record.buildings.len(),
    }
}

/// The construction site first (it wins its cells, so an upgrade in progress
/// shows over its building), then every certified building by id, then the
/// street plan under the buildings: lamp posts (a point beside the road would
/// vanish into it), owed road work, the finished road last.
fn features(ctx: &Ctx, state: &TickState, record: &SettlementRecord) -> Vec<Feature> {
    let mut planned = layout::street_features(state, record);
    planned.sort_by_key(|(kind, _)| match *kind {
        layout::LAMP => 0,
        layout::PAVED => 2,
        _ => 1,
    });
    let mut out = Vec::with_capacity(record.buildings.len() + planned.len() + 1);
    if let Some(project) = &record.project
        && let Some(bounds) = layout::project_bounds(state, project)
    {
        out.push(Feature {
            min: [bounds.min[0], bounds.min[2]],
            max: [bounds.max[0], bounds.max[2]],
            color: SITE,
            label: ctx.f(
                Key::MapSiteAt,
                &[
                    ("blueprint", blueprint_label(ctx, &project.blueprint)),
                    ("level", &project.level.to_string()),
                    ("from", &corner_text(bounds.min)),
                    ("to", &corner_text(bounds.max)),
                ],
            ),
            click: Some(Click::Site),
        });
    }
    for building in record.buildings.values() {
        let bounds = building.bounds;
        out.push(Feature {
            min: [bounds.min[0], bounds.min[2]],
            max: [bounds.max[0], bounds.max[2]],
            color: style(building.kind).0,
            label: ctx.f(
                Key::MapBuilding,
                &[
                    ("kind", &kind_label(ctx, building.kind)),
                    ("seq", &short(building.id).to_string()),
                    ("level", &building.level.to_string()),
                    ("from", &corner_text(bounds.min)),
                    ("to", &corner_text(bounds.max)),
                ],
            ),
            click: Some(Click::Building(building.id)),
        });
    }
    for (kind, bounds) in planned {
        let (from, to) = (corner_text(bounds.min), corner_text(bounds.max));
        let span = [("from", from.as_str()), ("to", to.as_str())];
        let (color, label) = match kind {
            layout::LAMP => (
                PLANNED_LAMP,
                ctx.f(Key::MapLampSite, &[("at", &corner_text(bounds.min))]),
            ),
            layout::PAVED => (PAVED_ROAD, ctx.f(Key::MapPavedRoad, &span)),
            _ => (PLANNED_ROAD, ctx.f(Key::MapUnpavedRoad, &span)),
        };
        out.push(Feature {
            min: [bounds.min[0], bounds.min[2]],
            max: [bounds.max[0], bounds.max[2]],
            color,
            label,
            click: None,
        });
    }
    out
}

fn literal(text: impl Into<String>) -> Tag {
    Tag::Compound(vec![("text".to_owned(), Tag::String(text.into()))])
}

fn colored(text: impl Into<String>, color: u32) -> Vec<(String, Tag)> {
    vec![
        ("text".to_owned(), Tag::String(text.into())),
        ("color".to_owned(), Tag::String(format!("#{color:06X}"))),
    ]
}

fn root(parts: Vec<Tag>) -> Tag {
    Tag::Compound(vec![
        ("text".to_owned(), Tag::String(String::new())),
        (
            "extra".to_owned(),
            Tag::List(ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: parts,
            }),
        ),
    ])
}

/// One text run per stretch of equal cells in a row, rows joined by newlines.
fn cells_component(grid: &Grid, id: SettlementId) -> Tag {
    let mut parts = Vec::new();
    for row in grid.cells.chunks(SIZE) {
        if !parts.is_empty() {
            parts.push(literal("\n"));
        }
        let mut start = 0;
        while start < row.len() {
            let cell = &row[start];
            let length = row[start..]
                .iter()
                .take_while(|other| *other == cell)
                .count();
            let mut entries = colored(GLYPH.to_string().repeat(length), cell.color);
            entries.push((
                "hover_event".to_owned(),
                Tag::Compound(vec![
                    ("action".to_owned(), Tag::String("show_text".to_owned())),
                    ("value".to_owned(), Tag::String(cell.hover.clone())),
                ]),
            ));
            if let Some(click) = cell.click {
                let mut payload = vec![("s".to_owned(), Tag::String(id.0.to_string()))];
                let action = match click {
                    Click::Building(building) => {
                        payload.push(("b".to_owned(), Tag::String(building.0.to_string())));
                        "buildings"
                    }
                    Click::Site => "project_status",
                };
                entries.push((
                    "click_event".to_owned(),
                    Tag::Compound(vec![
                        ("action".to_owned(), Tag::String("custom".to_owned())),
                        (
                            "id".to_owned(),
                            Tag::String(format!("solaris:{OWNER}/{action}")),
                        ),
                        ("payload".to_owned(), Tag::Compound(payload)),
                    ]),
                ));
            }
            parts.push(Tag::Compound(entries));
            start += length;
        }
    }
    root(parts)
}

/// Rough pixel width of `text` in the default font, meant never to be under the
/// real one: ASCII glyphs are at most 6 (5 + 1), the rest (Cyrillic) at most 8.
fn estimated_width(text: &str) -> usize {
    text.chars()
        .map(|ch| if ch.is_ascii() { 6 } else { 8 })
        .sum()
}

/// Coloured swatches with labels packed into lines under [`LEGEND_LINE`] (an entry
/// never splits across lines, so no swatch is left without its label), then the
/// scale line (and the empty note).
fn legend_component(ctx: &Ctx, grid: &Grid) -> Tag {
    let mut parts = Vec::new();
    let mut used = 0;
    for (color, label) in &grid.legend {
        // Swatch advance 9 + a 4-pixel space + the label.
        let width = 13 + estimated_width(label);
        if used == 0 {
            used = width;
        } else if used + 12 + width > LEGEND_LINE {
            parts.push(literal("\n"));
            used = width;
        } else {
            parts.push(literal("   "));
            used += 12 + width;
        }
        parts.push(Tag::Compound(colored(GLYPH, *color)));
        parts.push(literal(format!(" {label}")));
    }
    let mut note = ctx.f(Key::MapScale, &[("scale", &grid.scale.to_string())]);
    if grid.buildings == 0 {
        note.push('\n');
        note.push_str(ctx.t(Key::MapEmpty));
    }
    parts.push(literal(format!("\n{note}")));
    root(parts)
}

#[cfg(test)]
#[path = "ui_map_tests.rs"]
mod tests;
