//! DV5 routes use two planner-admitted houses, not injected UI/project records.
use super::*;
use crate::settlements::construction::{self, tests as construction_tests};

fn admitted_sites() -> (TickState, SettlementId, Vec<construction::Project>) {
    let (mut state, id, first_builder) = construction_tests::fixture_in(joined(0));
    state.settlements.get_mut(id).unwrap().owner = Some(PLAYER.to_owned());
    construction_tests::open_plan(&mut state);
    construction_tests::fund(&mut state, "house", 1);
    let bill: Vec<_> = state.block_entities[&(1, 201, 1)]
        .slots
        .iter()
        .filter(|stack| !stack.is_empty())
        .cloned()
        .collect();
    for stack in bill {
        *state
            .block_entities
            .get_mut(&(1, 201, 1))
            .unwrap()
            .slots
            .iter_mut()
            .find(|slot| slot.is_empty())
            .expect("two authored bills fit") = stack;
    }
    let second_builder = villager(&mut state, [2.5, 201.0, 4.5]);
    let entity = state.entities.body(second_builder).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    let worker = state
        .entities
        .row_mut(second_builder)
        .villager
        .as_mut()
        .unwrap();
    worker.data.profession = Profession::Builder;
    worker.activity = crate::villager::Activity::Work;
    construction_tests::request_started(&mut state, id, "house");
    let first = construction::projects_of(&state.settlements, id)
        .next()
        .unwrap()
        .structure
        .clone();
    construction_tests::bind_project(&mut state, id, &first, first_builder);
    construction_tests::request_started(&mut state, id, "house");
    let projects: Vec<_> = construction::projects_of(&state.settlements, id)
        .cloned()
        .collect();
    assert_eq!(
        projects.len(),
        2,
        "both owner orders must actually be admitted"
    );
    assert_ne!(projects[0].structure, projects[1].structure);
    assert_ne!(projects[0].origin, projects[1].origin);
    construction_tests::bind_project(&mut state, id, &projects[1].structure, second_builder);
    let projects: Vec<_> = construction::projects_of(&state.settlements, id)
        .cloned()
        .collect();
    for project in &projects {
        let snapshot =
            crate::plugin_structures::settlement_builds::status(&state, id, &project.structure)
                .unwrap();
        assert!(
            snapshot
                .stages
                .iter()
                .map(|stage| stage.work_units)
                .sum::<u64>()
                > 0
        );
    }
    state.tick_control.set_frozen(true);
    (state, id, projects)
}

fn dialog(changes: &[PlayerChange]) -> Tag {
    changes
        .iter()
        .rev()
        .find_map(|change| match change {
            PlayerChange::ShowDialog { dialog } => Some(dialog.clone()),
            _ => None,
        })
        .expect("the actual dispatch must display a dialog")
}

fn emitted_actions(tag: &Tag) -> Vec<Tag> {
    let mut actions = Vec::new();
    if field(tag, "id").is_some_and(|id| text(id).starts_with("krasnokamen:settlement/")) {
        actions.push(tag.clone());
    }
    match tag {
        Tag::Compound(fields) => {
            for (_, child) in fields {
                actions.extend(emitted_actions(child));
            }
        }
        Tag::List(list) => {
            for child in &list.elements {
                actions.extend(emitted_actions(child));
            }
        }
        _ => {}
    }
    actions
}

fn payload(action: &Tag) -> &Tag {
    field(action, "additions")
        .or_else(|| field(action, "payload"))
        .expect("rendered payload")
}

fn send(state: &mut TickState, action: &Tag) -> Vec<PlayerChange> {
    run(
        state,
        &[PlayerInput::DialogClick {
            connection: CONN,
            id: text(field(action, "id").unwrap()),
            payload: Some(payload(action).clone()),
        }],
    )
}

fn back(state: &mut TickState, from: &Tag) -> Vec<PlayerChange> {
    let action = emitted_actions(from)
        .into_iter()
        .find(|action| text(field(action, "id").unwrap()) == "krasnokamen:settlement/back")
        .expect("actual Back control");
    send(state, &action)
}

fn map_at(state: &mut TickState, id: SettlementId, centre: [i32; 2], scale: i32) -> Tag {
    dialog(&click(
        state,
        id,
        "map",
        &[
            ("mx", string(&centre[0].to_string())),
            ("mz", string(&centre[1].to_string())),
            ("ms", string(&scale.to_string())),
        ],
    ))
}

fn single_marker(map: &Tag, structure: &str) -> Tag {
    emitted_actions(map)
        .into_iter()
        .find(|action| {
            let payload = payload(action);
            field(payload, "sites").is_some_and(|sites| text(sites) == structure)
                && ["bids", "streets"]
                    .into_iter()
                    .all(|key| field(payload, key).is_none_or(|value| text(value).is_empty()))
                && field(payload, "storage").is_none_or(|value| text(value) != "1")
        })
        .expect("the rendered viewport has a singleton site marker")
}

fn assert_selected(
    state: &TickState,
    id: SettlementId,
    selected: &Tag,
    project: &construction::Project,
    neighbour: &construction::Project,
) {
    let ctx = ctx(state, CONN, id).unwrap();
    let expected = project_text(state, &ctx, project).unwrap();
    let shown = shown(selected);
    assert_eq!(
        shown.body, expected,
        "exactly the selected native facts, not an aggregate"
    );
    assert!(!shown.body.contains(&corner_text(neighbour.origin)));
    assert_eq!(list(field(selected, "body")).len(), 1);
}

#[test]
fn dv5_singleton_markers_select_distinct_admitted_projects_and_restore_exact_viewports() {
    let (mut state, id, projects) = admitted_sites();
    for (index, project) in projects.iter().enumerate() {
        let origin = [project.origin[0] + 2, project.origin[2] + 2];
        let map = map_at(&mut state, id, origin, 1);
        let selected = dialog(&send(&mut state, &single_marker(&map, &project.structure)));
        assert_selected(&state, id, &selected, project, &projects[1 - index]);
        assert_eq!(dialog(&back(&mut state, &selected)), map);
    }
}

#[test]
fn dv5_overlap_chooser_preserves_each_structure_and_both_back_origins() {
    let (mut state, id, projects) = admitted_sites();
    state.settlements.get_mut(id).unwrap().radius = 256;
    let centre = [
        (projects[0].origin[0] + projects[1].origin[0]) / 2,
        (projects[0].origin[2] + projects[1].origin[2]) / 2,
    ];
    let map = map_at(&mut state, id, centre, 31);
    let marker = emitted_actions(&map)
        .into_iter()
        .find(|action| {
            let sites = field(payload(action), "sites")
                .map(text)
                .unwrap_or_default();
            sites.lines().count() == 2
                && projects
                    .iter()
                    .all(|project| sites.lines().any(|site| site == project.structure))
        })
        .expect("one coarse rendered cell carries both legal sites");
    let chooser = dialog(&send(&mut state, &marker));
    for (index, project) in projects.iter().enumerate() {
        let action = emitted_actions(&chooser)
            .into_iter()
            .find(|action| {
                field(payload(action), "structure")
                    .is_some_and(|value| text(value) == project.structure)
            })
            .expect("each chooser action retains its own structure handle");
        let selected = dialog(&send(&mut state, &action));
        assert_selected(&state, id, &selected, project, &projects[1 - index]);
        assert_eq!(dialog(&back(&mut state, &selected)), chooser);
    }
    assert_eq!(dialog(&back(&mut state, &chooser)), map);
}

#[test]
fn dv5_cached_site_and_chooser_actions_refuse_removed_or_foreign_projects() {
    let (mut state, id, projects) = admitted_sites();
    let map = map_at(
        &mut state,
        id,
        [projects[0].origin[0] + 2, projects[0].origin[2] + 2],
        1,
    );
    let marker = single_marker(&map, &projects[0].structure);
    // Capture a real chooser action before removing the same selected site.
    state.settlements.get_mut(id).unwrap().radius = 256;
    let centre = [
        (projects[0].origin[0] + projects[1].origin[0]) / 2,
        (projects[0].origin[2] + projects[1].origin[2]) / 2,
    ];
    let overview = map_at(&mut state, id, centre, 31);
    let overlap = emitted_actions(&overview)
        .into_iter()
        .find(|action| {
            field(payload(action), "sites").is_some_and(|value| text(value).lines().count() == 2)
        })
        .unwrap();
    let chooser = dialog(&send(&mut state, &overlap));
    let selected_action = emitted_actions(&chooser)
        .into_iter()
        .find(|action| {
            field(payload(action), "structure")
                .is_some_and(|value| text(value) == projects[0].structure)
        })
        .unwrap();
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .projects
        .retain(|project| project.structure != projects[0].structure);
    for action in [&marker, &selected_action] {
        let selected = dialog(&send(&mut state, action));
        assert_eq!(
            shown(&selected).body,
            ui_text::text(Lang::En, Key::StateChanged)
        );
        assert!(
            !shown(&selected)
                .body
                .contains(&corner_text(projects[1].origin))
        );
    }
    let other = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "other".into(),
            PLAYER.into(),
            [100, 201, 100],
            8,
        )
        .unwrap();
    let foreign = dialog(&click(
        &mut state,
        other,
        "project_selected",
        &[("structure", string(&projects[1].structure))],
    ));
    assert_eq!(
        shown(&foreign).body,
        ui_text::text(Lang::En, Key::StateChanged)
    );
    assert!(
        construction::project_for_structure(
            state.settlements.get(id).unwrap(),
            &projects[1].structure
        )
        .is_some()
    );
}

#[test]
fn dv5_active_icons_select_one_project_for_visitors_and_back_to_actual_menu() {
    let (mut state, id, projects) = admitted_sites();
    state.settlements.get_mut(id).unwrap().owner = Some(uuid::Uuid::from_u128(99).to_string());
    for locale in ["en_us", "ru_ru"] {
        set_locale(&mut state, locale);
        let aggregate = last_dialog(&click(&mut state, id, "project_status", &[]));
        for project in &projects {
            assert!(aggregate.body.contains(&corner_text(project.origin)));
        }
        for (index, project) in projects.iter().enumerate() {
            let selected = dialog(&press(&mut state, index as i16));
            assert_selected(&state, id, &selected, project, &projects[1 - index]);
            let restored = last_dialog(&back(&mut state, &selected));
            assert_eq!(restored.body, aggregate.body);
        }
        // The explicit Status control remains settlement-wide.
        let details = dialog(&press(&mut state, 49));
        let body = shown(&details).body;
        for project in &projects {
            assert!(body.contains(&corner_text(project.origin)));
        }
    }
}

#[test]
fn dv5_cached_active_icon_refuses_removed_site_instead_of_retargeting_its_slot() {
    let (mut state, id, projects) = admitted_sites();
    click(&mut state, id, "project_status", &[]);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .projects
        .retain(|project| project.structure != projects[0].structure);
    let refused = dialog(&press(&mut state, 0));
    assert_eq!(
        shown(&refused).body,
        ui_text::text(Lang::En, Key::StateChanged)
    );
    back(&mut state, &refused);
    let selected = dialog(&press(&mut state, 0));
    assert_selected(&state, id, &selected, &projects[1], &projects[0]);
}
