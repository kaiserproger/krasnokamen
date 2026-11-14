//! Player session checks and authoritative book inventory commits.
use super::TickState;
use crate::boundary::{ConnectionId, EntityId, Hand, PlayerChange};

pub(crate) fn use_item(state: &mut TickState, actor: EntityId, hand: Hand) -> bool {
    let Some(player) = state.players.get(&actor) else {
        return false;
    };
    let held = &player.inventory.slots[super::held_slot_index(player, hand)];
    let Some(name) = state.registry.items.name_of(held.item_id) else {
        return false;
    };
    if held.is_empty()
        || !matches!(
            name.as_str(),
            "minecraft:writable_book" | "minecraft:written_book"
        )
    {
        return false;
    }
    if name.as_str() == "minecraft:written_book" && !player.vitals.is_dead() {
        state.push_player(player.connection, PlayerChange::BookOpened { hand });
    }
    true
}

pub(crate) fn edit(
    state: &mut TickState,
    connection: ConnectionId,
    slot: i32,
    pages: &[String],
    title: Option<&str>,
) {
    if !((0..=8).contains(&slot) || slot == 40) {
        return;
    }
    let Some(actor) = state.connections.entity_of(connection) else {
        return;
    };
    let Some(player) = state.players.get(&actor) else {
        return;
    };
    if player.spectator() || player.vitals.is_dead() {
        return;
    }
    let slot = slot as usize;
    let author = if title.is_some() {
        state.connections.name_of(actor).expect("connected author")
    } else {
        ""
    };
    let Some(replacement) = crate::books::replacement(
        &player.inventory.slots[slot],
        &state.registry.items,
        pages,
        title,
        author,
    ) else {
        return;
    };
    if crate::plugin_inventories::below_player_replacement_floors(
        state,
        actor,
        &[(slot, &replacement)],
    ) {
        let effects = super::inventory_transition_refused(state, actor);
        for change in effects.changes {
            state.push_player(connection, change);
        }
        return;
    }
    let player = state.players.get_mut(&actor).expect("book owner");
    player.inventory.slots[slot] = replacement;
    let change = player.inventory_contents();
    state.push_player(connection, change);
}

#[cfg(test)]
#[path = "../books_tests.rs"]
mod tests;
