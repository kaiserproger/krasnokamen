//! One connection-local history shared by chest menus and modal dialogs.
//! Dialogs restore the last displayed view; every action still revalidates live
//! authority and state. Chest views are re-rendered from current world state.
use super::{Ctx, menus};
use crate::boundary::PlayerChange;
use crate::settlements::SettlementId;
use crate::tick::TickState;
use mc_domain::ConnectionId;
use mc_nbt::Tag;

const MAX_HISTORY: usize = 16;

#[derive(Clone)]
enum Screen {
    Menu(menus::View),
    Dialog(&'static str, Tag),
}

pub(crate) struct Navigation {
    settlement: SettlementId,
    current: Screen,
    previous: Vec<Screen>,
}

fn record(
    state: &mut TickState,
    connection: ConnectionId,
    settlement: SettlementId,
    screen: Screen,
) {
    let navigation = match state.settlement_navigation.entry(connection) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(Navigation {
                settlement,
                current: screen,
                previous: Vec::new(),
            });
            return;
        }
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
    };
    if navigation.settlement != settlement {
        navigation.settlement = settlement;
        navigation.previous.clear();
        navigation.current = screen;
        return;
    }
    let same_view = match (&navigation.current, &screen) {
        (Screen::Menu(current), Screen::Menu(next)) => {
            std::mem::discriminant(current) == std::mem::discriminant(next)
        }
        (Screen::Dialog(current_key, _), Screen::Dialog(next_key, _)) => current_key == next_key,
        _ => false,
    };
    if same_view {
        navigation.current = screen;
        return;
    }
    if navigation.previous.len() == MAX_HISTORY {
        navigation.previous.remove(0);
    }
    navigation
        .previous
        .push(std::mem::replace(&mut navigation.current, screen));
}

pub(super) fn menu(
    state: &mut TickState,
    connection: ConnectionId,
    settlement: SettlementId,
    view: menus::View,
) {
    record(state, connection, settlement, Screen::Menu(view));
}

pub(super) fn dialog(
    state: &mut TickState,
    connection: ConnectionId,
    settlement: SettlementId,
    key: &'static str,
    dialog: Tag,
) {
    record(
        state,
        connection,
        settlement,
        Screen::Dialog(key, dialog.clone()),
    );
    state.push_player(connection, PlayerChange::ShowDialog { dialog });
}

pub(super) fn back(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    menus::close_for_dialog(state, connection);
    let previous = state
        .settlement_navigation
        .get_mut(&connection)
        .filter(|navigation| navigation.settlement == ctx.id)
        .and_then(|navigation| {
            let previous = navigation.previous.pop()?;
            navigation.current = previous.clone();
            Some(previous)
        });
    match previous {
        Some(Screen::Menu(view)) => menus::show(state, connection, ctx, view),
        Some(Screen::Dialog(_, dialog)) => {
            state.push_player(connection, PlayerChange::ShowDialog { dialog })
        }
        None => crate::dialog::clear(state, connection),
    }
}
