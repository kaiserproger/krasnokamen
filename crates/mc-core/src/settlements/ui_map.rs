//! Settlement map page (`/settlement menu map`, the dashboard's **Map** icon).
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
//! names what is there. Cell clicks carry every stable building/site identity to
//! a paged picker; one feature opens its own card directly. Clicks are untrusted
//! like every button: `dispatch` re-resolves the settlement and permissions.
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

/// Thirteen nine-pixel rows leave room for two compact action rows at scale 3.
/// The full legend has its own page; it must not displace navigation.
const SIZE: usize = 13;
const LEGEND_WIDTH: u32 = 408;
const LEGEND_LINE: usize = 396;
const GLYPH: char = '█';
/// Labels listed in one cell's hover before "(+n more)".
const MAX_HOVER: usize = 4;

const OUTSIDE: u32 = 0x2B2B2B;
const TERRITORY: u32 = 0x4C7A34;
const SITE: u32 = 0xFF8800;
const PAUSED_SITE: u32 = 0x880000;
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
#[derive(Debug, Clone, PartialEq, Eq)]
enum Click {
    Building(BuildingId),
    Site(String),
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
    click: Vec<Click>,
    bounds: [[i32; 2]; 2],
    streets: String,
}

/// The rendered page: `cells[row * SIZE + column]`, row 0 north, column 0 west;
/// cell (0, 0) starts at `anchor - (SIZE / 2) * scale - scale / 2` on X and Z.
struct Grid {
    /// Blocks per cell side.
    scale: i32,
    centre: [i32; 2],
    cells: Vec<Cell>,
    /// Legend entries: colour and label.
    legend: Vec<(u32, String)>,
    buildings: usize,
}

pub(super) fn screen_at(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: crate::dialog::ClickPayload<'_>,
) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let overview = ((2 * record.radius.min(MAX_RADIUS) + 1).div_ceil(SIZE as u32) as i32).max(1);
    let scale = payload
        .string("ms")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(overview)
        .clamp(1, overview);
    let centre = [("mx", record.anchor[0]), ("mz", record.anchor[2])].map(|(key, anchor)| {
        payload
            .string(key)
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(anchor)
            .clamp(
                anchor.saturating_sub(record.radius as i32),
                anchor.saturating_add(record.radius as i32),
            )
    });
    let grid = grid_at(
        ctx,
        state,
        record,
        player_pos(state, connection),
        centre,
        scale,
    );
    let title = format!(
        "{} · N↑ · 1:{} · {},{}",
        ctx.t(Key::Map),
        scale,
        centre[0],
        centre[1]
    );
    let step = scale * (SIZE as i32 / 2);
    let mut buttons = Vec::new();
    for (en, ru, at, zoom) in [
        ("Refresh", "Обновить", centre, scale),
        (
            "Overview",
            "Обзор",
            [record.anchor[0], record.anchor[2]],
            overview,
        ),
        ("Zoom +", "Зум +", centre, (scale / 2).max(1)),
        ("Zoom -", "Зум -", centre, (scale * 2).min(overview)),
        (
            "N ↑",
            "С ↑",
            [centre[0], centre[1].saturating_sub(step)],
            scale,
        ),
        (
            "E →",
            "В →",
            [centre[0].saturating_add(step), centre[1]],
            scale,
        ),
        (
            "S ↓",
            "Ю ↓",
            [centre[0], centre[1].saturating_add(step)],
            scale,
        ),
        (
            "W ←",
            "З ←",
            [centre[0].saturating_sub(step), centre[1]],
            scale,
        ),
    ] {
        buttons.push(view_button(
            ctx,
            if ctx.lang == super::Lang::Ru { ru } else { en },
            "map",
            at,
            zoom,
        ));
    }
    buttons.push(
        button(
            if ctx.lang == super::Lang::Ru {
                "Здания"
            } else {
                "Buildings"
            },
            "buildings",
            ctx.id,
        )
        .width(60),
    );
    buttons.push(
        button(
            if ctx.lang == super::Lang::Ru {
                "Проекты"
            } else {
                "Projects"
            },
            "projects",
            ctx.id,
        )
        .width(60),
    );
    buttons.push(view_button(
        ctx,
        if ctx.lang == super::Lang::Ru {
            "Легенда"
        } else {
            "Legend"
        },
        "map_legend",
        centre,
        scale,
    ));
    buttons.push(button(ctx.t(Key::Back), "dashboard", ctx.id).width(60));
    let mut dialog = Dialog::multi_action(title, buttons)
        .columns(6)
        .text(cells_component(&grid, ctx.id));
    if let Some(project) = &record.project {
        let caption = match crate::plugin_structures::settlement_builds::status(
            state,
            ctx.id,
            &project.structure,
        ) {
            Ok(status) if status.state.as_str() == "paused" || status.pause_reason.is_some() => {
                Some(if ctx.lang == super::Lang::Ru {
                    "Пауза — откройте стройплощадку"
                } else {
                    "Paused — open the construction site"
                })
            }
            Err(_) => Some(if ctx.lang == super::Lang::Ru {
                "Состояние стройки недоступно"
            } else {
                "Construction state unavailable"
            }),
            _ => None,
        };
        if let Some(caption) = caption {
            dialog = dialog.text(caption);
        }
    }
    present(state, connection, dialog);
}

fn view_button(ctx: &Ctx, label: &str, action: &str, centre: [i32; 2], scale: i32) -> Button {
    button(label, action, ctx.id)
        .width(60)
        .with("mx", Tag::String(centre[0].to_string()))
        .with("mz", Tag::String(centre[1].to_string()))
        .with("ms", Tag::String(scale.to_string()))
}

pub(super) fn legend(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: crate::dialog::ClickPayload<'_>,
) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let grid = grid(ctx, state, record, player_pos(state, connection));
    let centre = [("mx", record.anchor[0]), ("mz", record.anchor[2])].map(|(key, fallback)| {
        payload
            .string(key)
            .and_then(|s| s.parse().ok())
            .unwrap_or(fallback)
    });
    let scale = payload
        .string("ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(grid.scale);
    present(
        state,
        connection,
        Dialog::notice(
            ctx.t(Key::Map),
            view_button(ctx, ctx.t(Key::Back), "map", centre, scale),
        )
        .text_width(legend_component(ctx, &grid), LEGEND_WIDTH),
    );
}

/// Selection is bound to IDs carried by the rendered cell, not a new first-hit
/// lookup. Removed features refuse individually; a new overlapping feature cannot
/// take the place of the one the player saw.
pub(super) fn pick(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: crate::dialog::ClickPayload<'_>,
) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let ids = payload.string("bids").unwrap_or("");
    let page = payload
        .string("page")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let site = payload.string("site").unwrap_or("");
    let centre = ["mx", "mz"].map(|key| {
        payload
            .string(key)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    });
    let scale = payload
        .string("ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let streets = payload.string("streets").unwrap_or("");
    if let Some(page) = payload.string("sp").and_then(|s| s.parse::<usize>().ok()) {
        let count = streets.lines().count().max(1);
        let page = page.min(count - 1);
        let mut buttons = vec![view_button(ctx, ctx.t(Key::Back), "map", centre, scale)];
        for next in [page.checked_sub(1), (page + 1 < count).then_some(page + 1)]
            .into_iter()
            .flatten()
        {
            buttons.push(
                view_button(
                    ctx,
                    if next < page { "←" } else { "→" },
                    "map_pick",
                    centre,
                    scale,
                )
                .with("streets", Tag::String(streets.to_owned()))
                .with("sp", Tag::String(next.to_string())),
            );
        }
        present(
            state,
            connection,
            Dialog::multi_action(
                format!("{} {}/{}", ctx.t(Key::Map), page + 1, count),
                buttons,
            )
            .text(streets.lines().nth(page).unwrap_or("")),
        );
        return;
    }
    // Only a single stable feature bypasses the selection page.
    if streets.is_empty()
        && !ids.is_empty()
        && !ids.contains(',')
        && site.is_empty()
        && let Ok(raw) = ids.parse::<u64>()
        && record.buildings.contains_key(&BuildingId(raw))
    {
        let payload = Tag::Compound(vec![
            ("from".to_owned(), Tag::String("map".to_owned())),
            ("mx".to_owned(), Tag::String(centre[0].to_string())),
            ("mz".to_owned(), Tag::String(centre[1].to_string())),
            ("ms".to_owned(), Tag::String(scale.to_string())),
        ]);
        super::menus::building_from(
            state,
            connection,
            ctx,
            BuildingId(raw),
            crate::dialog::ClickPayload(Some(&payload)),
        );
        return;
    }
    if streets.is_empty()
        && ids.is_empty()
        && !site.is_empty()
        && record
            .project
            .as_ref()
            .is_some_and(|project| project.structure == site)
    {
        super::menus::project(state, connection, ctx);
        return;
    }
    let mut choices = Vec::new();
    if !site.is_empty() {
        if record
            .project
            .as_ref()
            .is_some_and(|project| project.structure == site)
        {
            choices.push(button(ctx.t(Key::Status), "project_status", ctx.id));
        } else {
            choices.push(view_button(
                ctx,
                ctx.t(Key::StateChanged),
                "map",
                centre,
                scale,
            ));
        }
    }
    for id in ids
        .split(',')
        .filter_map(|id| id.parse::<u64>().ok())
        .map(BuildingId)
    {
        if let Some(building) = record.buildings.get(&id) {
            choices.push(
                view_button(
                    ctx,
                    &format!(
                        "{} #{} L{}",
                        kind_label(ctx, building.kind),
                        short(id),
                        building.level
                    ),
                    "buildings",
                    centre,
                    scale,
                )
                .width(300)
                .with("b", Tag::String(id.0.to_string()))
                .with("from", Tag::String("map".to_owned())),
            );
        } else {
            choices.push(
                view_button(
                    ctx,
                    &format!("{} #{}", ctx.t(Key::BuildingGone), short(id)),
                    "map",
                    centre,
                    scale,
                )
                .width(300),
            );
        }
    }
    let pages = choices.len().div_ceil(4).max(1);
    let page = page.min(pages - 1);
    let mut buttons: Vec<_> = choices.into_iter().skip(page * 4).take(4).collect();
    for next in [page.checked_sub(1), (page + 1 < pages).then_some(page + 1)]
        .into_iter()
        .flatten()
    {
        buttons.push(
            view_button(
                ctx,
                if next < page { "←" } else { "→" },
                "map_pick",
                centre,
                scale,
            )
            .with("bids", Tag::String(ids.to_owned()))
            .with("site", Tag::String(site.to_owned()))
            .with("streets", Tag::String(streets.to_owned()))
            .with(
                "cell",
                Tag::String(payload.string("cell").unwrap_or("").to_owned()),
            )
            .with("page", Tag::String(next.to_string())),
        );
    }
    if !streets.is_empty() {
        buttons.push(
            view_button(ctx, ctx.t(Key::Status), "map_pick", centre, scale)
                .with("streets", Tag::String(streets.to_owned()))
                .with("sp", Tag::String("0".to_owned())),
        );
    }
    buttons.push(view_button(ctx, ctx.t(Key::Back), "map", centre, scale));
    let text = payload
        .string("cell")
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");
    present(
        state,
        connection,
        Dialog::multi_action(
            format!("{} {}/{}", ctx.t(Key::Map), page + 1, pages),
            buttons,
        )
        .columns(1)
        .text(text)
        .exit(view_button(ctx, ctx.t(Key::Back), "map", centre, scale)),
    );
}

fn grid(ctx: &Ctx, state: &TickState, record: &SettlementRecord, viewer: Option<[i32; 3]>) -> Grid {
    let scale = ((2 * record.radius.min(MAX_RADIUS) + 1).div_ceil(SIZE as u32) as i32).max(1);
    grid_at(
        ctx,
        state,
        record,
        viewer,
        [record.anchor[0], record.anchor[2]],
        scale,
    )
}

fn grid_at(
    ctx: &Ctx,
    state: &TickState,
    record: &SettlementRecord,
    viewer: Option<[i32; 3]>,
    centre: [i32; 2],
    scale: i32,
) -> Grid {
    let radius = record.radius.min(MAX_RADIUS);
    let radius = radius as i32;
    let size = SIZE as i32;
    let half = size / 2;
    let anchor = [record.anchor[0], record.anchor[2]];
    let origin = centre.map(|centre| centre.saturating_sub(half * scale + scale / 2));
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
                .map(|axis| {
                    (i64::from(min[axis]) + i64::from(max[axis]) - 2 * i64::from(anchor[axis]))
                        .pow(2)
                })
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
                click: here
                    .iter()
                    .filter_map(|feature| feature.click.clone())
                    .collect(),
                bounds: [min, max],
                streets: here
                    .iter()
                    .filter(|feature| feature.click.is_none())
                    .map(|feature| feature.label.replace('\n', " · "))
                    .collect::<Vec<_>>()
                    .join("\n"),
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
    if let Some(feature) = features
        .iter()
        .find(|feature| matches!(feature.click, Some(Click::Site(_))))
    {
        legend.push((feature.color, feature.label.clone()));
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
        centre,
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
        let status = crate::plugin_structures::settlement_builds::status(
            state,
            record.id,
            &project.structure,
        )
        .ok();
        let paused = status.as_ref().is_some_and(|snapshot| {
            snapshot.state.as_str() == "paused" || snapshot.pause_reason.is_some()
        });
        let caption = match &status {
            Some(snapshot) => {
                let phase = super::project_phase(ctx, snapshot.state.as_str());
                snapshot
                    .pause_reason
                    .as_ref()
                    .map_or_else(|| phase.to_owned(), |reason| format!("{phase}: {reason}"))
            }
            None => match ctx.lang {
                super::Lang::En => "Construction state unavailable; cause unknown".to_owned(),
                super::Lang::Ru => "Состояние стройки недоступно; причина неизвестна".to_owned(),
            },
        };
        out.push(Feature {
            min: [bounds.min[0], bounds.min[2]],
            max: [bounds.max[0], bounds.max[2]],
            color: if paused { PAUSED_SITE } else { SITE },
            label: format!(
                "{caption}\n{}",
                ctx.f(
                    Key::MapSiteAt,
                    &[
                        ("blueprint", blueprint_label(ctx, &project.blueprint)),
                        ("level", &project.level.to_string()),
                        ("from", &corner_text(bounds.min)),
                        ("to", &corner_text(bounds.max)),
                    ],
                )
            ),
            click: Some(Click::Site(project.structure.clone())),
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

/// One independently clickable run per bounded cell, with rows joined by newlines.
fn cells_component(grid: &Grid, id: SettlementId) -> Tag {
    let mut parts = Vec::new();
    for row in grid.cells.chunks(SIZE) {
        if !parts.is_empty() {
            parts.push(literal("\n"));
        }
        for cell in row {
            let mut entries = colored(GLYPH, cell.color);
            entries.push((
                "hover_event".to_owned(),
                Tag::Compound(vec![
                    ("action".to_owned(), Tag::String("show_text".to_owned())),
                    ("value".to_owned(), Tag::String(cell.hover.clone())),
                ]),
            ));
            {
                let buildings = cell
                    .click
                    .iter()
                    .filter_map(|click| match click {
                        Click::Building(building) => Some(building.0.to_string()),
                        Click::Site(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let site = cell
                    .click
                    .iter()
                    .find_map(|click| match click {
                        Click::Site(site) => Some(site.as_str()),
                        _ => None,
                    })
                    .unwrap_or("");
                let payload = vec![
                    ("s".to_owned(), Tag::String(id.0.to_string())),
                    ("bids".to_owned(), Tag::String(buildings)),
                    ("site".to_owned(), Tag::String(site.to_owned())),
                    ("mx".to_owned(), Tag::String(grid.centre[0].to_string())),
                    ("mz".to_owned(), Tag::String(grid.centre[1].to_string())),
                    ("ms".to_owned(), Tag::String(grid.scale.to_string())),
                    (
                        "cell".to_owned(),
                        Tag::String(format!(
                            "X {}..{} · Z {}..{}\n{}",
                            cell.bounds[0][0],
                            cell.bounds[1][0],
                            cell.bounds[0][1],
                            cell.bounds[1][1],
                            cell.hover
                        )),
                    ),
                    ("streets".to_owned(), Tag::String(cell.streets.clone())),
                ];
                let action = "map_pick";
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
