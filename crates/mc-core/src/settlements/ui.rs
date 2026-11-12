//! Settlement management dialogs (26.1.2 server-driven UI): `/settlement`,
//! `/settlement menu [page]`, and every button click routed here as
//! `krasnokamen:settlement/<action>` by [`crate::dialog::route_click`].
//!
//! Authority stays in the tick: a dialog is a view built from live state, and a
//! click is an untrusted request. Every click re-resolves the settlement from the
//! `s` payload entry, re-derives permission (record owner or operator, the same
//! rule as the chat commands; reads are open) and re-validates all other input
//! before calling the existing native paths ([`buildings::certify`],
//! [`buildings::upgrade`], production [`Action`]s, registry rename).
//!
//! Titles, buttons, labels and notes come from [`ui_text`]: the language is the
//! connection's client locale (`ClientInformation.language` carried by the edge),
//! Russian for `ru*` and English otherwise. Game data (item, blueprint,
//! profession and settlement names) stays as authored.

use std::collections::BTreeSet;

use mc_data::Identifier;
use mc_nbt::Tag;
use mc_script::{
    ScriptOperationFailure as Failure, ScriptSettlementBuildingRef,
    ScriptSettlementEconomyAction as Action, ScriptWorldBounds,
};
use uuid::Uuid;

use super::buildings::{self, BuildingId, BuildingKind, CertError};
use super::ui_text::{self, Key, Lang};
use super::{SettlementId, economy_customization};
use crate::boundary::{ConnectionId, PlayerChange};
use crate::dialog::{self, Button, ClickPayload, Dialog, Input};
use crate::tick::TickState;
use crate::villager::VillagerProfession as Profession;

pub(crate) const OWNER: &str = "settlement";

/// The two block positions a player marked for the certify form.
pub(crate) type Marks = [Option<[i32; 3]>; 2];

const MAX_LISTED: usize = 12;
const MAX_RESIDENTS: usize = 30;
const MAX_STATIONS: usize = 4;
const MAX_MARKS: usize = 64;
const MAX_ORDER: f32 = 64.0;
const MAX_EXTRA_HEIGHT: f32 = 24.0;

/// Actions that change state; everything else is a read-only screen.
const MUTATING: &[&str] = &[
    "certify",
    "mark1",
    "mark2",
    "certify_do",
    "upgrade",
    "project_order",
    "project_upgrade",
    "project_cancel_pending",
    "order_start",
    "order_pick",
    "order_create",
    "order_pause",
    "order_resume",
    "order_cancel",
    "rename",
    "army_recruit",
    "army_dismiss",
    "army_order",
];

struct Ctx {
    id: SettlementId,
    manage: bool,
    /// Dialog language for this connection's client locale.
    lang: Lang,
}

impl Ctx {
    fn t(&self, key: Key) -> &'static str {
        ui_text::text(self.lang, key)
    }

    /// Translated `key` with `{name}` placeholders filled in.
    fn f(&self, key: Key, args: &[(&str, &str)]) -> String {
        ui_text::fill(self.t(key), args)
    }
}

struct Station {
    building: BuildingId,
    kind: BuildingKind,
    at: [i32; 3],
    block: String,
}

#[derive(Default)]
struct Form {
    kind: String,
    from: String,
    to: String,
    extra: f32,
}

// ---------------------------------------------------------------- entry points

/// `/settlement` and `/settlement menu [page]`: the settlement around the player
/// (else one they own), on the requested page.
pub(crate) fn open_menu(state: &mut TickState, connection: ConnectionId, page: Option<&str>) {
    let lang = Lang::of(state.connections.locale_of(connection));
    let Some(id) = locate(state, connection) else {
        say(state, connection, ui_text::text(lang, Key::NoSettlement));
        return;
    };
    let Some(ctx) = ctx(state, connection, id) else {
        say(
            state,
            connection,
            ui_text::text(lang, Key::AdmissionUnavailable),
        );
        return;
    };
    let action = match page.unwrap_or("main") {
        "main" => "main",
        "buildings" => "buildings",
        "map" => "map",
        "certify" => "certify",
        "residents" | "jobs" => "residents",
        "storage" => "storage",
        "production" | "orders" => "orders",
        "army" => "army",
        "projects" => "projects",
        "settings" => "settings",
        _ => {
            say(state, connection, ui_text::text(lang, Key::Pages));
            return;
        }
    };
    dispatch(state, connection, &ctx, action, ClickPayload(None));
}

/// Opens the main settlement page for a known settlement anchor.
pub(crate) fn open_for_settlement(
    state: &mut TickState,
    connection: ConnectionId,
    id: SettlementId,
) {
    let Some(ctx) = ctx(state, connection, id) else {
        let lang = Lang::of(state.connections.locale_of(connection));
        say(
            state,
            connection,
            ui_text::text(lang, Key::AdmissionUnavailable),
        );
        return;
    };
    dispatch(state, connection, &ctx, "main", ClickPayload(None));
}

/// One `krasnokamen:settlement/<action>` click.
pub(crate) fn click(
    state: &mut TickState,
    connection: ConnectionId,
    action: &str,
    payload: ClickPayload<'_>,
) {
    let Some(id) = payload
        .string("s")
        .and_then(|text| text.parse().ok())
        .map(SettlementId)
    else {
        return;
    };
    let Some(ctx) = ctx(state, connection, id) else {
        let lang = Lang::of(state.connections.locale_of(connection));
        say(state, connection, ui_text::text(lang, Key::SettlementGone));
        return;
    };
    dispatch(state, connection, &ctx, action, payload);
}

fn dispatch(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    action: &str,
    payload: ClickPayload<'_>,
) {
    let spectator = state
        .connections
        .entity_of(connection)
        .and_then(|entity| state.players.get(&entity))
        .is_some_and(crate::player::PlayerState::spectator);
    if MUTATING.contains(&action) && (!ctx.manage || spectator) {
        main_screen(state, connection, ctx, ctx.t(Key::Denied));
        return;
    }
    match action {
        "main" => main_screen(state, connection, ctx, ""),
        "dashboard" => menus::dashboard(state, connection, ctx),
        "help" => help_screen(state, connection, ctx, payload),
        "neighbours" => present(
            state,
            connection,
            Dialog::notice(
                ctx.t(Key::Neighbours),
                button(ctx.t(Key::Back), "main", ctx.id),
            )
            .text(ctx.t(Key::NeighboursUnavailable)),
        ),
        "checklist" => present(
            state,
            connection,
            Dialog::notice(
                ctx.t(Key::Checklist),
                button(ctx.t(Key::Back), "dashboard", ctx.id),
            )
            .text(ctx.t(Key::ChecklistUnavailable)),
        ),
        "next" => {
            let Some(record) = state.settlements.get(ctx.id) else {
                return;
            };
            if record.owner.is_none() {
                claim_screen(state, connection, ctx);
            } else if record.project.is_some() || record.pending_order.is_some() {
                dispatch(state, connection, ctx, "project_status", ClickPayload(None));
            } else if record.buildings.is_empty() {
                dispatch(state, connection, ctx, "buildings", ClickPayload(None));
            } else {
                menus::dashboard(state, connection, ctx);
            }
        }
        "claim_review" => claim_screen(state, connection, ctx),
        "map_pick" => map::pick(state, connection, ctx, payload),
        "map_legend" => map::legend(state, connection, ctx, payload),
        "building_filters" => building_filters(state, connection, ctx, payload),
        "buildings_filter" => menus::filtered_buildings(state, connection, ctx, payload),
        "project_estimate" => project_estimate(state, connection, ctx, payload),
        "resources" => menus::resources(state, connection, ctx),
        "recipe_review" => recipe_review(state, connection, ctx, payload),
        "building_map" => {
            let Some(id) = building_of(state, ctx, payload) else {
                return menus::buildings(state, connection, ctx);
            };
            let Some(building) = state
                .settlements
                .get(ctx.id)
                .and_then(|record| record.buildings.get(&id))
            else {
                return;
            };
            let payload = Tag::Compound(vec![
                (
                    "mx".to_owned(),
                    Tag::String(
                        (building.bounds.min[0]
                            + (building.bounds.max[0] - building.bounds.min[0]) / 2)
                            .to_string(),
                    ),
                ),
                (
                    "mz".to_owned(),
                    Tag::String(
                        (building.bounds.min[2]
                            + (building.bounds.max[2] - building.bounds.min[2]) / 2)
                            .to_string(),
                    ),
                ),
                ("ms".to_owned(), Tag::String("1".to_owned())),
            ]);
            map::screen_at(state, connection, ctx, ClickPayload(Some(&payload)));
        }
        "certificate_review" => certificate_review(state, connection, ctx, payload),
        "project_order_review" | "project_upgrade_review" | "project_cancel_review" => {
            project_review(state, connection, ctx, action, payload)
        }
        "claim" => {
            let note = match super::commands::claim(state, connection, Some(ctx.id)) {
                Ok(message) => message,
                Err(message) => message.to_owned(),
            };
            if let Some(ctx) = self::ctx(state, connection, ctx.id) {
                main_screen(state, connection, &ctx, &note);
            }
        }
        // A building id (map cells, icon pages) opens that building's icon page.
        "buildings" => match building_of(state, ctx, payload) {
            Some(building) => menus::building_from(state, connection, ctx, building, payload),
            None => menus::buildings(state, connection, ctx),
        },
        "buildings_icons" => menus::buildings(state, connection, ctx),
        "map" => map::screen_at(state, connection, ctx, payload),
        "residents" => menus::residents(state, connection, ctx),
        "storage" => menus::storage(state, connection, ctx),
        "orders" => menus::production(state, connection, ctx),
        "army" => army_screen(state, connection, ctx, ""),
        "projects" => menus::projects(state, connection, ctx),
        "project_status" => menus::project(state, connection, ctx),
        "project_status_details" => project_status_screen(state, connection, ctx),
        "settings" => settings_screen(state, connection, ctx, ""),
        "certify" => {
            let form = form_of(state, connection, payload);
            certify_screen(state, connection, ctx, &form, "");
        }
        "mark1" | "mark2" => {
            let index = usize::from(action == "mark2");
            let mut form = form_of(state, connection, payload);
            match player_pos(state, connection) {
                Some(at) => {
                    set_mark(state, connection, index, at);
                    *(if index == 0 {
                        &mut form.from
                    } else {
                        &mut form.to
                    }) = corner_text(at);
                    let note = ctx.f(
                        Key::CornerSet,
                        &[("n", &(index + 1).to_string()), ("at", &corner_text(at))],
                    );
                    certify_screen(state, connection, ctx, &form, &note);
                }
                None => certify_screen(state, connection, ctx, &form, ctx.t(Key::PositionUnknown)),
            }
        }
        "certify_do" => certify(state, connection, ctx, payload),
        "upgrade" => upgrade(state, connection, ctx, payload),
        "project_order" => project_order(state, connection, ctx, payload),
        "project_upgrade" => project_upgrade(state, connection, ctx, payload),
        "project_cancel_pending" => {
            if !pending_order_matches(state, ctx, payload) {
                return main_screen(state, connection, ctx, ctx.t(Key::StateChanged));
            }
            let note = match super::construction::cancel_pending(state, ctx.id) {
                Ok(()) => ctx.t(Key::PendingCancelled).to_owned(),
                Err(error) => error,
            };
            projects_screen(state, connection, ctx, &note);
        }
        "order_start" => order_start(state, connection, ctx),
        "order_pick" => order_pick(state, connection, ctx, payload),
        "order_create" => order_create(state, connection, ctx, payload),
        "order_pause" | "order_resume" | "order_cancel" => {
            order_control(state, connection, ctx, action, payload)
        }
        "army_soldier" => army_soldier(state, connection, ctx, payload),
        "army_recruit" => army_recruit(state, connection, ctx),
        "army_dismiss" => army_dismiss(state, connection, ctx, payload),
        "army_order" => army_order(state, connection, ctx, payload),
        "rename" => rename(state, connection, ctx, payload),
        _ => {
            tracing::debug!(target: "dialog", connection = connection.0, action, "unknown settlement action");
        }
    }
}

// ------------------------------------------------------------------- resolving

fn ctx(state: &TickState, connection: ConnectionId, id: SettlementId) -> Option<Ctx> {
    let record = state.settlements.get(id)?;
    let actor = state.connections.uuid_of(connection)?.to_string();
    Some(Ctx {
        id,
        manage: record.can_manage(&actor, state.plugin_players.is_operator(connection))
            && state
                .connections
                .entity_of(connection)
                .and_then(|entity| state.players.get(&entity))
                .is_some_and(|player| !player.spectator()),
        lang: Lang::of(state.connections.locale_of(connection)),
    })
}

fn player_pos(state: &TickState, connection: ConnectionId) -> Option<[i32; 3]> {
    let entity = state.connections.entity_of(connection)?;
    let slot = state.entities.slot_of(entity)?;
    Some(state.entities.pos(slot).map(|value| value.floor() as i32))
}

/// The nearest settlement whose radius contains the player, else the first the
/// player owns.
fn locate(state: &TickState, connection: ConnectionId) -> Option<SettlementId> {
    let actor = state.connections.uuid_of(connection)?.to_string();
    if let Some(at) = player_pos(state, connection) {
        let inside = state
            .settlements
            .records()
            .filter_map(|record| {
                let distance: i64 = at
                    .iter()
                    .zip(record.anchor)
                    .map(|(a, b)| (i64::from(*a) - i64::from(b)).pow(2))
                    .sum();
                (distance <= i64::from(record.radius).pow(2)).then_some((distance, record.id))
            })
            .min();
        if let Some((_, id)) = inside {
            return Some(id);
        }
    }
    state
        .settlements
        .records()
        .find(|record| record.owner.as_deref() == Some(actor.as_str()))
        .map(|record| record.id)
}

fn say(state: &mut TickState, connection: ConnectionId, message: &str) {
    state.push_player(
        connection,
        PlayerChange::Chat {
            message: message.to_owned(),
        },
    );
}

fn present(state: &mut TickState, connection: ConnectionId, dialog: Dialog) {
    if let Err(error) = dialog::open(state, connection, dialog) {
        tracing::warn!(target: "dialog", %error, "settlement dialog rejected");
    }
}

fn button(label: impl Into<String>, action: &str, id: SettlementId) -> Button {
    Button::custom(label.into(), &format!("krasnokamen:{OWNER}/{action}"))
        .with("s", Tag::String(id.0.to_string()))
}

fn frame(ctx: &Ctx, title: String, note: &str, text: String, buttons: Vec<Button>) -> Dialog {
    let mut dialog = Dialog::multi_action(title, buttons);
    for paragraph in [note, text.as_str()] {
        if !paragraph.is_empty() {
            dialog = dialog.text(paragraph);
        }
    }
    dialog.exit(Button::plain(ctx.t(Key::Close)))
}

fn short(id: BuildingId) -> u64 {
    id.0 & u64::from(u32::MAX)
}

fn corner_text(at: [i32; 3]) -> String {
    format!("{} {} {}", at[0], at[1], at[2])
}

fn kind_label(ctx: &Ctx, kind: BuildingKind) -> String {
    match kind {
        BuildingKind::House => ctx.t(Key::KindHouse).into(),
        BuildingKind::Storage => ctx.t(Key::KindStorage).into(),
        BuildingKind::Mine => ctx.t(Key::KindMine).into(),
        BuildingKind::LumberCamp => ctx.t(Key::KindLumberCamp).into(),
        BuildingKind::Barracks => ctx.t(Key::KindBarracks).into(),
        BuildingKind::Wall => ctx.t(Key::KindWall).into(),
        BuildingKind::Tower => ctx.t(Key::KindTower).into(),
        BuildingKind::Gate => ctx.t(Key::KindGate).into(),
        BuildingKind::Workshop(profession) => ctx.f(
            Key::KindWorkshop,
            &[("profession", &format!("{profession:?}"))],
        ),
    }
}

fn category(ctx: &Ctx, kind: BuildingKind) -> &'static str {
    match kind {
        BuildingKind::House => ctx.t(Key::CategoryHousing),
        BuildingKind::Storage => ctx.t(Key::CategoryStorage),
        BuildingKind::Workshop(_) | BuildingKind::Mine | BuildingKind::LumberCamp => {
            ctx.t(Key::CategoryProduction)
        }
        BuildingKind::Barracks => ctx.t(Key::CategoryMilitary),
        BuildingKind::Wall | BuildingKind::Tower | BuildingKind::Gate => {
            ctx.t(Key::CategoryFortification)
        }
    }
}

fn all_kinds() -> Vec<BuildingKind> {
    let mut kinds = vec![BuildingKind::House, BuildingKind::Storage];
    kinds.extend(
        [
            Profession::Armorer,
            Profession::Butcher,
            Profession::Cartographer,
            Profession::Cleric,
            Profession::Farmer,
            Profession::Fisherman,
            Profession::Fletcher,
            Profession::Leatherworker,
            Profession::Librarian,
            Profession::Mason,
            Profession::Shepherd,
            Profession::Toolsmith,
            Profession::Weaponsmith,
        ]
        .map(BuildingKind::Workshop),
    );
    kinds.extend([
        BuildingKind::Mine,
        BuildingKind::LumberCamp,
        BuildingKind::Barracks,
        BuildingKind::Wall,
        BuildingKind::Tower,
        BuildingKind::Gate,
    ]);
    kinds
}

fn missing_text(error: &CertError) -> String {
    error
        .missing
        .iter()
        .map(|requirement| {
            if requirement.required <= 1 && requirement.found == 0 {
                requirement.description.clone()
            } else {
                format!(
                    "{} {}/{}",
                    requirement.description, requirement.found, requirement.required
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn block_name(state: &TickState, at: [i32; 3]) -> Option<&str> {
    let native = state.world.block_at(at[0], at[1], at[2])?;
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(native))
        .map(|block| block.block.id.as_str())
}

fn busy_workers(state: &TickState, id: SettlementId) -> BTreeSet<Uuid> {
    state
        .settlements
        .get(id)
        .into_iter()
        .flat_map(|record| record.economy.production.jobs.values())
        .filter(|job| job.unfinished())
        .map(|job| job.worker)
        .collect()
}

// ------------------------------------------------------------------------ main

fn main_screen(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, note: &str) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let owner = record
        .owner_name
        .as_deref()
        .or(record.owner.as_deref())
        .unwrap_or(ctx.t(Key::Autonomous));
    let mut body = ctx.f(Key::HubBody, &[("owner", owner)]);
    if !ctx.manage && record.owner.is_some() {
        body.push('\n');
        body.push_str(ctx.t(Key::ViewOnly));
    }
    let title = ctx.f(Key::TitleSettlement, &[("name", &record.name)]);
    let next = if record.owner.is_none() {
        Key::ClaimReview
    } else {
        Key::NextStep
    };
    let buttons = vec![
        button(ctx.t(next), "next", ctx.id),
        button(ctx.t(Key::Settlement), "dashboard", ctx.id),
        button(ctx.t(Key::Army), "army", ctx.id),
        button(ctx.t(Key::Neighbours), "neighbours", ctx.id),
        button(ctx.t(Key::Help), "help", ctx.id),
    ];
    present(state, connection, frame(ctx, title, note, body, buttons));
}

fn claim_screen(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    present(
        state,
        connection,
        Dialog::confirmation(
            ctx.t(Key::ClaimReview),
            button(ctx.t(Key::ClaimReview), "claim", ctx.id),
            button(ctx.t(Key::Back), "main", ctx.id),
        )
        .text(ctx.f(Key::ClaimPolicy, &[("name", &record.name)])),
    );
}

fn help_screen(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    let topics = [
        (Key::GuideClaim, Key::ClaimReview, "claim_review"),
        (Key::CivilHelp, Key::Settlement, "dashboard"),
        (Key::GuideConstruction, Key::Projects, "projects"),
        (Key::GuideChecklist, Key::Checklist, "checklist"),
        (Key::GuideRecruit, Key::Army, "army"),
        (Key::GuideArmour, Key::KindStorage, "storage"),
        (Key::GuideWeapons, Key::Army, "army"),
        (Key::GuideHousing, Key::Buildings, "buildings"),
        (Key::GuideWar, Key::Neighbours, "neighbours"),
        (Key::GuideBattle, Key::Army, "army"),
        (Key::GuideSupply, Key::KindStorage, "storage"),
    ];
    let page = payload
        .string("page")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
        .min(topics.len() - 1);
    let (topic, label, route) = topics[page];
    let mut buttons = vec![button(ctx.t(label), route, ctx.id)];
    for next in [
        page.checked_sub(1),
        (page + 1 < topics.len()).then_some(page + 1),
    ]
    .into_iter()
    .flatten()
    {
        buttons.push(
            button(if next < page { "←" } else { "→" }, "help", ctx.id)
                .with("page", Tag::String(next.to_string())),
        );
    }
    buttons.push(button(ctx.t(Key::Back), "main", ctx.id));
    present(
        state,
        connection,
        Dialog::multi_action(
            format!("{} {}/{}", ctx.t(Key::Help), page + 1, topics.len()),
            buttons,
        )
        .text(ctx.t(topic)),
    );
}

// Confirm the visible order intent, not the simulation epoch: native planning,
// builder borrowing and progress may change the settlement revision meanwhile.
fn pending_order_matches(state: &TickState, ctx: &Ctx, payload: ClickPayload<'_>) -> bool {
    let Some(record) = state.settlements.get(ctx.id) else {
        return false;
    };
    let pending = record.pending_order.as_ref();
    payload.string("pending_blueprint")
        == Some(pending.map_or("", |order| order.blueprint.as_str()))
        && payload
            .string("pending_level")
            .and_then(|value| value.parse::<u8>().ok())
            == Some(pending.map_or(0, |order| order.level))
        && payload
            .string("pending_building")
            .and_then(|value| value.parse::<u64>().ok())
            == Some(
                pending
                    .and_then(|order| order.building)
                    .map_or(0, |id| id.0),
            )
}

fn with_pending_order(button: Button, state: &TickState, ctx: &Ctx) -> Button {
    let pending = state
        .settlements
        .get(ctx.id)
        .and_then(|record| record.pending_order.as_ref());
    button
        .with(
            "pending_blueprint",
            Tag::String(
                pending
                    .map_or("", |order| order.blueprint.as_str())
                    .to_owned(),
            ),
        )
        .with(
            "pending_level",
            Tag::String(pending.map_or(0, |order| order.level).to_string()),
        )
        .with(
            "pending_building",
            Tag::String(
                pending
                    .and_then(|order| order.building)
                    .map_or(0, |id| id.0)
                    .to_string(),
            ),
        )
}

fn with_navigation(mut button: Button, payload: ClickPayload<'_>) -> Button {
    for key in ["from", "bk", "bp", "mx", "mz", "ms"] {
        if let Some(value) = payload.string(key) {
            button = button.with(key, Tag::String(value.to_owned()));
        }
    }
    button
}

fn building_filters(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    let kinds = all_kinds();
    let pages = kinds.len().div_ceil(8);
    let page = payload
        .string("page")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
        .min(pages - 1);
    let mut buttons = vec![button(ctx.t(Key::Buildings), "buildings_filter", ctx.id)];
    buttons.extend(kinds.into_iter().skip(page * 8).take(8).map(|kind| {
        button(kind_label(ctx, kind), "buildings_filter", ctx.id)
            .with("bk", Tag::String(kind.name().to_owned()))
    }));
    for next in [page.checked_sub(1), (page + 1 < pages).then_some(page + 1)]
        .into_iter()
        .flatten()
    {
        buttons.push(
            button(
                if next < page { "←" } else { "→" },
                "building_filters",
                ctx.id,
            )
            .with("page", Tag::String(next.to_string())),
        );
    }
    buttons.push(button(ctx.t(Key::Back), "buildings", ctx.id));
    present(
        state,
        connection,
        Dialog::multi_action(ctx.t(Key::Buildings), buttons),
    );
}

fn project_estimate(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let building = building_of(state, ctx, payload).and_then(|id| record.buildings.get(&id));
    let blueprint = building
        .and_then(|building| {
            super::blueprints::all(&state.registry)
                .iter()
                .find(|blueprint| blueprint.kind == building.kind)
        })
        .or_else(|| {
            payload
                .string("blueprint")
                .and_then(|id| super::blueprints::get(&state.registry, id))
        });
    let level = building
        .and_then(|building| super::construction::owner_upgrade_level(state, ctx.id, building.id))
        .unwrap_or(1);
    let Some(authored) = blueprint.and_then(|blueprint| blueprint.level(level)) else {
        return main_screen(state, connection, ctx, ctx.t(Key::StateChanged));
    };
    let pages = authored.materials.len().div_ceil(6).max(1);
    let page = payload
        .string("page")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
        .min(pages - 1);
    let body = authored
        .materials
        .iter()
        .skip(page * 6)
        .take(6)
        .map(|(id, count)| {
            format!(
                "{} ×{count}",
                state
                    .registry
                    .items
                    .name_of(*id)
                    .map_or("?", Identifier::as_str)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let review = if building.is_some() {
        "project_upgrade_review"
    } else {
        "project_order_review"
    };
    let target = |label: &str, action: &str| {
        let mut button = with_navigation(button(label, action, ctx.id), payload);
        if let Some(building) = building {
            button = button.with("b", Tag::String(building.id.0.to_string()));
        }
        if let Some(blueprint) = blueprint {
            button = button.with("blueprint", Tag::String(blueprint.id.clone()));
        }
        button
    };
    let mut buttons = vec![target(ctx.t(Key::Back), review)];
    for next in [page.checked_sub(1), (page + 1 < pages).then_some(page + 1)]
        .into_iter()
        .flatten()
    {
        buttons.push(
            target(if next < page { "←" } else { "→" }, "project_estimate")
                .with("page", Tag::String(next.to_string())),
        );
    }
    present(
        state,
        connection,
        Dialog::multi_action(
            format!("{} {}/{}", ctx.t(Key::MaterialsEstimate), page + 1, pages),
            buttons,
        )
        .text(body)
        .text(ctx.t(Key::RequestPolicy)),
    );
}

fn certificate_review(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    let Some(id) = building_of(state, ctx, payload) else {
        return main_screen(state, connection, ctx, ctx.t(Key::BuildingGone));
    };
    let record = state.settlements.get(ctx.id).unwrap();
    let building = &record.buildings[&id];
    let next = building.level.saturating_add(1);
    let result = if building.level >= 3 {
        ctx.t(Key::MaxLevel).to_owned()
    } else {
        match buildings::inspect(state, building.kind, next, building.bounds) {
            Ok(_) => ctx.f(Key::ReadyFor, &[("next", &next.to_string())]),
            Err(error) => missing_text(&error),
        }
    };
    let confirm = with_navigation(
        button(
            ctx.f(Key::Certificate, &[("level", &next.to_string())]),
            "upgrade",
            ctx.id,
        ),
        payload,
    )
    .with("b", Tag::String(id.0.to_string()))
    .with("level", Tag::String(building.level.to_string()));
    let back = with_navigation(button(ctx.t(Key::Back), "buildings", ctx.id), payload)
        .with("b", Tag::String(id.0.to_string()));
    let dialog = if ctx.manage && building.level < 3 {
        Dialog::confirmation(ctx.t(Key::Buildings), confirm, back)
    } else {
        Dialog::notice(ctx.t(Key::Buildings), back)
    };
    present(
        state,
        connection,
        dialog.text(ctx.t(Key::CertificatePolicy)).text(result),
    );
}

fn project_review(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    action: &str,
    payload: ClickPayload<'_>,
) {
    let mut confirm;
    let mut body = ctx.t(Key::RequestPolicy).to_owned();
    let title;
    match action {
        "project_upgrade_review" => {
            let Some(id) = building_of(state, ctx, payload) else {
                return main_screen(state, connection, ctx, ctx.t(Key::BuildingGone));
            };
            let Some(level) = super::construction::owner_upgrade_level(state, ctx.id, id) else {
                return main_screen(state, connection, ctx, ctx.t(Key::NoBlueprintUpgrade));
            };
            title = format!(
                "{} #{}",
                ctx.f(Key::ConstructionUpgrade, &[("level", &level.to_string())]),
                short(id)
            );
            confirm = button(&title, "project_upgrade", ctx.id)
                .with("b", Tag::String(id.0.to_string()))
                .with("level", Tag::String(level.to_string()));
        }
        "project_cancel_review" => {
            title = ctx.t(Key::CancelPending).to_owned();
            body = match ctx.lang {
                Lang::En => {
                    "Cancel only the queued order; active construction is not cancelled.".to_owned()
                }
                Lang::Ru => "Отменить только очередь; активная стройка продолжится.".to_owned(),
            };
            confirm = button(&title, "project_cancel_pending", ctx.id);
        }
        _ => {
            let Some(blueprint) = payload
                .string("blueprint")
                .and_then(|name| super::blueprints::get(&state.registry, name))
            else {
                return main_screen(state, connection, ctx, ctx.t(Key::ChooseBlueprint));
            };
            title = ctx.f(
                Key::OrderBlueprint,
                &[("blueprint", blueprint_label(ctx, &blueprint.id))],
            );
            confirm = button(&title, "project_order", ctx.id)
                .with("blueprint", Tag::String(blueprint.id.clone()));
        }
    }
    if let Some(pending) = pending_order_text(state, ctx) {
        body.push('\n');
        body.push_str(match ctx.lang {
            Lang::En => "Replace queued order? ",
            Lang::Ru => "Заменить заказ в очереди? ",
        });
        body.push_str(&pending);
    }
    confirm = with_navigation(with_pending_order(confirm, state, ctx), payload);
    let back = if action == "project_cancel_review" {
        button(ctx.t(Key::Back), "project_status_details", ctx.id)
    } else if let Some(id) = building_of(state, ctx, payload) {
        with_navigation(button(ctx.t(Key::Back), "buildings", ctx.id), payload)
            .with("b", Tag::String(id.0.to_string()))
    } else {
        button(ctx.t(Key::Back), "projects", ctx.id)
    };
    let mut buttons = vec![back];
    if action != "project_cancel_review" {
        let mut estimate = with_navigation(
            button(ctx.t(Key::MaterialsEstimate), "project_estimate", ctx.id),
            payload,
        );
        for key in ["b", "blueprint"] {
            if let Some(value) = payload.string(key) {
                estimate = estimate.with(key, Tag::String(value.to_owned()));
            }
        }
        buttons.push(estimate);
    }
    if ctx.manage {
        buttons.insert(0, confirm);
    }
    present(
        state,
        connection,
        Dialog::multi_action(title, buttons).text(body),
    );
}

// ------------------------------------------------------------------- buildings

fn buildings_screen(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, note: &str) {
    present(
        state,
        connection,
        Dialog::notice(
            ctx.t(Key::Buildings),
            button(ctx.t(Key::Back), "buildings", ctx.id),
        )
        .text(note),
    );
}

fn form_of(state: &TickState, connection: ConnectionId, payload: ClickPayload<'_>) -> Form {
    let marks = state
        .settlement_marks
        .get(&connection)
        .copied()
        .unwrap_or_default();
    let here = player_pos(state, connection);
    let default_corner =
        |mark: Option<[i32; 3]>| mark.or(here).map(corner_text).unwrap_or_default();
    Form {
        kind: payload.string("kind").unwrap_or("house").to_owned(),
        from: payload
            .string("from")
            .map_or_else(|| default_corner(marks[0]), str::to_owned),
        to: payload
            .string("to")
            .map_or_else(|| default_corner(marks[1]), str::to_owned),
        extra: payload.float("extra").unwrap_or(4.0),
    }
}

fn set_mark(state: &mut TickState, connection: ConnectionId, index: usize, at: [i32; 3]) {
    if state.settlement_marks.len() >= MAX_MARKS
        && !state.settlement_marks.contains_key(&connection)
    {
        let connections = &state.connections;
        state
            .settlement_marks
            .retain(|other, _| connections.entity_of(*other).is_some());
    }
    state.settlement_marks.entry(connection).or_default()[index] = Some(at);
}

fn certify_screen(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    form: &Form,
    note: &str,
) {
    let kinds = all_kinds();
    let selected = BuildingKind::parse(&form.kind).unwrap_or(BuildingKind::House);
    let options = kinds
        .iter()
        .map(|kind| {
            (
                kind.name().to_owned(),
                Tag::String(format!(
                    "{}: {}",
                    category(ctx, *kind),
                    kind_label(ctx, *kind)
                )),
            )
        })
        .collect();
    // Requirements are reported on failure (and per building under "needs"), not
    // as a paragraph: that keeps the form inside a small window.
    // Short labels, one button row and no Close button (Escape closes): the
    // form must fit a small window without scrolling.
    let buttons = vec![
        button(ctx.t(Key::Certify), "certify_do", ctx.id)
            .width(100)
            .tooltip(ctx.t(Key::TipCorners)),
        button(ctx.t(Key::Corner1), "mark1", ctx.id)
            .width(100)
            .tooltip(ctx.t(Key::TipCorner1)),
        button(ctx.t(Key::Corner2), "mark2", ctx.id)
            .width(100)
            .tooltip(ctx.t(Key::TipCorner2)),
        button(ctx.t(Key::Back), "buildings", ctx.id).width(100),
    ];
    let mut dialog = Dialog::multi_action(ctx.t(Key::TitleCertify), buttons).columns(4);
    if !note.is_empty() {
        dialog = dialog.text(note);
    }
    let dialog = dialog
        .input(
            Input::single_option("kind", ctx.t(Key::LabelKind), options)
                .initial_option(selected.name()),
        )
        .input(
            Input::text("from", ctx.t(Key::LabelFrom))
                .initial_text(&form.from)
                .max_length(40),
        )
        .input(
            Input::text("to", ctx.t(Key::LabelTo))
                .initial_text(&form.to)
                .max_length(40),
        )
        .input(
            Input::number_range("extra", ctx.t(Key::LabelExtra), 0.0, MAX_EXTRA_HEIGHT)
                .initial_number(form.extra.clamp(0.0, MAX_EXTRA_HEIGHT).round())
                .step(1.0),
        );
    present(state, connection, dialog);
}

fn corner(text: &str) -> Option<[i32; 3]> {
    let mut parts = text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|part| !part.is_empty());
    let at = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    parts.next().is_none().then_some(at)
}

fn certify(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, payload: ClickPayload<'_>) {
    let form = form_of(state, connection, payload);
    let Some(kind) = BuildingKind::parse(&form.kind) else {
        return certify_screen(state, connection, ctx, &form, ctx.t(Key::ChooseKind));
    };
    let (Some(a), Some(b)) = (corner(&form.from), corner(&form.to)) else {
        return certify_screen(state, connection, ctx, &form, ctx.t(Key::BadCorners));
    };
    let extra = form.extra.clamp(0.0, MAX_EXTRA_HEIGHT) as i32;
    let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
    let mut max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
    max[1] = max[1].saturating_add(extra);
    let Ok(bounds) = ScriptWorldBounds::new(min, max) else {
        return certify_screen(state, connection, ctx, &form, ctx.t(Key::InvalidCorners));
    };
    match buildings::certify(state, ctx.id, kind, bounds) {
        Ok(id) => {
            state.settlement_marks.remove(&connection);
            let note = ctx.f(
                Key::Certified,
                &[
                    ("kind", &kind_label(ctx, kind)),
                    ("id", &short(id).to_string()),
                ],
            );
            buildings_screen(state, connection, ctx, &note);
        }
        Err(error) => {
            let note = ctx.f(
                Key::CannotCertify,
                &[
                    ("kind", &kind_label(ctx, kind)),
                    ("missing", &missing_text(&error)),
                ],
            );
            certify_screen(state, connection, ctx, &form, &note);
        }
    }
}

fn building_of(state: &TickState, ctx: &Ctx, payload: ClickPayload<'_>) -> Option<BuildingId> {
    let id = BuildingId(payload.string("b")?.parse().ok()?);
    state
        .settlements
        .get(ctx.id)?
        .buildings
        .contains_key(&id)
        .then_some(id)
}

fn upgrade(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, payload: ClickPayload<'_>) {
    let Some(id) = building_of(state, ctx, payload) else {
        return buildings_screen(state, connection, ctx, ctx.t(Key::BuildingGone));
    };
    if payload
        .string("level")
        .and_then(|value| value.parse::<u8>().ok())
        != state
            .settlements
            .get(ctx.id)
            .and_then(|record| record.buildings.get(&id))
            .map(|building| building.level)
    {
        return main_screen(state, connection, ctx, ctx.t(Key::StateChanged));
    }
    let note = match buildings::upgrade(state, id) {
        Ok(()) => {
            let level = state
                .settlements
                .get(ctx.id)
                .and_then(|record| record.buildings.get(&id))
                .map_or(0, |building| building.level);
            ctx.f(
                Key::Upgraded,
                &[
                    ("id", &short(id).to_string()),
                    ("level", &level.to_string()),
                ],
            )
        }
        Err(error) => ctx.f(
            Key::CannotUpgrade,
            &[
                ("id", &short(id).to_string()),
                ("missing", &missing_text(&error)),
            ],
        ),
    };
    let back = with_navigation(button(ctx.t(Key::Back), "buildings", ctx.id), payload)
        .with("b", Tag::String(id.0.to_string()));
    present(
        state,
        connection,
        Dialog::notice(ctx.t(Key::Buildings), back).text(note),
    );
}

// ---------------------------------------------------------------------- orders

fn job_status(ctx: &Ctx, job: &super::production::Job) -> String {
    if job.cancelled {
        ctx.t(Key::JobCancelled).to_owned()
    } else if job.delivery_pending() {
        ctx.f(
            Key::JobWaiting,
            &[(
                "reason",
                job.paused.as_deref().unwrap_or("awaiting output storage"),
            )],
        )
    } else if job.done >= job.count {
        ctx.t(Key::JobDone).to_owned()
    } else {
        match (job.paused.as_deref(), job.after.as_deref()) {
            (Some("paused"), _) => ctx.t(Key::JobPaused).to_owned(),
            (_, Some(after)) => ctx.f(Key::JobAwaitingStep, &[("job", after)]),
            (Some(reason), None) => ctx.f(Key::JobWaiting, &[("reason", reason)]),
            (None, None) => ctx.t(Key::JobWorking).to_owned(),
        }
    }
}

fn orders_screen(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, note: &str) {
    if note.is_empty() {
        menus::production(state, connection, ctx);
    } else {
        present(
            state,
            connection,
            Dialog::notice(
                ctx.t(Key::ProductionOrders),
                button(ctx.t(Key::Back), "orders", ctx.id),
            )
            .text(note),
        );
    }
}

fn production_detail(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    id: &str,
    note: &str,
) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let Some(job) = record.economy.production.jobs.get(id) else {
        return orders_screen(state, connection, ctx, ctx.t(Key::OrderGone));
    };
    let text = ctx.f(
        Key::OrderRow,
        &[
            ("id", &job.id),
            ("recipe", &job.recipe),
            ("done", &job.done.to_string()),
            ("count", &job.count.to_string()),
            ("worker", super::presentation::villager_name(job.worker)),
            ("status", &job_status(ctx, job)),
        ],
    );
    let mut buttons = vec![button(ctx.t(Key::Back), "orders", ctx.id)];
    if ctx.manage && job.unfinished() {
        let (label, route) = if job.paused.as_deref() == Some("paused") {
            (ctx.f(Key::Resume, &[("id", id)]), "order_resume")
        } else {
            (ctx.f(Key::Pause, &[("id", id)]), "order_pause")
        };
        for (label, action) in [
            (label, route),
            (ctx.f(Key::Cancel, &[("id", id)]), "order_cancel"),
        ] {
            buttons.push(button(label, action, ctx.id).with("j", Tag::String(id.to_owned())));
        }
    }
    let mut dialog = Dialog::multi_action(ctx.t(Key::ProductionOrders), buttons).text(text);
    if !note.is_empty() {
        dialog = dialog.text(note);
    }
    present(state, connection, dialog);
}

/// Crafting stations inside certified workshops, a bounded scan on click only.
fn stations(state: &TickState, id: SettlementId) -> Vec<Station> {
    let wanted: BTreeSet<&str> = state
        .registry
        .item_facts
        .work_recipes()
        .map(|recipe| recipe.station.as_str())
        .collect();
    let mut found = Vec::new();
    let Some(record) = state.settlements.get(id) else {
        return found;
    };
    for building in record
        .buildings
        .values()
        .filter(|building| matches!(building.kind, BuildingKind::Workshop(_)))
        .take(MAX_LISTED)
    {
        let mut here = 0;
        'scan: for y in building.bounds.min[1]..=building.bounds.max[1] {
            for z in building.bounds.min[2]..=building.bounds.max[2] {
                for x in building.bounds.min[0]..=building.bounds.max[0] {
                    let Some(name) = block_name(state, [x, y, z]) else {
                        continue;
                    };
                    if wanted.contains(name) {
                        found.push(Station {
                            building: building.id,
                            kind: building.kind,
                            at: [x, y, z],
                            block: name.to_owned(),
                        });
                        here += 1;
                        if here == MAX_STATIONS {
                            break 'scan;
                        }
                    }
                }
            }
        }
    }
    found
}

fn order_start(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    let found = stations(state, ctx.id);
    match found.as_slice() {
        [] => orders_screen(state, connection, ctx, ctx.t(Key::NoStation)),
        [only] => recipes_screen(state, connection, ctx, only.building, only.at),
        _ => {
            let buttons = found
                .iter()
                .map(|station| {
                    button(
                        ctx.f(
                            Key::StationRow,
                            &[
                                ("id", &short(station.building).to_string()),
                                ("kind", &kind_label(ctx, station.kind)),
                                (
                                    "block",
                                    station.block.rsplit(':').next().unwrap_or(&station.block),
                                ),
                                ("at", &corner_text(station.at)),
                            ],
                        ),
                        "order_pick",
                        ctx.id,
                    )
                    .with("b", Tag::String(station.building.0.to_string()))
                    .with("p", Tag::String(corner_text(station.at)))
                    .width(300)
                })
                .chain([button(ctx.t(Key::Back), "orders", ctx.id)])
                .collect();
            let dialog = frame(
                ctx,
                ctx.t(Key::ChooseStation).to_owned(),
                "",
                ctx.t(Key::WhereWork).to_owned(),
                buttons,
            )
            .columns(1);
            present(state, connection, dialog);
        }
    }
}

fn order_pick(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    let (Some(building), Some(at)) = (
        building_of(state, ctx, payload),
        payload.string("p").and_then(corner),
    ) else {
        return orders_screen(state, connection, ctx, ctx.t(Key::StationGone));
    };
    recipes_screen(state, connection, ctx, building, at);
}

fn recipes_screen(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    building: BuildingId,
    at: [i32; 3],
) {
    menus::recipes(state, connection, ctx, building, at);
}

fn recipe_review(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    if !ctx.manage {
        return orders_screen(state, connection, ctx, ctx.t(Key::ViewOnly));
    }
    let (Some(building), Some(at), Some(recipe)) = (
        building_of(state, ctx, payload),
        payload.string("p").and_then(corner),
        payload
            .string("r")
            .and_then(|id| Identifier::parse(id).ok()),
    ) else {
        return orders_screen(state, connection, ctx, ctx.t(Key::StationGone));
    };
    let Some(recipe) = state
        .registry
        .item_facts
        .work_recipe(&recipe)
        .filter(|recipe| Some(recipe.station.as_str()) == block_name(state, at))
    else {
        return orders_screen(state, connection, ctx, ctx.t(Key::WrongStation));
    };
    let workers = menus::production_workers(state, ctx.id);
    if workers.is_empty() {
        return orders_screen(state, connection, ctx, ctx.t(Key::NoFreeResident));
    }
    let outputs = recipe
        .outputs
        .iter()
        .map(|stack| format!("{} ×{}", stack.item, stack.count))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = recipe
        .inputs
        .iter()
        .map(|input| {
            let mut alternatives = Vec::new();
            if let Some(item) = &input.item {
                alternatives.push(item.to_string());
            }
            if let Some(tag) = &input.tag {
                alternatives.push(format!("#{tag}"));
            }
            for alternative in &input.alternatives {
                alternatives.push(match alternative {
                    mc_data::recipes::work::WorkRecipeAlternative::Item(item) => item.to_string(),
                    mc_data::recipes::work::WorkRecipeAlternative::Tag(tag) => format!("#{tag}"),
                });
            }
            format!(
                "({}) ×{}{}",
                alternatives.join(" | "),
                input.count,
                if input.components.is_some() {
                    ctx.t(Key::ExactComponents)
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let facts = ctx.f(
        Key::RecipeFacts,
        &[
            ("recipe", recipe.id.as_str()),
            ("inputs", &inputs),
            ("outputs", &outputs),
            ("ticks", &recipe.duration_ticks.to_string()),
        ],
    );
    let confirm = button(ctx.t(Key::NewOrder), "order_create", ctx.id)
        .with("b", Tag::String(building.0.to_string()))
        .with("p", Tag::String(corner_text(at)))
        .with("r", Tag::String(recipe.id.as_str().to_owned()));
    present(
        state,
        connection,
        Dialog::multi_action(
            ctx.t(Key::TitleNewOrder),
            vec![
                confirm,
                button(ctx.t(Key::Back), "order_pick", ctx.id)
                    .with("b", Tag::String(building.0.to_string()))
                    .with("p", Tag::String(corner_text(at))),
            ],
        )
        .text(facts)
        .text(ctx.f(Key::PickProduct, &[("at", &corner_text(at))]))
        .input(
            Input::number_range("count", ctx.t(Key::Quantity), 1.0, MAX_ORDER)
                .initial_number(8.0)
                .step(1.0),
        )
        .input(Input::single_option(
            "worker",
            ctx.t(Key::Resident),
            workers,
        ))
        .input(Input::boolean("auto", ctx.t(Key::AutoCraft)).initial_bool(false)),
    );
}

fn failure_text(ctx: &Ctx, failure: Failure) -> String {
    match failure {
        Failure::Busy => ctx.t(Key::FailureBusy).to_owned(),
        Failure::NotFound => ctx.t(Key::FailureNotFound).to_owned(),
        Failure::Forbidden => ctx.t(Key::FailureForbidden).to_owned(),
        Failure::Blocked => ctx.t(Key::FailureBlocked).to_owned(),
        Failure::Unloaded => ctx.t(Key::FailureUnloaded).to_owned(),
        other => ctx.f(Key::FailureOther, &[("reason", &format!("{other:?}"))]),
    }
}

fn order_create(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    let (Some(building), Some(at), Some(recipe), Some(worker)) = (
        building_of(state, ctx, payload),
        payload.string("p").and_then(corner),
        payload
            .string("r")
            .and_then(|name| Identifier::parse(name).ok()),
        payload
            .string("worker")
            .and_then(|text| text.parse::<Uuid>().ok()),
    ) else {
        return orders_screen(state, connection, ctx, ctx.t(Key::OrderIncomplete));
    };
    let count = payload
        .float("count")
        .map_or(1, |value| value.clamp(1.0, MAX_ORDER) as u32);
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let Some(found) = record.buildings.get(&building).cloned() else {
        return orders_screen(state, connection, ctx, ctx.t(Key::WorkshopGone));
    };
    let valid = matches!(found.kind, BuildingKind::Workshop(_))
        && (0..3).all(|i| found.bounds.min[i] <= at[i] && at[i] <= found.bounds.max[i])
        && state
            .registry
            .item_facts
            .work_recipe(&recipe)
            .is_some_and(|work| block_name(state, at) == Some(work.station.as_str()));
    if !valid {
        return orders_screen(state, connection, ctx, ctx.t(Key::WrongStation));
    }
    let Some(stand) = super::production::stand_at(state, at, found.bounds) else {
        return orders_screen(state, connection, ctx, ctx.t(Key::NoStand));
    };
    let production = &record.economy.production;
    let mut sequence = production.sequence.saturating_add(1);
    let job = loop {
        let candidate = format!("ui-{sequence}");
        if !production.jobs.contains_key(&candidate) {
            break candidate;
        }
        sequence += 1;
    };
    let building = ScriptSettlementBuildingRef::new(
        "building".to_owned(),
        building.0.to_string(),
        format!("krasnokamen:{}", found.kind.name()),
    );
    // Checked "craft missing ingredients": plan and admit the whole chain.
    if payload.bool("auto") == Some(true) {
        let order = super::autocraft::Order {
            job: job.clone(),
            worker,
            recipe: recipe.as_str().to_owned(),
            count,
            building,
            station: at,
            stand_at: stand,
        };
        let note = match super::autocraft::order(state, ctx.id, order) {
            Ok(steps) => ctx.f(
                Key::OrderPlanned,
                &[
                    ("job", &job),
                    ("steps", &super::autocraft::describe(&steps)),
                ],
            ),
            Err(super::autocraft::Refusal::Admission(failure)) => failure_text(ctx, failure),
            Err(refusal) => ctx.f(Key::FailureOther, &[("reason", &refusal.text(ctx.lang))]),
        };
        return orders_screen(state, connection, ctx, &note);
    }
    let action = Action::ProductionStart {
        job: job.clone(),
        worker: worker.to_string(),
        recipe: recipe.as_str().to_owned(),
        count,
        building,
        station: at,
        stand_at: stand,
    };
    let note = match economy_customization::execute(state, ctx.id, &action) {
        Ok(()) => ctx.f(
            Key::OrderStarted,
            &[
                ("job", &job),
                ("item", recipe.path()),
                ("count", &count.to_string()),
            ],
        ),
        Err(failure) => failure_text(ctx, failure),
    };
    orders_screen(state, connection, ctx, &note);
}

fn order_control(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    action: &str,
    payload: ClickPayload<'_>,
) {
    let Some(job) = payload.string("j").map(str::to_owned).filter(|job| {
        state
            .settlements
            .get(ctx.id)
            .is_some_and(|record| record.economy.production.jobs.contains_key(job))
    }) else {
        return orders_screen(state, connection, ctx, ctx.t(Key::OrderGone));
    };
    let request = match action {
        "order_pause" => Action::ProductionPause { job: job.clone() },
        "order_resume" => Action::ProductionResume { job: job.clone() },
        _ => Action::ProductionCancel { job: job.clone() },
    };
    let note = match economy_customization::execute(state, ctx.id, &request) {
        Ok(()) => ctx.f(Key::OrderUpdated, &[("job", &job)]),
        Err(failure) => failure_text(ctx, failure),
    };
    production_detail(state, connection, ctx, &job, &note);
}

// ------------------------------------------------------------------------ army

/// First eight hex digits of a soldier UUID: the roster label. Clicks carry the full UUID.
fn soldier_id(uuid: Uuid) -> String {
    uuid.to_string()[..8].to_owned()
}

fn order_word(order: &super::army::Order) -> String {
    use super::army::Order;
    match order {
        Order::Follow(_) => "Follow".to_owned(),
        Order::Guard(at) => format!("Guard {}", corner_text(*at)),
        Order::Patrol => "Patrol".to_owned(),
        Order::Return => "Return".to_owned(),
    }
}

/// Native soldiers. The roster is one short line and one `Soldier` button per
/// soldier, so its button grid stays small however many soldiers there are; the
/// per-soldier orders and dismissal live on [`soldier_screen`]. The all-soldier
/// orders stay here. Follow targets the clicking player; guard posts at their
/// feet. Reads are open; mutations require `manage` (checked before dispatch,
/// re-read here for the live executor pose).
fn army_screen(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, note: &str) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let rows = super::army::rows(state, ctx.id);
    let mut text = if rows.is_empty() {
        ctx.t(Key::NoSoldiers).to_owned()
    } else {
        rows.iter()
            .take(MAX_RESIDENTS)
            .map(|row| {
                format!(
                    "{} — {:.0} HP — {}",
                    soldier_id(row.uuid),
                    row.health,
                    order_word(&row.order)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    if !ctx.manage {
        text.push('\n');
        text.push_str(ctx.t(Key::ViewOnly));
    }
    let mut buttons = Vec::new();
    if ctx.manage {
        buttons.push(button(ctx.t(Key::Recruit), "army_recruit", ctx.id));
    }
    for row in rows.iter().take(MAX_RESIDENTS) {
        buttons.push(
            button(
                ctx.f(Key::SoldierButton, &[("id", &soldier_id(row.uuid))]),
                "army_soldier",
                ctx.id,
            )
            .with("u", Tag::String(row.uuid.to_string())),
        );
    }
    if ctx.manage && !rows.is_empty() {
        for (label, order) in [
            (Key::FollowMeAll, "follow"),
            (Key::GuardHereAll, "guard"),
            (Key::PatrolAll, "patrol"),
            (Key::ReturnAll, "return"),
        ] {
            buttons.push(
                button(ctx.t(label), "army_order", ctx.id)
                    .with("u", Tag::String("all".to_owned()))
                    .with("o", Tag::String(order.to_owned())),
            );
        }
    }
    buttons.push(button(ctx.t(Key::Back), "main", ctx.id));
    let title = ctx.f(Key::TitleArmy, &[("name", &record.name)]);
    present(state, connection, frame(ctx, title, note, text, buttons));
}

/// One soldier: details plus its Follow/Guard/Patrol/Return and Dismiss buttons.
/// A soldier that no longer exists falls back to the roster.
fn soldier_screen(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    uuid: Uuid,
    note: &str,
) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let Some(row) = super::army::rows(state, ctx.id)
        .into_iter()
        .find(|row| row.uuid == uuid)
    else {
        return army_screen(state, connection, ctx, ctx.t(Key::SoldierGone));
    };
    let text = format!(
        "{uuid}\n{:.0} HP — {}\n{}",
        row.health,
        order_word(&row.order),
        row.equipment.join(", ")
    );
    let mut buttons = Vec::new();
    if ctx.manage {
        for (label, order) in [
            (Key::FollowMe, "follow"),
            (Key::GuardHere, "guard"),
            (Key::Patrol, "patrol"),
            (Key::Return, "return"),
        ] {
            buttons.push(
                button(ctx.t(label), "army_order", ctx.id)
                    .with("u", Tag::String(uuid.to_string()))
                    .with("o", Tag::String(order.to_owned())),
            );
        }
        buttons.push(
            button(ctx.t(Key::Dismiss), "army_dismiss", ctx.id)
                .with("u", Tag::String(uuid.to_string())),
        );
    }
    buttons.push(button(ctx.t(Key::Back), "army", ctx.id));
    let title = ctx.f(
        Key::TitleSoldier,
        &[("id", &soldier_id(uuid)), ("name", &record.name)],
    );
    present(state, connection, frame(ctx, title, note, text, buttons));
}

/// Back to the soldier a click targeted, or to the roster for an all-soldier click.
fn army_target(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    target: Option<Uuid>,
    note: &str,
) {
    match target {
        Some(uuid) => soldier_screen(state, connection, ctx, uuid, note),
        None => army_screen(state, connection, ctx, note),
    }
}

fn army_soldier(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    match payload.string("u").and_then(|text| text.parse().ok()) {
        Some(uuid) => soldier_screen(state, connection, ctx, uuid, ""),
        None => army_screen(state, connection, ctx, ctx.t(Key::SoldierGone)),
    }
}

/// A successful recruit lands on the new soldier's page: it has no Recruit
/// button, so a repeated click on the same spot cannot recruit a second soldier.
fn army_recruit(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    match super::army::recruit(state, ctx.id) {
        Ok(uuid) => {
            let note = ctx.f(Key::Recruited, &[("uuid", &soldier_id(uuid))]);
            soldier_screen(state, connection, ctx, uuid, &note);
        }
        Err(message) => army_screen(state, connection, ctx, &message),
    }
}

fn army_dismiss(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    let Some(uuid) = payload.string("u").and_then(|text| text.parse().ok()) else {
        return army_screen(state, connection, ctx, ctx.t(Key::SoldierGone));
    };
    let note = match super::army::dismiss(state, ctx.id, uuid) {
        Ok(()) => ctx.f(Key::Dismissed, &[("uuid", &soldier_id(uuid))]),
        Err(message) => message,
    };
    army_screen(state, connection, ctx, &note);
}

fn army_order(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    let target = payload.string("u").unwrap_or("all");
    let uuid = if target == "all" {
        None
    } else if let Ok(uuid) = target.parse() {
        Some(uuid)
    } else {
        return army_screen(state, connection, ctx, ctx.t(Key::SoldierGone));
    };
    let order = match payload.string("o") {
        Some("follow") => {
            let Some(executor) = state.connections.entity_of(connection) else {
                return army_target(state, connection, ctx, uuid, ctx.t(Key::PositionUnknown));
            };
            let Some(slot) = state.entities.slot_of(executor) else {
                return army_target(state, connection, ctx, uuid, ctx.t(Key::PositionUnknown));
            };
            super::army::Order::Follow(state.entities.row(slot).uuid)
        }
        Some("guard") => match player_pos(state, connection) {
            Some(at) => super::army::Order::Guard(at),
            None => {
                return army_target(state, connection, ctx, uuid, ctx.t(Key::PositionUnknown));
            }
        },
        Some("patrol") => super::army::Order::Patrol,
        Some("return") => super::army::Order::Return,
        _ => return army_target(state, connection, ctx, uuid, ctx.t(Key::ChooseOrder)),
    };
    let note = match super::army::order(state, ctx.id, uuid, order) {
        Ok(count) => ctx.f(Key::Ordered, &[("count", &count.to_string())]),
        Err(message) => message,
    };
    army_target(state, connection, ctx, uuid, &note);
}

// -------------------------------------------------------------------- projects

/// A native request outcome is visible before entering the live project card.
fn projects_screen(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, note: &str) {
    if note.is_empty() {
        menus::projects(state, connection, ctx);
    } else {
        present(
            state,
            connection,
            Dialog::notice(
                ctx.t(Key::Projects),
                button(ctx.t(Key::Status), "project_status", ctx.id),
            )
            .text(note),
        );
    }
}

fn pending_order_text(state: &TickState, ctx: &Ctx) -> Option<String> {
    let order = state.settlements.get(ctx.id)?.pending_order.as_ref()?;
    let raw_reason = super::construction::pending_reason(state, ctx.id).unwrap_or_default();
    let reason = if raw_reason == "busy" {
        ctx.t(Key::PendingBusy)
    } else if raw_reason.starts_with("not enough:") {
        ctx.t(Key::PendingMaterials)
    } else if raw_reason == "waiting for a site" {
        ctx.t(Key::PendingSite)
    } else {
        raw_reason.as_str()
    };
    Some(ctx.f(
        Key::PendingOrder,
        &[
            ("blueprint", blueprint_label(ctx, &order.blueprint)),
            ("level", &order.level.to_string()),
            ("reason", reason),
        ],
    ))
}

fn project_phase(ctx: &Ctx, state: &str) -> &'static str {
    let labels = match state {
        "prepared" => ("Prepared", "Подготовлено"),
        "running" => ("Building", "Строится"),
        "paused" => ("Paused", "Приостановлено"),
        "committed" => ("Complete", "Завершено"),
        "cancelled" => ("Cancelled", "Отменено"),
        _ => ("Native state unavailable", "Состояние недоступно"),
    };
    match ctx.lang {
        Lang::En => labels.0,
        Lang::Ru => labels.1,
    }
}

/// Compact project facts. Exact materials remain on the paged C1 icon card.
fn project_status_screen(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let mut buttons = vec![
        button(menus::storage_label(ctx), "storage", ctx.id),
        button(ctx.t(Key::Back), "project_status", ctx.id),
    ];
    if ctx.manage && record.pending_order.is_some() {
        buttons.push(button(
            ctx.t(Key::CancelPending),
            "project_cancel_review",
            ctx.id,
        ));
    }
    let mut dialog = Dialog::multi_action(ctx.t(Key::Status), buttons);
    if let Some(pending) = pending_order_text(state, ctx) {
        dialog = dialog.text(pending);
    }
    if let Some(project) = &record.project {
        match crate::plugin_structures::settlement_builds::status(state, ctx.id, &project.structure)
        {
            Ok(snapshot) => {
                let total: u64 = snapshot.stages.iter().map(|stage| stage.work_units).sum();
                let done = snapshot
                    .stages
                    .iter()
                    .take(snapshot.current_stage_index as usize)
                    .map(|stage| stage.work_units)
                    .sum::<u64>()
                    .saturating_add(snapshot.completed_work_units);
                dialog = dialog.text(format!(
                    "{} L{} · {}\n{done}/{total}\n{}",
                    blueprint_label(ctx, &project.blueprint),
                    project.level,
                    project_phase(ctx, snapshot.state.as_str()),
                    corner_text(project.origin)
                ));
                if let Some(reason) = &snapshot.pause_reason {
                    dialog = dialog.text(reason.clone());
                }
            }
            Err(_) => {
                dialog = dialog.text(match ctx.lang {
                    Lang::En => "Project records unavailable; cause unknown.",
                    Lang::Ru => "Данные стройки недоступны; причина неизвестна.",
                })
            }
        }
    } else if record.pending_order.is_none() {
        dialog = dialog.text(match ctx.lang {
            Lang::En => "No active project; no queued order.",
            Lang::Ru => "Нет активной стройки и заказа в очереди.",
        });
    }
    present(
        state,
        connection,
        dialog.exit(Button::plain(ctx.t(Key::Close))),
    );
}

/// One missing material of the current project: item, missing count, why it is missing, and the
/// item that would supply it.
type ProjectNeed = (String, u64, String, Option<String>);

/// A quote only for native admitted work, never a second blueprint planner.
fn project_needs(state: &TickState, ctx: &Ctx) -> Result<Vec<ProjectNeed>, String> {
    let record = state
        .settlements
        .get(ctx.id)
        .ok_or(ctx.t(Key::SettlementGone))?;
    if let Some(project) = &record.project {
        return crate::plugin_structures::settlement_builds::missing(
            state,
            ctx.id,
            &project.structure,
        )
        .map(|missing| {
            missing
                .into_iter()
                .map(|(item, count)| {
                    (
                        item,
                        count,
                        ctx.t(Key::SupplyCauseUnavailable).to_owned(),
                        None,
                    )
                })
                .collect()
        })
        .map_err(|error| format!("{error:?}"));
    }
    let Some(pending) = &record.pending_order else {
        return Ok(Vec::new());
    };
    Ok(super::construction::waiting(state, ctx.id)
        .into_iter()
        .filter(|kind| kind.blueprint == pending.blueprint)
        .flat_map(|kind| {
            kind.needs.into_iter().map(|need| {
                let reason = source_reason(ctx.lang, need.no_source).to_owned();
                (
                    need.item,
                    need.total.saturating_sub(need.have),
                    reason,
                    need.source_item,
                )
            })
        })
        .collect())
}

fn material_detail(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, item: &str) {
    let mut dialog = Dialog::multi_action(
        ctx.t(Key::PendingMaterials),
        vec![
            button(menus::storage_label(ctx), "storage", ctx.id),
            button(ctx.t(Key::ProductionOrders), "orders", ctx.id),
            button(ctx.t(Key::Back), "project_status", ctx.id),
        ],
    );
    dialog = match project_needs(state, ctx) {
        Ok(needs) => match needs.into_iter().find(|(name, _, _, _)| name == item) {
            Some((name, missing, reason, source)) => {
                let mut detail = dialog
                    .text(material_text(state, &name, format!(" ×{missing}")))
                    .text(reason);
                if let Some(source) = source {
                    detail = detail
                        .text(ctx.t(Key::SourceMaterialHint))
                        .text(material_text(state, &source, String::new()));
                }
                detail
            }
            None => dialog.text(ctx.t(Key::StateChanged)),
        },
        Err(error) => dialog.text(error),
    };
    present(state, connection, dialog);
}

fn material_text(state: &TickState, item: &str, suffix: String) -> Tag {
    let Ok(id) = Identifier::parse(item) else {
        return Tag::String("Unknown material".into());
    };
    let prefix = if state.registry.blocks.block(&id).is_some() {
        "block"
    } else {
        "item"
    };
    Tag::Compound(vec![
        (
            "translate".into(),
            Tag::String(format!("{prefix}.{}.{}", id.namespace(), id.path())),
        ),
        (
            "extra".into(),
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: vec![Tag::Compound(vec![("text".into(), Tag::String(suffix))])],
            }),
        ),
    ])
}

fn source_reason(lang: Lang, reason: Option<super::construction::SourceReason>) -> &'static str {
    use super::construction::SourceReason;
    match (lang, reason) {
        (Lang::En, None) => "awaiting production",
        (Lang::Ru, None) => "ожидается производство",
        (Lang::En, Some(SourceReason::Mine)) => "needs a mine",
        (Lang::Ru, Some(SourceReason::Mine)) => "нужна шахта",
        (Lang::En, Some(SourceReason::Miner)) => "no miner",
        (Lang::Ru, Some(SourceReason::Miner)) => "нет шахтёра",
        (Lang::En, Some(SourceReason::LumberCamp)) => "needs a lumber camp",
        (Lang::Ru, Some(SourceReason::LumberCamp)) => "нужна лесопилка",
        (Lang::En, Some(SourceReason::Lumberjack)) => "no lumberjack",
        (Lang::Ru, Some(SourceReason::Lumberjack)) => "нет лесоруба",
        (Lang::En, Some(SourceReason::Farm)) => "needs a farm",
        (Lang::Ru, Some(SourceReason::Farm)) => "нужна ферма",
        (Lang::En, Some(SourceReason::Farmer)) => "no farmer",
        (Lang::Ru, Some(SourceReason::Farmer)) => "нет фермера",
        (Lang::En, Some(SourceReason::Shearer)) => "no available resident",
        (Lang::Ru, Some(SourceReason::Shearer)) => "нет свободного жителя",
        (Lang::En, Some(SourceReason::Furnace)) => "needs a settlement-owned furnace",
        (Lang::Ru, Some(SourceReason::Furnace)) => "нужна печь поселения",
        (Lang::En, Some(SourceReason::Shears)) => "needs shears: 2 iron ingots",
        (Lang::Ru, Some(SourceReason::Shears)) => "нужны ножницы: 2 железных слитка",
        (Lang::En, Some(SourceReason::Sheep)) => "no sheep of the required color",
        (Lang::Ru, Some(SourceReason::Sheep)) => "нет овцы нужного цвета",
        (Lang::En, Some(SourceReason::Unknown)) => "no source",
        (Lang::Ru, Some(SourceReason::Unknown)) => "нет источника",
    }
}

fn blueprint_label(ctx: &Ctx, blueprint: &str) -> &'static str {
    match (ctx.lang, blueprint) {
        (Lang::En, "house") => "House",
        (Lang::Ru, "house") => "Дом",
        (Lang::En, "storage") => "Storage",
        (Lang::Ru, "storage") => "Склад",
        (Lang::En, "farm") => "Farm",
        (Lang::Ru, "farm") => "Ферма",
        (Lang::En, "mason") => "Mason's workshop",
        (Lang::Ru, "mason") => "Мастерская каменщика",
        (Lang::En, "shepherd") => "Shepherd's workshop",
        (Lang::Ru, "shepherd") => "Мастерская пастуха",
        (Lang::En, "lumber_camp") => "Lumber camp",
        (Lang::Ru, "lumber_camp") => "Лесопилка",
        (Lang::En, "mine") => "Mine",
        (Lang::Ru, "mine") => "Шахта",
        (Lang::En, "barracks") => "Barracks",
        (Lang::Ru, "barracks") => "Казарма",
        (Lang::En, "wall") => "Wall",
        (Lang::Ru, "wall") => "Стена",
        (Lang::En, "watch_tower") => "Watchtower",
        (Lang::Ru, "watch_tower") => "Сторожевая башня",
        (Lang::En, "road") => "Road",
        (Lang::Ru, "road") => "Дорога",
        (Lang::En, "lamp") => "Street lamp",
        (Lang::Ru, "lamp") => "Фонарь",
        (Lang::En, _) => "Building",
        (Lang::Ru, _) => "Здание",
    }
}

fn project_order(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    let Some(blueprint) = payload.string("blueprint") else {
        return projects_screen(state, connection, ctx, ctx.t(Key::ChooseBlueprint));
    };
    if !pending_order_matches(state, ctx, payload) {
        return main_screen(state, connection, ctx, ctx.t(Key::StateChanged));
    }
    let note = match super::construction::request_build(state, ctx.id, blueprint) {
        Ok(outcome) => match &outcome {
            super::construction::OrderOutcome::Queued { .. } => pending_order_text(state, ctx)
                .unwrap_or_else(|| outcome.message(blueprint_label(ctx, blueprint))),
            super::construction::OrderOutcome::Started => {
                outcome.message(blueprint_label(ctx, blueprint))
            }
        },
        Err(error) => error,
    };
    projects_screen(state, connection, ctx, &note);
}

fn project_upgrade(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    let Some(id) = building_of(state, ctx, payload) else {
        return projects_screen(state, connection, ctx, ctx.t(Key::BuildingGone));
    };
    if !pending_order_matches(state, ctx, payload)
        || payload
            .string("level")
            .and_then(|value| value.parse::<u8>().ok())
            != super::construction::owner_upgrade_level(state, ctx.id, id)
    {
        return main_screen(state, connection, ctx, ctx.t(Key::StateChanged));
    }
    let blueprint = state
        .settlements
        .get(ctx.id)
        .and_then(|record| record.buildings.get(&id))
        .and_then(|building| {
            super::blueprints::all(&state.registry)
                .iter()
                .find(|blueprint| blueprint.kind == building.kind)
        })
        .map(|blueprint| blueprint.id.clone())
        .unwrap_or_default();
    let note = match super::construction::request_upgrade(state, ctx.id, id) {
        Ok(outcome) => {
            let label = match &outcome {
                super::construction::OrderOutcome::Queued { .. } => state
                    .settlements
                    .get(ctx.id)
                    .and_then(|record| record.pending_order.as_ref())
                    .map(|order| format!("{} L{}", blueprint_label(ctx, &blueprint), order.level))
                    .unwrap_or_else(|| blueprint_label(ctx, &blueprint).to_owned()),
                super::construction::OrderOutcome::Started => {
                    blueprint_label(ctx, &blueprint).to_owned()
                }
            };
            match &outcome {
                super::construction::OrderOutcome::Queued { .. } => {
                    pending_order_text(state, ctx).unwrap_or_else(|| outcome.message(&label))
                }
                super::construction::OrderOutcome::Started => outcome.message(&label),
            }
        }
        Err(error) => error,
    };
    projects_screen(state, connection, ctx, &note);
}

fn settings_screen(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, note: &str) {
    let Some(record) = state.settlements.get(ctx.id) else {
        return;
    };
    let text = ctx.f(
        Key::SettingsBody,
        &[
            ("name", &record.name),
            ("radius", &record.radius.to_string()),
            ("revision", &record.revision.to_string()),
        ],
    );
    let mut buttons = Vec::new();
    if ctx.manage {
        buttons.push(button(ctx.t(Key::Rename), "rename", ctx.id));
    }
    buttons.push(button(ctx.t(Key::Back), "main", ctx.id));
    let title = ctx.f(Key::TitleSettings, &[("name", &record.name)]);
    let mut dialog = frame(ctx, title, note, text, buttons);
    if ctx.manage {
        dialog = dialog.input(
            Input::text("name", ctx.t(Key::NewName))
                .initial_text(&record.name)
                .max_length(64),
        );
    }
    present(state, connection, dialog);
}

fn rename(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, payload: ClickPayload<'_>) {
    let Some(name) = payload.string("name") else {
        return settings_screen(state, connection, ctx, ctx.t(Key::EnterName));
    };
    let note = match state.settlements.rename(ctx.id, name.to_owned()) {
        Ok(()) => ctx.f(Key::Renamed, &[("name", name)]),
        Err(message) => message.to_owned(),
    };
    settings_screen(state, connection, ctx, &note);
}

#[path = "ui_map.rs"]
mod map;

#[path = "ui_menus.rs"]
pub(crate) mod menus;

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "army_ui_tests.rs"]
mod army_ui_tests;
