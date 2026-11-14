use super::*;
use crate::boundary::PlayerInput;
use crate::settlements::allocator::tests::{block, test_state, villager};
use crate::tick::run_tick;
use mc_nbt::Tag;
use std::sync::Arc;

const CONN: ConnectionId = ConnectionId(7);
const PLAYER: &str = "00000000-0000-0000-0000-000000000000";

/// `test_state` plus the native work-recipe catalog, with one joined player.
fn joined(permission_level: u8) -> TickState {
    let mut state = test_state();
    let mut registry = (*state.registry).clone();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    state.registry = Arc::new(registry);
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: CONN,
            uuid: uuid::Uuid::nil(),
            name: "Mayor".into(),
            permission_level,
        }],
    );
    state
}

fn town(state: &mut TickState, owner: &str) -> SettlementId {
    state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "oak".into(),
            owner.into(),
            [8, 201, 8],
            48,
        )
        .unwrap()
}

fn field<'a>(tag: &'a Tag, key: &str) -> Option<&'a Tag> {
    match tag {
        Tag::Compound(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, v)| v),
        _ => None,
    }
}

fn list(tag: Option<&Tag>) -> &[Tag] {
    match tag {
        Some(Tag::List(list)) => &list.elements,
        _ => &[],
    }
}

fn text(tag: &Tag) -> String {
    match tag {
        Tag::String(value) => value.clone(),
        Tag::List(values) => values
            .elements
            .iter()
            .map(text)
            .collect::<Vec<_>>()
            .join("\n"),
        Tag::Compound(_) => {
            let mut value = field(tag, "text")
                .or_else(|| field(tag, "translate"))
                .map(text)
                .unwrap_or_default();
            for extra in list(field(tag, "extra")) {
                value.push_str(&text(extra));
            }
            value
        }
        _ => String::new(),
    }
}

/// The received text components and controls of a dialog or chest image.
pub(super) struct Shown {
    pub(super) title: String,
    pub(super) body: String,
    actions: Vec<(String, String)>,
    inputs: Vec<String>,
    pub(super) icons: Vec<(usize, mc_data::ItemStack)>,
}

fn shown(tag: &Tag) -> Shown {
    let mut actions = Vec::new();
    for button in list(field(tag, "actions")).iter().chain(
        ["action", "yes", "no", "exit_action"]
            .iter()
            .filter_map(|key| field(tag, key)),
    ) {
        let Some(id) = field(button, "action")
            .and_then(|action| field(action, "id"))
            .map(text)
        else {
            continue;
        };
        actions.push((text(field(button, "label").unwrap()), id));
    }
    Shown {
        title: text(field(tag, "title").unwrap()),
        body: list(field(tag, "body"))
            .iter()
            .map(|paragraph| text(field(paragraph, "contents").unwrap()))
            .collect::<Vec<_>>()
            .join("\n"),
        actions,
        inputs: list(field(tag, "inputs"))
            .iter()
            .map(|input| text(field(input, "key").unwrap()))
            .collect(),
        icons: Vec::new(),
    }
}

impl Shown {
    pub(super) fn has(&self, action: &str) -> bool {
        self.actions
            .iter()
            .any(|(_, id)| id == &format!("krasnokamen:settlement/{action}"))
    }
    fn label_of(&self, action: &str) -> &str {
        &self
            .actions
            .iter()
            .find(|(_, id)| id == &format!("krasnokamen:settlement/{action}"))
            .unwrap()
            .0
    }
}

fn run(state: &mut TickState, inputs: &[PlayerInput]) -> Vec<PlayerChange> {
    let (_, outbox, _) = run_tick(state, inputs);
    outbox
        .into_iter()
        .filter(|(connection, _)| *connection == CONN)
        .flat_map(|(_, changes)| changes)
        .collect()
}

pub(super) fn icon_text(stack: &mc_data::ItemStack) -> String {
    let mut lines = Vec::new();
    for component in &stack.extended().preserved_components {
        if matches!(
            component.name.as_str(),
            "minecraft:custom_name" | "minecraft:lore"
        ) {
            let mut bytes = component.value.as_slice();
            let tag = mc_nbt::read_network_value(&mut bytes)
                .expect("icon component must be valid network NBT");
            assert!(bytes.is_empty());
            lines.push(text(&tag));
        }
    }
    lines.join("\n")
}

pub(super) fn last_dialog(changes: &[PlayerChange]) -> Shown {
    for change in changes.iter().rev() {
        match change {
            PlayerChange::ShowDialog { dialog } => return shown(dialog),
            PlayerChange::MenuContents { slots, .. } => {
                let title = changes.iter().rev().find_map(|change| match change {
                    PlayerChange::MenuOpened { title, .. } => Some(
                        serde_json::from_str::<serde_json::Value>(title)
                            .ok()
                            .and_then(|value| value.get("text")?.as_str().map(str::to_owned))
                            .unwrap_or_else(|| title.clone()),
                    ),
                    _ => None,
                });
                let icons = slots
                    .iter()
                    .take(54)
                    .enumerate()
                    .filter_map(|(slot, stack)| stack.clone().map(|stack| (slot, stack)))
                    .collect::<Vec<_>>();
                let lines = icons
                    .iter()
                    .map(|(_, stack)| icon_text(stack))
                    .collect::<Vec<_>>();
                return Shown {
                    title: title.unwrap_or_default(),
                    body: lines.join("\n"),
                    actions: Vec::new(),
                    inputs: Vec::new(),
                    icons,
                };
            }
            _ => {}
        }
    }
    panic!("a dialog or native chest image must be sent");
}

fn confirmation_payload(changes: &[PlayerChange], id: &str) -> Tag {
    let dialog = changes
        .iter()
        .find_map(|change| match change {
            PlayerChange::ShowDialog { dialog } => Some(dialog),
            _ => None,
        })
        .expect("actual native review");
    let action = list(field(dialog, "actions"))
        .iter()
        .chain(
            ["action", "yes", "no", "exit_action"]
                .iter()
                .filter_map(|key| field(dialog, key)),
        )
        .find_map(|button| {
            let action = field(button, "action")?;
            field(action, "id")
                .is_some_and(|value| text(value) == id)
                .then_some(action)
        })
        .expect("actual confirmation control");
    field(action, "additions").unwrap().clone()
}

fn chat(changes: &[PlayerChange]) -> Vec<String> {
    changes
        .iter()
        .filter_map(|change| match change {
            PlayerChange::Chat { message } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn command(state: &mut TickState, line: &str) -> Vec<PlayerChange> {
    run(
        state,
        &[PlayerInput::Command {
            connection: CONN,
            command: line.into(),
            permission_level: 0,
        }],
    )
}

fn click(
    state: &mut TickState,
    id: SettlementId,
    action: &str,
    extra: &[(&str, Tag)],
) -> Vec<PlayerChange> {
    let mut entries = vec![("s".to_owned(), Tag::String(id.0.to_string()))];
    entries.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
    if matches!(
        action,
        "project_order" | "project_upgrade" | "project_cancel_pending"
    ) && let Some(record) = state.settlements.get(id)
    {
        let pending = record.pending_order.as_ref();
        for (key, value) in [
            (
                "pending_blueprint",
                pending
                    .map_or("", |order| order.blueprint.as_str())
                    .to_owned(),
            ),
            (
                "pending_level",
                pending.map_or(0, |order| order.level).to_string(),
            ),
            (
                "pending_building",
                pending
                    .and_then(|order| order.building)
                    .map_or(0, |id| id.0)
                    .to_string(),
            ),
        ] {
            if !extra.iter().any(|(given, _)| *given == key) {
                entries.push((key.to_owned(), Tag::String(value)));
            }
        }
    }
    if matches!(action, "upgrade" | "project_upgrade")
        && !extra.iter().any(|(key, _)| *key == "level")
    {
        let building = extra
            .iter()
            .find(|(key, _)| *key == "b")
            .and_then(|(_, value)| text(value).parse::<u64>().ok())
            .map(BuildingId);
        let level = building.and_then(|building| {
            if action == "project_upgrade" {
                super::super::construction::owner_upgrade_level(state, id, building)
            } else {
                state
                    .settlements
                    .get(id)
                    .and_then(|record| record.buildings.get(&building))
                    .map(|building| building.level)
            }
        });
        if let Some(level) = level {
            entries.push(("level".to_owned(), Tag::String(level.to_string())));
        }
    }
    run(
        state,
        &[PlayerInput::DialogClick {
            connection: CONN,
            id: format!("krasnokamen:settlement/{action}"),
            payload: Some(Tag::Compound(entries)),
        }],
    )
}

pub(super) fn press(state: &mut TickState, slot: i16) -> Vec<PlayerChange> {
    let entity = state.connections.entity_of(CONN).unwrap();
    let menu = state.players[&entity]
        .open
        .clone()
        .expect("native menu open");
    run(
        state,
        &[PlayerInput::ContainerClick {
            connection: CONN,
            window: menu.window,
            state_id: menu.state_id,
            slot,
            button: 0,
            mode: crate::boundary::ClickMode::Pickup,
        }],
    )
}

fn string(value: &str) -> Tag {
    Tag::String(value.into())
}

fn station(state: &mut TickState, at: [i32; 3], name: &str) {
    let id = block(state, name, &[]);
    state.world.set_block(at[0], at[1], at[2], id);
    state.world.fold_overlay();
}

fn certify_form(kind: &str) -> Vec<(&'static str, Tag)> {
    vec![
        ("kind", string(kind)),
        ("from", string("3 200 3")),
        ("to", string("7 200 7")),
        ("extra", Tag::Float(4.0)),
    ]
}

fn use_bell(state: &mut TickState, at: [i32; 3]) -> Vec<PlayerChange> {
    run(
        state,
        &[PlayerInput::UseItemOn {
            connection: CONN,
            hand: crate::boundary::Hand::Main,
            position: crate::boundary::BlockPos {
                x: at[0],
                y: at[1],
                z: at[2],
            },
            face: 2,
            cursor: [0.5; 3],
            sequence: 1,
        }],
    )
}

fn assert_bell_rings(changes: &[PlayerChange], at: [i32; 3]) {
    assert!(
        changes.iter().any(|change| matches!(
            change,
            PlayerChange::BlockEvent { position, action: 1, parameter: 2, .. }
                if [position.x, position.y, position.z] == at
        )),
        "bell animation must still be queued"
    );
    assert!(
        changes.iter().any(|change| matches!(
            change,
            PlayerChange::BellRung { position }
                if [position.x, position.y, position.z] == at
        )),
        "bell sound must still be queued"
    );
}

#[test]
fn anchor_bell_opens_its_settlement_for_a_non_owner_and_rings() {
    let mut state = joined(0);
    let id = town(&mut state, "someone-else");
    // Another settlement is closer to the player: select by the clicked anchor,
    // not the command's nearest-settlement lookup.
    state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "closer".into(),
            PLAYER.into(),
            [8, 202, 8],
            48,
        )
        .unwrap();
    station(&mut state, [8, 201, 8], "minecraft:bell");
    let changes = use_bell(&mut state, [8, 201, 8]);
    let dialog = last_dialog(&changes);
    assert_eq!(dialog.title, "Settlement oak");
    assert!(dialog.body.contains("View only"));
    assert_bell_rings(&changes, [8, 201, 8]);
    let denied = last_dialog(&click(
        &mut state,
        id,
        "rename",
        &[("name", string("stolen"))],
    ));
    assert!(denied.body.contains("Only the owner or an operator"));
    assert_eq!(state.settlements.get(id).unwrap().name, "oak");
}

#[test]
fn unrelated_bell_inside_settlement_only_rings() {
    let mut state = joined(0);
    town(&mut state, PLAYER);
    station(&mut state, [9, 201, 8], "minecraft:bell");
    let changes = use_bell(&mut state, [9, 201, 8]);
    assert_bell_rings(&changes, [9, 201, 8]);
    assert!(
        !changes
            .iter()
            .any(|change| matches!(change, PlayerChange::ShowDialog { .. }))
    );
}

#[test]
fn settlement_anchor_without_a_bell_does_not_open_menu() {
    let mut state = joined(0);
    town(&mut state, PLAYER);
    station(&mut state, [8, 201, 8], "minecraft:stone");
    let changes = use_bell(&mut state, [8, 201, 8]);
    assert!(
        !changes
            .iter()
            .any(|change| matches!(change, PlayerChange::ShowDialog { .. }))
    );
}

#[test]
fn settlement_command_opens_a_compact_hub_and_native_building_page() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    let changes = command(&mut state, "settlement");
    let dialog = last_dialog(&changes);
    assert_eq!(dialog.title, "Settlement oak");
    assert!(dialog.body.contains(PLAYER));
    assert_eq!(dialog.actions.len(), 5);
    for action in ["next", "dashboard", "army", "neighbours", "help"] {
        assert!(dialog.has(action), "{action}");
    }
    // Pages go straight to a screen.
    let page = last_dialog(&command(&mut state, "settlement menu buildings"));
    assert!(page.title.starts_with("Buildings"));
    assert!(page.body.contains("Certify"));
    assert!(page.icons.iter().any(|(slot, _)| *slot == 52));
    assert_eq!(id, SettlementId(1));
}

#[test]
fn native_storage_counts_a_double_chest_once() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    let (left, right) = ([1, 201, 1], [2, 201, 1]);
    let stack = |state: &TickState, name: &str, count: i32| {
        let item = state
            .registry
            .items
            .id_of(&Identifier::parse(name).unwrap())
            .unwrap();
        mc_data::ItemStack::new(item, count)
    };
    let halves = [
        (left, "left", stack(&state, "minecraft:wheat", 3)),
        (right, "right", stack(&state, "minecraft:hay_block", 1)),
    ];
    for (at, kind, stored) in halves {
        let chest = block(
            &state,
            "minecraft:chest",
            &[("facing", "north"), ("type", kind)],
        );
        state.world.set_block(at[0], at[1], at[2], chest);
        let mut slots = vec![mc_data::ItemStack::EMPTY; 27];
        slots[0] = stored;
        state.block_entities.insert(
            (at[0], at[1], at[2]),
            crate::tick::BlockEntityState {
                slots,
                furnace: None,
            },
        );
    }
    state.world.fold_overlay();
    // Both halves are storage points: the founding chest and a storage hut's.
    let record = state.settlements.get_mut(id).unwrap();
    record.bootstrap_storage = Some(left);
    record.building_sequence += 1;
    let hut = BuildingId((id.0 << 32) | record.building_sequence);
    record.buildings.insert(
        hut,
        crate::settlements::buildings::Building {
            id: hut,
            settlement: id,
            kind: BuildingKind::Storage,
            level: 1,
            bounds: mc_script::ScriptWorldBounds::new(right, right).unwrap(),
            work_points: vec![right],
            revision: 1,
            natural_zone: false,
        },
    );
    let page = last_dialog(&click(&mut state, id, "storage", &[]));
    assert!(
        page.body.contains("1 containers, 2 kinds, 4 items"),
        "{}",
        page.body
    );
    for (name, expected) in [("minecraft:wheat", 3), ("minecraft:hay_block", 1)] {
        let item = state
            .registry
            .items
            .id_of(&Identifier::parse(name).unwrap())
            .unwrap();
        let stock: Vec<_> = page
            .icons
            .iter()
            .filter(|(slot, stack)| *slot < 45 && stack.item_id == item)
            .collect();
        assert_eq!(stock.len(), 1);
        assert_eq!(stock[0].1.count, expected);
    }
}

#[test]
fn command_without_any_settlement_explains_how_to_found_one() {
    let mut state = joined(0);
    let changes = command(&mut state, "settlement");
    assert!(chat(&changes)[0].contains("/settlement create"));
    assert!(
        !changes
            .iter()
            .any(|change| matches!(change, PlayerChange::ShowDialog { .. }))
    );
}

#[test]
fn non_owner_sees_the_screens_read_only_and_every_mutating_click_is_refused() {
    let mut state = joined(0);
    let id = town(&mut state, "someone-else");
    let main = last_dialog(&command(&mut state, "settlement"));
    assert!(main.body.contains("View only"));
    let buildings = last_dialog(&click(&mut state, id, "buildings", &[]));
    assert!(!buildings.icons.iter().any(|(slot, _)| *slot == 52));
    let settings = last_dialog(&click(&mut state, id, "settings", &[]));
    assert!(!settings.has("rename") && settings.inputs.is_empty());

    station(&mut state, [4, 201, 4], "minecraft:composter");
    for (action, extra) in [
        ("certify_do", certify_form("farmer")),
        ("certify", vec![]),
        ("order_start", vec![]),
        ("project_order", vec![("blueprint", string("house"))]),
        ("project_upgrade", vec![("b", string("1"))]),
        ("project_cancel_pending", vec![]),
        ("rename", vec![("name", string("hijacked"))]),
        ("upgrade", vec![("b", string("1"))]),
    ] {
        let refused = last_dialog(&click(&mut state, id, action, &extra));
        assert!(
            refused.body.contains("Only the owner or an operator"),
            "{action}"
        );
    }
    let record = state.settlements.get(id).unwrap();
    assert!(record.buildings.is_empty());
    assert!(record.project.is_none());
    assert_eq!(record.name, "oak");
}

#[test]
fn operator_manages_a_settlement_it_does_not_own() {
    let mut state = joined(2);
    let id = town(&mut state, "someone-else");
    let buildings = last_dialog(&click(&mut state, id, "buildings", &[]));
    assert!(buildings.icons.iter().any(|(slot, _)| *slot == 52));
    let renamed = last_dialog(&click(&mut state, id, "rename", &[("name", string("elm"))]));
    assert!(renamed.body.contains("Renamed to elm."));
    assert_eq!(state.settlements.get(id).unwrap().name, "elm");
}

#[test]
fn stale_or_foreign_ids_never_panic_or_leak() {
    let mut state = joined(0);
    town(&mut state, PLAYER);
    let changes = click(&mut state, SettlementId(99), "buildings", &[]);
    assert_eq!(chat(&changes), ["That settlement no longer exists."]);
    // Missing/garbled settlement entry and unknown actions are ignored.
    let changes = run(
        &mut state,
        &[
            PlayerInput::DialogClick {
                connection: CONN,
                id: "krasnokamen:settlement/buildings".into(),
                payload: Some(Tag::Compound(vec![("s".into(), string("nope"))])),
            },
            PlayerInput::DialogClick {
                connection: CONN,
                id: "krasnokamen:settlement/ghost".into(),
                payload: None,
            },
        ],
    );
    assert!(chat(&changes).is_empty());
    let changes = click(&mut state, SettlementId(1), "ghost", &[]);
    assert!(changes.is_empty());
}

#[test]
fn certify_reports_missing_requirements_then_certifies_and_lists_with_upgrade_needs() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    // Nothing built: the form comes back with the reasons and the typed values.
    let failed = last_dialog(&click(
        &mut state,
        id,
        "certify_do",
        &certify_form("farmer"),
    ));
    assert_eq!(failed.title, "Certify building");
    assert!(
        failed
            .body
            .contains("Cannot certify Farmer workshop. Missing:")
    );
    assert_eq!(failed.inputs, ["kind", "from", "to", "extra"]);
    assert!(state.settlements.get(id).unwrap().buildings.is_empty());

    station(&mut state, [4, 201, 4], "minecraft:composter");
    let done = last_dialog(&click(
        &mut state,
        id,
        "certify_do",
        &certify_form("farmer"),
    ));
    assert!(done.title.starts_with("Buildings"));
    assert!(
        done.body
            .contains("Certified Farmer workshop #1 (level 1).")
    );
    last_dialog(&click(&mut state, id, "buildings", &[]));
    let card = last_dialog(&press(&mut state, 0));
    assert!(card.body.contains("Workers: 0/1"), "{}", card.body);
    assert!(card.body.contains("Level 2 needs:"), "{}", card.body);
    assert!(card.body.contains("workstations 1/2"), "{}", card.body);
    let changes = press(&mut state, 6);
    let cached = confirmation_payload(&changes, "krasnokamen:settlement/upgrade");
    let review = last_dialog(&changes);
    assert!(review.has("upgrade"));
    assert_eq!(state.settlements.get(id).unwrap().buildings.len(), 1);

    // Upgrade refused while the second station is missing, then granted.
    state.settlements.get_mut(id).unwrap().revision += 1;
    let refused = last_dialog(&run(
        &mut state,
        &[PlayerInput::DialogClick {
            connection: CONN,
            id: "krasnokamen:settlement/upgrade".into(),
            payload: Some(cached.clone()),
        }],
    ));
    assert!(
        refused
            .body
            .contains("Cannot upgrade #1. Missing: workstations 1/2.")
    );
    station(&mut state, [6, 201, 4], "minecraft:composter");
    state.settlements.get_mut(id).unwrap().revision += 1;
    let upgraded = last_dialog(&run(
        &mut state,
        &[PlayerInput::DialogClick {
            connection: CONN,
            id: "krasnokamen:settlement/upgrade".into(),
            payload: Some(cached.clone()),
        }],
    ));
    assert!(upgraded.body.contains("Building #1 upgraded to level 2."));
    let stale = last_dialog(&run(
        &mut state,
        &[PlayerInput::DialogClick {
            connection: CONN,
            id: "krasnokamen:settlement/upgrade".into(),
            payload: Some(cached),
        }],
    ));
    assert!(stale.body.contains("changed"), "{}", stale.body);
    let card = last_dialog(&click(
        &mut state,
        id,
        "buildings",
        &[("b", string(&(id.0 << 32 | 1).to_string()))],
    ));
    assert!(card.title.contains("L2"), "{}", card.title);
    assert!(card.body.contains("Farmer workshop"), "{}", card.body);
    // A building of another settlement id is rejected, not trusted.
    let forged = last_dialog(&click(
        &mut state,
        id,
        "upgrade",
        &[("b", string(&(5u64 << 32 | 1).to_string()))],
    ));
    assert!(forged.body.contains("no longer exists"));
}

#[test]
fn marking_corners_prefills_the_certify_form_and_bad_input_is_explained() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    let first = last_dialog(&click(&mut state, id, "mark1", &certify_form("house")));
    let at = player_pos(&state, CONN).unwrap();
    assert!(
        first
            .body
            .contains(&format!("Corner 1 = {}", corner_text(at)))
    );
    assert_eq!(state.settlement_marks[&CONN][0], Some(at));
    // Reopening from the buildings screen uses the stored mark as corner 1.
    click(&mut state, id, "buildings", &[]);
    let reopened = last_dialog(&click(&mut state, id, "certify", &[]));
    assert_eq!(reopened.inputs, ["kind", "from", "to", "extra"]);
    let bad = last_dialog(&click(
        &mut state,
        id,
        "certify_do",
        &[
            ("kind", string("house")),
            ("from", string("a b c")),
            ("to", string("1 2 3")),
        ],
    ));
    assert!(bad.body.contains("three whole numbers"));
    let unknown = last_dialog(&click(
        &mut state,
        id,
        "certify_do",
        &[
            ("kind", string("castle")),
            ("from", string("1 2 3")),
            ("to", string("1 2 3")),
        ],
    ));
    assert!(unknown.body.contains("Choose a building type"));
}

fn bound_resident(state: &mut TickState, id: SettlementId) -> uuid::Uuid {
    let slot = villager(state, [5.5, 201.0, 7.5]);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(state, id, entity).unwrap();
    state.entities.row(slot).uuid
}

#[test]
fn census_idle_filter_excludes_nitwits_and_recovery_keeps_the_workers_profession() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    let farmer = bound_resident(&mut state, id);
    let idle = bound_resident(&mut state, id);
    let nitwit = bound_resident(&mut state, id);
    for (uuid, profession) in [
        (farmer, Profession::Farmer),
        (idle, Profession::None),
        (nitwit, Profession::Nitwit),
    ] {
        let slot = state.entities.slot_of_uuid(uuid).unwrap();
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .data
            .profession = profession;
    }
    click(&mut state, id, "residents", &[]);
    press(&mut state, 6); // working, then eligible idle
    let idle_page = last_dialog(&press(&mut state, 6));
    assert_eq!(
        idle_page
            .icons
            .iter()
            .filter(|(slot, _)| (9..45).contains(slot))
            .count(),
        2
    );
    assert!(!idle_page.body.contains("Nitwit"));
    // Certifying a matching workshop and claiming its work point recovers the
    // worker; the profession is kept throughout.
    station(&mut state, [4, 201, 4], "minecraft:composter");
    click(&mut state, id, "certify_do", &certify_form("farmer"));
    let dimension = state.dimension.name().to_owned();
    let slot = state.entities.slot_of_uuid(farmer).unwrap();
    let villager = state.entities.row_mut(slot).villager.as_mut().unwrap();
    villager.job_site = Some(crate::boundary::BlockPos { x: 4, y: 201, z: 4 });
    villager.job_site_dimension = dimension;
    click(&mut state, id, "residents", &[]);
    let working = last_dialog(&press(&mut state, 6));
    assert_eq!(
        working
            .icons
            .iter()
            .filter(|(slot, _)| (9..45).contains(slot))
            .count(),
        1
    );
    assert!(
        working
            .body
            .contains(super::super::presentation::villager_name(farmer))
    );
    let slot = state.entities.slot_of_uuid(farmer).unwrap();
    let villager = state.entities.row(slot).villager.as_ref().unwrap();
    assert_eq!(villager.data.profession, Profession::Farmer);
}

#[test]
fn residents_screen_lists_jobs_and_free_slots() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    station(&mut state, [4, 201, 4], "minecraft:composter");
    click(&mut state, id, "certify_do", &certify_form("farmer"));
    let uuid = bound_resident(&mut state, id);
    let screen = last_dialog(&click(&mut state, id, "residents", &[]));
    let name = super::super::presentation::villager_name(uuid);
    assert!(screen.body.contains(name));
    assert!(screen.body.contains("No workplace"));
    assert_eq!(
        screen
            .icons
            .iter()
            .filter(|(slot, _)| (9..45).contains(slot))
            .count(),
        1
    );
    let building = BuildingId(id.0 << 32 | 1);
    let workshop = last_dialog(&click(
        &mut state,
        id,
        "buildings",
        &[("b", string(&building.0.to_string()))],
    ));
    assert!(workshop.body.contains("Workers: 0/1"), "{}", workshop.body);
    assert!(workshop.body.contains("Free work place"));
}

#[test]
fn production_order_flow_creates_pauses_and_cancels_a_real_job() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    station(&mut state, [4, 201, 4], "minecraft:composter");
    station(&mut state, [5, 201, 5], "minecraft:crafting_table");
    click(&mut state, id, "certify_do", &certify_form("farmer"));
    let building = BuildingId(id.0 << 32 | 1);

    // No resident yet: the flow explains it instead of opening an empty form.
    click(&mut state, id, "order_start", &[]);
    let none = last_dialog(&press(&mut state, 0));
    assert!(none.body.contains("No free resident"));

    let worker = bound_resident(&mut state, id);
    let catalogue = last_dialog(&click(&mut state, id, "order_start", &[]));
    assert!(catalogue.icons.iter().any(|(slot, _)| *slot < 45));
    let form = last_dialog(&press(&mut state, 0));
    assert_eq!(form.title, "New production order");
    assert_eq!(form.inputs, ["count", "worker", "auto"]);
    assert!(form.has("order_create"));

    let recipe = state
        .registry
        .item_facts
        .work_recipes()
        .find(|recipe| recipe.station.as_str() == "minecraft:crafting_table")
        .unwrap()
        .id
        .as_str()
        .to_owned();
    assert!(form.body.contains(&recipe));
    assert!(form.body.contains("Per craft"));
    assert!(form.body.contains("5 201 5"));
    let create = |recipe: &str, at: &str| {
        vec![
            ("b", string(&building.0.to_string())),
            ("p", string(at)),
            ("r", string(recipe)),
            ("worker", string(&worker.to_string())),
            ("count", Tag::Float(3.0)),
        ]
    };
    // A station position that is not a crafting table is refused.
    let wrong = last_dialog(&click(
        &mut state,
        id,
        "order_create",
        &create(&recipe, "4 201 4"),
    ));
    assert!(wrong.body.contains("needs a different workstation"));
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .economy
            .production
            .jobs
            .is_empty()
    );

    let ok = last_dialog(&click(
        &mut state,
        id,
        "order_create",
        &create(&recipe, "5 201 5"),
    ));
    assert!(ok.body.contains("Order ui-1 started"), "{}", ok.body);
    let job = state.settlements.get(id).unwrap().economy.production.jobs["ui-1"].clone();
    assert_eq!(
        (job.worker, job.count, job.station),
        (worker, 3, [5, 201, 5])
    );
    assert_eq!(job.building.kind, "building");
    assert_eq!(job.building.id, building.0.to_string());
    assert!((0..3).all(|i| (job.stand_at[i] - job.station[i]).abs() <= 1));
    assert_ne!(job.stand_at, job.station);

    // The same worker cannot take a second order.
    let busy = last_dialog(&click(
        &mut state,
        id,
        "order_create",
        &create(&recipe, "5 201 5"),
    ));
    assert!(busy.body.contains("already has an active order"));
    click(&mut state, id, "order_start", &[]);
    let none = last_dialog(&press(&mut state, 0));
    assert!(none.body.contains("No free resident"));

    let catalogue = last_dialog(&click(&mut state, id, "orders", &[]));
    assert!(catalogue.body.contains("ui-1"));
    let listing = last_dialog(&press(&mut state, 0));
    assert!(listing.body.contains("ui-1:"));
    assert!(listing.has("order_pause") && listing.has("order_cancel"));
    assert_eq!(listing.label_of("order_pause"), "Pause ui-1");

    let paused = last_dialog(&click(
        &mut state,
        id,
        "order_pause",
        &[("j", string("ui-1"))],
    ));
    assert!(paused.body.contains("paused"));
    assert!(paused.has("order_resume"));
    let resumed = last_dialog(&click(
        &mut state,
        id,
        "order_resume",
        &[("j", string("ui-1"))],
    ));
    assert!(resumed.has("order_pause"));
    let cancelled = last_dialog(&click(
        &mut state,
        id,
        "order_cancel",
        &[("j", string("ui-1"))],
    ));
    assert!(cancelled.body.contains("cancelled"));
    assert!(state.settlements.get(id).unwrap().economy.production.jobs["ui-1"].cancelled);
    let gone = last_dialog(&click(
        &mut state,
        id,
        "order_cancel",
        &[("j", string("nope"))],
    ));
    assert!(gone.body.contains("no longer exists"));
}

#[test]
fn production_autocraft_order_plans_missing_ingredients_or_says_why_not() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    station(&mut state, [4, 201, 4], "minecraft:composter");
    station(&mut state, [5, 201, 5], "minecraft:crafting_table");
    click(&mut state, id, "certify_do", &certify_form("farmer"));
    let building = BuildingId(id.0 << 32 | 1);
    let worker = bound_resident(&mut state, id);
    let chest = [10, 201, 12];
    station(&mut state, chest, "minecraft:chest");
    let hay = state
        .registry
        .items
        .id_of(&Identifier::parse("minecraft:hay_block").unwrap())
        .unwrap();
    let mut slots = vec![mc_data::ItemStack::EMPTY; 27];
    slots[0] = mc_data::ItemStack::new(hay, 1);
    state.block_entities.insert(
        (chest[0], chest[1], chest[2]),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
    state.settlements.get_mut(id).unwrap().bootstrap_storage = Some(chest);
    let create = |recipe: &str| {
        vec![
            ("b", string(&building.0.to_string())),
            ("p", string("5 201 5")),
            ("r", string(recipe)),
            ("worker", string(&worker.to_string())),
            ("count", Tag::Float(3.0)),
            ("auto", Tag::Byte(1)),
        ]
    };
    let jobs = |state: &TickState| {
        state
            .settlements
            .get(id)
            .unwrap()
            .economy
            .production
            .jobs
            .len()
    };

    let missing = last_dialog(&click(
        &mut state,
        id,
        "order_create",
        &create("krasnokamen-settlements:sugar"),
    ));
    assert!(
        missing.body.contains(
            "Refused: missing 3 minecraft:sugar_cane: not in storage and no recipe makes it."
        ),
        "{}",
        missing.body
    );
    let cycle = last_dialog(&click(
        &mut state,
        id,
        "order_create",
        &create("krasnokamen-settlements:hay_block"),
    ));
    assert!(
        cycle
            .body
            .contains("its recipes form a cycle (hay_block -> open_hay -> hay_block)"),
        "{}",
        cycle.body
    );
    assert_eq!(jobs(&state), 0);

    let planned = last_dialog(&click(
        &mut state,
        id,
        "order_create",
        &create("krasnokamen-settlements:bread"),
    ));
    assert!(
        planned
            .body
            .contains("Order ui-1 planned: open_hay x1 -> bread x3."),
        "{}",
        planned.body
    );
    assert_eq!(jobs(&state), 2);
    let listing = last_dialog(&click(&mut state, id, "orders", &[]));
    assert!(
        listing.body.contains("waiting for ui-1-1"),
        "{}",
        listing.body
    );
}

#[test]
fn production_without_a_crafting_table_says_what_to_place() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    station(&mut state, [4, 201, 4], "minecraft:composter");
    click(&mut state, id, "certify_do", &certify_form("farmer"));
    let note = last_dialog(&click(&mut state, id, "order_start", &[]));
    assert!(
        note.body
            .contains("No crafting table inside a certified workshop")
    );
}

#[test]
fn rename_validates_and_updates_the_record() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    let bad = last_dialog(&click(
        &mut state,
        id,
        "rename",
        &[("name", string("bad name!"))],
    ));
    assert!(bad.body.contains("invalid settlement name"));
    assert_eq!(state.settlements.get(id).unwrap().name, "oak");
    let good = last_dialog(&click(
        &mut state,
        id,
        "rename",
        &[("name", string("birch"))],
    ));
    assert!(good.body.contains("Renamed to birch."));
    assert_eq!(state.settlements.get(id).unwrap().name, "birch");
    assert_eq!(good.inputs, ["name"]);
}

#[test]
fn autonomous_main_page_keeps_claim_first_and_fits_a_small_window() {
    let mut state = joined(0);
    let at = player_pos(&state, CONN).unwrap();
    let id = state
        .settlements
        .adopt_village(
            &mut state.settlement_sequence,
            "minecraft:overworld:compact-village",
            at,
        )
        .unwrap();
    for index in 0..8 {
        let slot = villager(
            &mut state,
            [at[0] as f64 + index as f64, at[1] as f64, at[2] as f64],
        );
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, id, entity).unwrap();
    }
    let page = last_dialog(&command(&mut state, "settlement"));
    assert_eq!(
        page.actions.first().map(|(_, action)| action.as_str()),
        Some("krasnokamen:settlement/next"),
        "the primary action must not be placed below the other navigation buttons"
    );
    assert!(
        page.actions.len() == 5,
        "unclaimed navigation must stay compact"
    );
    assert!(
        page.body.lines().count() <= 3,
        "move detailed statistics off the claim page"
    );
    assert!(
        page.body.len() <= 240,
        "the claim page must not need body scrolling"
    );
    assert!(page.has("dashboard") && page.has("army") && page.has("help"));
    let review = last_dialog(&click(&mut state, id, "next", &[]));
    assert!(review.has("claim") && review.has("main"));
}

#[test]
fn autonomous_dialog_claim_requires_live_presence_and_refreshes_management() {
    let mut state = joined(2);
    let at = player_pos(&state, CONN).unwrap();
    let id = state
        .settlements
        .adopt_village(
            &mut state.settlement_sequence,
            "minecraft:overworld:ui-village",
            at,
        )
        .unwrap();
    let resident = villager(&mut state, at.map(f64::from));
    let resident_uuid = state.entities.row(resident).uuid;
    let changes = command(&mut state, "settlement");
    assert!(last_dialog(&changes).has("next"));
    assert!(last_dialog(&click(&mut state, id, "next", &[])).has("claim"));
    let original = state.settlements.get(id).unwrap().name.clone();
    click(&mut state, id, "rename", &[("name", string("stolen"))]);
    assert_eq!(state.settlements.get(id).unwrap().name, original);
    let projects = last_dialog(&click(&mut state, id, "projects", &[]));
    assert!(!projects.has("project_order"));
    for (action, extra) in [
        ("project_order", vec![("blueprint", string("house"))]),
        ("project_upgrade", vec![("b", string("1"))]),
    ] {
        click(&mut state, id, action, &extra);
    }
    assert!(
        state.settlements.get(id).unwrap().project.is_none(),
        "even an operator must claim before ordering construction"
    );
    let entity = state.connections.entity_of(CONN).unwrap();
    let slot = state.entities.slot_of(entity).unwrap();
    state.entities.set_pos(slot, [1000.0, 201.0, 1000.0]);
    click(&mut state, id, "claim", &[]);
    assert_eq!(state.settlements.get(id).unwrap().owner, None);
    assert_eq!(
        state
            .entities
            .row(resident)
            .villager
            .as_ref()
            .unwrap()
            .settlement,
        None,
        "a rejected remote claim must not acquire residents"
    );
    state.entities.set_pos(slot, at.map(f64::from));
    let changes = click(&mut state, id, "claim", &[]);
    assert_eq!(
        state.settlements.get(id).unwrap().owner.as_deref(),
        Some(PLAYER)
    );
    assert!(!last_dialog(&changes).has("claim"));
    assert_eq!(
        state
            .entities
            .row(resident)
            .villager
            .as_ref()
            .unwrap()
            .settlement,
        Some(id)
    );
    assert_eq!(state.entities.row(resident).uuid, resident_uuid);
}

fn blueprint_build(state: &mut TickState, id: SettlementId, blueprint: &str) -> BuildingId {
    use crate::settlements::construction::tests::{complete, fund, start};
    for x in 0..16 {
        for z in 0..16 {
            for y in 201..224 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    let slot = villager(state, [1.5, 201.0, 1.5]);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(state, id, entity).unwrap();
    let chest = block(state, "minecraft:chest", &[]);
    state.world.set_block(1, 201, 1, chest);
    state.block_entities.insert(
        (1, 201, 1),
        crate::tick::BlockEntityState {
            slots: vec![mc_data::ItemStack::EMPTY; 27],
            furnace: None,
        },
    );
    state.settlements.get_mut(id).unwrap().bootstrap_storage = Some([1, 201, 1]);
    {
        let villager = state.entities.row_mut(slot).villager.as_mut().unwrap();
        villager.data.profession = crate::villager::VillagerProfession::Builder;
        villager.activity = crate::villager::Activity::Work;
    }
    state.day_time = 3000;
    state.world.fold_overlay();
    start(state, id, blueprint, 1, None);
    fund(state, blueprint, 1);
    complete(state, id, slot);
    assert!(state.settlements.get(id).unwrap().project.is_none());
    *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap()
}
/// The flat street-plan area of the construction tests plus an empty
/// bootstrap chest and a bound resident, so an owner click runs the same
/// admission shape as the planner RED fixture. Funding stays explicit at the
/// call site: only the click may start the single project, and the click
/// itself must debit nothing.
fn prep_projects_site(state: &mut TickState, id: SettlementId) {
    crate::settlements::construction::tests::open_plan(state);
    if let std::collections::btree_map::Entry::Vacant(entry) =
        state.block_entities.entry((1, 201, 1))
    {
        entry.insert(crate::tick::BlockEntityState {
            slots: vec![mc_data::ItemStack::EMPTY; 27],
            furnace: None,
        });
        let chest = block(state, "minecraft:chest", &[]);
        state.world.set_block(1, 201, 1, chest);
        state.settlements.get_mut(id).unwrap().bootstrap_storage = Some([1, 201, 1]);
    }
    if crate::settlements::member_count(state, id) == 0 {
        let slot = villager(state, [1.5, 201.0, 1.5]);
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(state, id, entity).unwrap();
    }
    state.day_time = 3000;
    state.world.fold_overlay();
}

#[test]
fn projects_screen_offers_every_blueprint_order_to_the_owner() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    let page = last_dialog(&click(&mut state, id, "projects", &[]));
    assert!(page.title.starts_with("Projects"));
    let expected = crate::settlements::blueprints::all(&state.registry);
    assert!(!expected.is_empty());
    assert_eq!(
        page.icons.iter().filter(|(slot, _)| *slot < 45).count(),
        expected.len()
    );
    for blueprint in expected {
        assert!(page.body.contains(blueprint_label(
            &ctx(&state, CONN, id).unwrap(),
            &blueprint.id
        )));
    }
    assert!(!page.body.contains("View only"));

    // An unfunded owner order is kept without editing blocks or debiting stock.
    prep_projects_site(&mut state, id);
    let before_block = state.world.block_at(8, 201, 8);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let poor = last_dialog(&click(
        &mut state,
        id,
        "project_order",
        &[("blueprint", string("watch_tower"))],
    ));
    assert!(
        poor.body.contains("Pending: Watchtower L1"),
        "{}",
        poor.body
    );
    assert!(poor.has("project_status"));
    let details = last_dialog(&click(&mut state, id, "project_status_details", &[]));
    assert!(details.has("project_cancel_review"));
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .pending_order
            .as_ref()
            .unwrap()
            .blueprint,
        "watch_tower"
    );
    assert!(state.settlements.get(id).unwrap().project.is_none());
    let status = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(status.body.contains("Watchtower"), "{}", status.body);
    let cancelled = last_dialog(&click(&mut state, id, "project_cancel_pending", &[]));
    assert!(cancelled.body.contains("Pending order cancelled."));
    assert!(state.settlements.get(id).unwrap().pending_order.is_none());
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert_eq!(state.world.block_at(8, 201, 8), before_block);
    assert_eq!(poor.title, page.title);

    // Funded stock starts the single project immediately, with no debit or edit.
    crate::settlements::construction::tests::fund(&mut state, "watch_tower", 1);
    let before = state.block_entities[&(1, 201, 1)].slots.clone();
    let ordered = last_dialog(&click(
        &mut state,
        id,
        "project_order",
        &[("blueprint", string("watch_tower"))],
    ));
    assert!(
        ordered.body.contains("Started Watchtower construction."),
        "{}",
        ordered.body
    );
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .clone()
        .expect("funded order starts immediately");
    assert_eq!(project.blueprint, "watch_tower");
    assert_eq!(project.level, 1);
    assert!(!project.structure.is_empty());
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, before);
    assert_eq!(state.world.block_at(8, 201, 8), before_block);
    assert_eq!(ordered.title, page.title);

    // A second order is queued while the active project is busy.
    let busy = last_dialog(&click(
        &mut state,
        id,
        "project_order",
        &[("blueprint", string("house"))],
    ));
    assert!(busy.body.contains("House"), "{}", busy.body);
    assert!(busy.has("project_status"));
    assert!(busy.body.contains("busy"), "{}", busy.body);
    let status = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(status.body.contains("Pending: House L1"), "{}", status.body);
    assert!(status.body.contains("busy"), "{}", status.body);
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .pending_order
            .as_ref()
            .unwrap()
            .blueprint,
        "house"
    );
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .expect("active project kept")
            .blueprint,
        "watch_tower"
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, before);
    assert_eq!(busy.title, page.title);
}

#[test]
fn building_card_offers_construction_upgrade_only_for_blueprint_built_buildings() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    let building = blueprint_build(&mut state, id, "house");
    assert!(crate::settlements::construction::upgrade_available(
        &state, id, building
    ));
    let page = last_dialog(&click(
        &mut state,
        id,
        "buildings",
        &[("b", string(&building.0.to_string()))],
    ));
    assert!(page.body.contains("Order construction L2"), "{}", page.body);
    assert!(
        page.body.contains("Recheck certificate L2"),
        "{}",
        page.body
    );
    let changes = press(&mut state, 52);
    let cached = confirmation_payload(&changes, "krasnokamen:settlement/project_upgrade");
    let review = last_dialog(&changes);
    assert!(review.has("project_upgrade") && review.has("project_estimate"));

    // An unfunded upgrade is queued without debiting stock.
    for slot in state
        .block_entities
        .get_mut(&(1, 201, 1))
        .unwrap()
        .slots
        .iter_mut()
    {
        *slot = mc_data::ItemStack::EMPTY;
    }
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .buildings
        .get_mut(&building)
        .unwrap()
        .level = 2;
    let stale = last_dialog(&run(
        &mut state,
        &[PlayerInput::DialogClick {
            connection: CONN,
            id: "krasnokamen:settlement/project_upgrade".into(),
            payload: Some(cached.clone()),
        }],
    ));
    assert!(stale.body.contains("changed"), "{}", stale.body);
    assert!(state.settlements.get(id).unwrap().pending_order.is_none());
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    let record = state.settlements.get_mut(id).unwrap();
    record.buildings.get_mut(&building).unwrap().level = 1;
    record.revision += 1;
    let poor = last_dialog(&run(
        &mut state,
        &[PlayerInput::DialogClick {
            connection: CONN,
            id: "krasnokamen:settlement/project_upgrade".into(),
            payload: Some(cached),
        }],
    ));
    assert!(poor.body.contains("Pending: House L2"), "{}", poor.body);
    assert!(poor.has("project_status"));
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .pending_order
            .as_ref()
            .unwrap()
            .building,
        Some(building)
    );
    assert!(state.settlements.get(id).unwrap().project.is_none());
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    click(&mut state, id, "project_cancel_pending", &[]);

    // Funded L2 stock starts the upgrade immediately, with no debit.
    crate::settlements::construction::tests::fund(&mut state, "house", 2);
    let before = state.block_entities[&(1, 201, 1)].slots.clone();
    let started = last_dialog(&click(
        &mut state,
        id,
        "project_upgrade",
        &[("b", string(&building.0.to_string()))],
    ));
    assert!(
        started.body.contains("Started House construction."),
        "{}",
        started.body
    );
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .clone()
        .expect("funded upgrade starts immediately");
    assert_eq!(project.building, Some(building));
    assert_eq!(project.level, 2);
    assert!(!project.structure.is_empty());
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, before);

    // While the ordered L2 upgrade is active, the next order targets L3.
    let page = last_dialog(&click(
        &mut state,
        id,
        "buildings",
        &[("b", string(&building.0.to_string()))],
    ));
    assert!(page.body.contains("Order construction L3"), "{}", page.body);
    let queued = last_dialog(&click(
        &mut state,
        id,
        "project_upgrade",
        &[("b", string(&building.0.to_string()))],
    ));
    assert!(queued.body.contains("Pending: House L3"), "{}", queued.body);

    // A player-certified building of the same kind is never offered an upgrade.
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    station(&mut state, [4, 201, 4], "minecraft:composter");
    click(&mut state, id, "certify_do", &certify_form("farmer"));
    let building = BuildingId(id.0 << 32 | 1);
    let page = last_dialog(&click(
        &mut state,
        id,
        "buildings",
        &[("b", string(&building.0.to_string()))],
    ));
    assert!(page.body.contains("No blueprint upgrade"), "{}", page.body);
}

#[test]
fn catalog_and_project_status_are_native_graphical_screens() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    blueprint_build(&mut state, id, "house");
    let changes = click(&mut state, id, "projects", &[]);
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, PlayerChange::MenuContents { .. }))
    );
    let page = last_dialog(&changes);
    assert_eq!(
        page.icons.iter().filter(|(slot, _)| *slot < 45).count(),
        crate::settlements::blueprints::all(&state.registry).len()
    );
    assert!(page.icons.iter().any(|(slot, _)| *slot == 49));
    assert!(page.icons.iter().any(|(slot, _)| *slot == 52));
    let status = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert_eq!(status.title, "Construction");
    assert!(
        status.body.contains("No active project; no queued order"),
        "{}",
        status.body
    );
    assert!(!status.icons.iter().any(|(slot, _)| (18..45).contains(slot)));
}

#[test]
fn projects_show_localized_missing_owned_furnace_for_house_wool() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    prep_projects_site(&mut state, id);
    let worker = villager(&mut state, [8.5, 201.0, 8.5]);
    let entity = state.entities.body(worker).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    let sheep = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:sheep").unwrap())
        .unwrap();
    state.spawn_mob(sheep, [8.5, 201.0, 8.5], 8.0);
    let item = |name: &str| {
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap()
    };
    let raw = item("minecraft:raw_iron");
    let coal = item("minecraft:coal");
    let logs = item("minecraft:oak_log");
    let chest = state.block_entities.get_mut(&(1, 201, 1)).unwrap();
    chest.slots[0] = mc_data::ItemStack::new(raw, 2);
    chest.slots[1] = mc_data::ItemStack::new(coal, 1);
    chest.slots[2] = mc_data::ItemStack::new(logs, 64);
    state.settlements.get_mut(id).unwrap().pending_order =
        Some(crate::settlements::construction::OwnerOrder {
            blueprint: "house".into(),
            level: 1,
            building: None,
        });
    let english = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(
        english.body.contains("needs a settlement-owned furnace"),
        "{}",
        english.body
    );
    set_locale(&mut state, "ru_ru");
    let russian = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(
        russian.body.contains("нужна печь поселения"),
        "{}",
        russian.body
    );
}

#[test]
fn pending_order_has_exact_native_material_icons_and_owner_only_cancel_review() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    prep_projects_site(&mut state, id);
    let receipt = last_dialog(&click(
        &mut state,
        id,
        "project_order",
        &[("blueprint", string("watch_tower"))],
    ));
    assert!(
        receipt.body.contains("Pending: Watchtower L1"),
        "{}",
        receipt.body
    );
    assert!(receipt.has("project_status"));
    let needs = super::super::construction::waiting(&state, id)
        .into_iter()
        .find(|kind| kind.blueprint == "watch_tower")
        .expect("pending materials")
        .needs;
    assert!(!needs.is_empty());
    let mut page = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(
        page.body.contains("Pending: Watchtower L1"),
        "{}",
        page.body
    );
    assert!(page.body.contains("waiting for materials"), "{}", page.body);
    assert!(!page.body.contains("House L1"));
    let pages = needs.len().div_ceil(18).max(1);
    let mut material_icons = Vec::new();
    for index in 0..pages {
        if index > 0 {
            page = last_dialog(&press(&mut state, 50));
        }
        assert_eq!(
            page.icons.iter().any(|(slot, _)| *slot == 50),
            index + 1 < pages
        );
        material_icons.extend(
            page.icons
                .drain(..)
                .filter(|(slot, _)| (27..45).contains(slot)),
        );
    }
    assert_eq!(material_icons.len(), needs.len());
    for need in &needs {
        let item = state
            .registry
            .items
            .id_of(&Identifier::parse(&need.item).unwrap())
            .unwrap();
        let (_, icon) = material_icons
            .iter()
            .find(|(_, icon)| icon.item_id == item)
            .expect("every native bill entry remains reachable");
        let missing = need.total.saturating_sub(need.have);
        assert_eq!(icon.count, missing.min(99) as i32);
        assert!(
            icon_text(icon).contains(&format!(" ×{missing}")),
            "{icon:?}"
        );
        let label = material_text(&state, &need.item, String::new());
        assert!(icon_text(icon).contains(&text(field(&label, "translate").unwrap())));
    }
    let details = last_dialog(&click(&mut state, id, "project_status_details", &[]));
    assert!(details.has("project_cancel_review"));
    state.settlements.get_mut(id).unwrap().owner = Some("someone-else".into());
    let visitor = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(visitor.body.contains("Pending: Watchtower L1"));
    let details = last_dialog(&click(&mut state, id, "project_status_details", &[]));
    assert!(!details.has("project_cancel_review"));
    let denied = last_dialog(&click(&mut state, id, "project_cancel_pending", &[]));
    assert!(denied.body.contains("Only the owner or an operator"));
    assert!(state.settlements.get(id).unwrap().pending_order.is_some());
    state.settlements.get_mut(id).unwrap().owner = Some(PLAYER.into());
    set_locale(&mut state, "ru_ru");
    let status = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(status.body.contains("В очереди: Сторожевая башня ур. 1"));
    assert!(
        status.body.contains("не хватает материалов"),
        "{}",
        status.body
    );
    let details = last_dialog(&click(&mut state, id, "project_status_details", &[]));
    assert!(details.has("project_cancel_review"));
    let review = last_dialog(&click(&mut state, id, "project_cancel_review", &[]));
    assert!(review.has("project_cancel_pending"));
    let cancelled = last_dialog(&click(&mut state, id, "project_cancel_pending", &[]));
    assert!(cancelled.body.contains("Заказ в очереди отменён."));
    assert!(state.settlements.get(id).unwrap().pending_order.is_none());
}

#[test]
fn pending_funded_order_shows_localized_waiting_for_site() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    prep_projects_site(&mut state, id);
    for x in 0..16 {
        for z in 0..16 {
            if (x, z) != (1, 1) {
                for y in 201..=216 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    let queued = last_dialog(&click(
        &mut state,
        id,
        "project_order",
        &[("blueprint", string("watch_tower"))],
    ));
    assert!(
        queued.body.contains("Pending: Watchtower L1"),
        "{}",
        queued.body
    );
    crate::settlements::construction::tests::fund(&mut state, "watch_tower", 1);
    let projects = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(
        projects.body.contains("waiting for a site"),
        "{}",
        projects.body
    );
    let status = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(
        status.body.contains("waiting for a site"),
        "{}",
        status.body
    );
    set_locale(&mut state, "ru_ru");
    let projects = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(
        projects.body.contains("ожидание места для стройки"),
        "{}",
        projects.body
    );
}

#[test]
fn selected_shortage_preserves_native_translatable_production_input_hint() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    prep_projects_site(&mut state, id);
    click(
        &mut state,
        id,
        "project_order",
        &[("blueprint", string("house"))],
    );
    let need = super::super::construction::waiting(&state, id)
        .into_iter()
        .find(|kind| kind.blueprint == "house")
        .unwrap()
        .needs
        .into_iter()
        .find(|need| need.item == "minecraft:oak_planks")
        .unwrap();
    let source = need.source_item.as_ref().expect("native production input");
    let item = state
        .registry
        .items
        .id_of(&Identifier::parse(&need.item).unwrap())
        .unwrap();
    let page = last_dialog(&click(&mut state, id, "project_status", &[]));
    let slot = page
        .icons
        .iter()
        .find(|(slot, stack)| (27..45).contains(slot) && stack.item_id == item)
        .expect("native planks shortage icon")
        .0;
    let detail = last_dialog(&press(&mut state, slot as i16));
    assert!(
        detail.body.contains("block.minecraft.oak_planks"),
        "{}",
        detail.body
    );
    let source_label = material_text(&state, source, String::new());
    assert!(
        detail.body.contains(&text(&source_label)),
        "{}",
        detail.body
    );
    assert!(
        detail
            .body
            .contains("Native production input, for example:")
    );
    assert!(!detail.body.contains("minecraft:oak_log"));
    assert!(detail.has("storage") && detail.has("orders") && detail.has("project_status"));
}

#[test]
fn project_status_does_not_claim_all_candidates_lack_materials_when_one_is_funded() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    prep_projects_site(&mut state, id);
    assert!(!super::super::construction::has_funded_candidate(
        &state, id
    ));
    crate::settlements::construction::tests::fund(&mut state, "storage", 1);
    assert!(super::super::construction::has_funded_candidate(&state, id));
}

#[test]
fn project_status_active_shows_readable_progress() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    prep_projects_site(&mut state, id);
    crate::settlements::construction::tests::fund(&mut state, "watch_tower", 1);
    click(
        &mut state,
        id,
        "project_order",
        &[("blueprint", string("watch_tower"))],
    );
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .clone()
        .expect("native admitted project");
    let snapshot =
        crate::plugin_structures::settlement_builds::status(&state, id, &project.structure)
            .unwrap();
    let total: u64 = snapshot.stages.iter().map(|stage| stage.work_units).sum();
    let done = snapshot
        .stages
        .iter()
        .take(snapshot.current_stage_index as usize)
        .map(|stage| stage.work_units)
        .sum::<u64>()
        + snapshot.completed_work_units;
    let status = last_dialog(&click(&mut state, id, "project_status", &[]));
    assert!(
        status.body.contains(&format!("{done}/{total}")),
        "{}",
        status.body
    );
    let bar = status
        .icons
        .iter()
        .filter(|(slot, _)| (18..27).contains(slot))
        .collect::<Vec<_>>();
    assert_eq!(bar.len(), 9);
    for (segment, (_, icon)) in bar.iter().enumerate() {
        let full = total > 0 && u128::from(done) * 9 >= u128::from(total) * (segment + 1) as u128;
        let expected = if full {
            "minecraft:lime_stained_glass_pane"
        } else {
            "minecraft:gray_stained_glass_pane"
        };
        assert_eq!(
            icon.item_id,
            state
                .registry
                .items
                .id_of(&Identifier::parse(expected).unwrap())
                .unwrap()
        );
        assert!(icon_text(icon).contains(&format!("{done}/{total}")));
    }
    let site = &status
        .icons
        .iter()
        .find(|(slot, _)| *slot == 14)
        .expect("actual site icon")
        .1;
    assert!(icon_text(site).contains(&corner_text(project.origin)));
}

#[test]
fn projects_clicks_are_read_only_for_visitors_and_forged_payloads_start_nothing() {
    let mut state = joined(0);
    let id = town(&mut state, "someone-else");
    let page = last_dialog(&click(&mut state, id, "projects", &[]));
    assert!(page.body.contains("View only"));
    let preview = last_dialog(&press(&mut state, 0));
    assert!(!preview.has("project_order"));
    assert!(
        preview.has("project_estimate"),
        "visitors retain the real quote, not mutation"
    );

    for (action, extra) in [
        ("project_order", vec![("blueprint", string("house"))]),
        ("project_upgrade", vec![("b", string("1"))]),
    ] {
        let refused = last_dialog(&click(&mut state, id, action, &extra));
        assert!(
            refused.body.contains("Only the owner or an operator"),
            "{action}"
        );
    }
    assert!(state.settlements.get(id).unwrap().project.is_none());

    // Owner clicks with forged or stale payloads start nothing and stay on page.
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    let forged_blueprint = last_dialog(&click(
        &mut state,
        id,
        "project_order",
        &[("blueprint", string("castle"))],
    ));
    assert!(forged_blueprint.body.contains("Unknown blueprint"));
    let stale_building = last_dialog(&click(
        &mut state,
        id,
        "project_upgrade",
        &[("b", string("999"))],
    ));
    assert!(stale_building.body.contains("no longer exists"));
    let foreign = last_dialog(&click(
        &mut state,
        id,
        "project_upgrade",
        &[("b", string(&(5u64 << 32 | 1).to_string()))],
    ));
    assert!(foreign.body.contains("no longer exists"));
    assert!(state.settlements.get(id).unwrap().project.is_none());
}

// ------------------------------------------------------------------ localisation

/// Play `ClientInformation` locale updates arrive as `PlayerInput::SetLocale`.
fn set_locale(state: &mut TickState, locale: &str) {
    run_tick(
        state,
        &[PlayerInput::SetLocale {
            connection: CONN,
            locale: locale.into(),
        }],
    );
}

#[test]
fn russian_locale_localizes_the_main_menu_and_army_page() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    set_locale(&mut state, "ru_ru");

    let main = last_dialog(&command(&mut state, "settlement"));
    assert_eq!(main.title, "Поселение oak");
    assert_eq!(main.actions.len(), 5);
    assert_eq!(main.label_of("next"), "Следующий шаг");
    assert_eq!(main.label_of("dashboard"), "Поселение");
    assert_eq!(main.label_of("army"), "Армия");
    assert_eq!(main.label_of("neighbours"), "Соседи и война");
    assert_eq!(main.label_of("help"), "Помощь");
    assert!(main.body.contains("Mayor"), "{}", main.body);
    let dashboard = last_dialog(&click(&mut state, id, "dashboard", &[]));
    for label in ["Постройки", "Жители и работа", "Стройка", "Склад", "Помощь"]
    {
        assert!(
            dashboard.body.contains(label),
            "{label}: {}",
            dashboard.body
        );
    }

    let army = last_dialog(&click(&mut state, id, "army", &[]));
    assert_eq!(army.title, "Армия — oak");
    assert_eq!(army.label_of("army_recruit"), "Нанять солдата");
    assert_eq!(army.label_of("main"), "Назад");
    assert!(army.body.contains("Солдат нет"), "{}", army.body);
}

#[test]
fn en_us_locale_keeps_the_settlement_dialogs_english() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    set_locale(&mut state, "en_us");

    let main = last_dialog(&command(&mut state, "settlement"));
    assert_eq!(main.title, "Settlement oak");
    assert_eq!(main.label_of("army"), "Army");
    assert!(main.body.contains("Mayor"), "{}", main.body);
    assert_eq!(main.actions.len(), 5);
    assert_eq!(main.label_of("dashboard"), "Settlement");
    assert_eq!(main.label_of("help"), "Help");

    let army = last_dialog(&click(&mut state, id, "army", &[]));
    assert_eq!(army.title, "Army - oak");
    assert_eq!(army.label_of("army_recruit"), "Recruit soldier");
    assert_eq!(army.label_of("main"), "Back");
}

#[test]
fn play_locale_update_applies_to_the_next_open() {
    let mut state = joined(0);
    town(&mut state, PLAYER);
    let first = last_dialog(&command(&mut state, "settlement"));
    assert_eq!(first.title, "Settlement oak", "no locale yet reads English");

    set_locale(&mut state, "ru_ru");
    let second = last_dialog(&command(&mut state, "settlement"));
    assert_eq!(second.title, "Поселение oak");

    set_locale(&mut state, "en_gb");
    let third = last_dialog(&command(&mut state, "settlement"));
    assert_eq!(third.title, "Settlement oak");
}

#[test]
fn locale_is_recorded_before_join_and_cleared_on_leave() {
    let mut state = test_state();
    run_tick(
        &mut state,
        &[PlayerInput::SetLocale {
            connection: CONN,
            locale: "ru_ru".into(),
        }],
    );
    assert_eq!(state.connections.locale_of(CONN), Some("ru_ru"));

    run_tick(&mut state, &[PlayerInput::Leave { connection: CONN }]);
    assert_eq!(state.connections.locale_of(CONN), None);
}

#[test]
fn reviews_ignore_simulation_revisions_but_refuse_changed_pending_orders_and_spectators() {
    let mut state = joined(0);
    let id = town(&mut state, PLAYER);
    prep_projects_site(&mut state, id);
    let changes = click(
        &mut state,
        id,
        "project_order_review",
        &[("blueprint", string("house"))],
    );
    let cached = confirmation_payload(&changes, "krasnokamen:settlement/project_order");
    assert_eq!(text(field(&cached, "pending_blueprint").unwrap()), "");
    assert_eq!(text(field(&cached, "pending_level").unwrap()), "0");
    assert_eq!(text(field(&cached, "pending_building").unwrap()), "0");
    state.settlements.get_mut(id).unwrap().revision += 1;
    let queued = last_dialog(&run(
        &mut state,
        &[PlayerInput::DialogClick {
            connection: CONN,
            id: "krasnokamen:settlement/project_order".into(),
            payload: Some(cached.clone()),
        }],
    ));
    assert!(queued.body.contains("Pending: House L1"), "{}", queued.body);
    assert!(state.settlements.get(id).unwrap().pending_order.is_some());
    for payload in [
        cached,
        Tag::Compound(vec![
            ("s".into(), string(&id.0.to_string())),
            ("blueprint".into(), string("house")),
        ]),
    ] {
        let refused = last_dialog(&run(
            &mut state,
            &[PlayerInput::DialogClick {
                connection: CONN,
                id: "krasnokamen:settlement/project_order".into(),
                payload: Some(payload),
            }],
        ));
        assert!(refused.body.contains("changed"), "{}", refused.body);
        let record = state.settlements.get(id).unwrap();
        assert!(record.project.is_none() && record.pending_order.is_some());
    }
    click(
        &mut state,
        id,
        "project_order",
        &[("blueprint", string("house"))],
    );
    assert!(state.settlements.get(id).unwrap().pending_order.is_some());
    let entity = state.connections.entity_of(CONN).unwrap();
    state.players.get_mut(&entity).unwrap().game_mode = crate::commands::GameMode::Spectator;
    let refused = last_dialog(&click(&mut state, id, "project_cancel_pending", &[]));
    assert!(refused.body.contains("Only the owner or an operator"));
    assert!(state.settlements.get(id).unwrap().pending_order.is_some());
}
