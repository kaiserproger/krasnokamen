use super::bundle;
use super::click_tests::{BUCKET, BUNDLE, STONE, Session, WATER_BUCKET, stack};
use super::{ItemRules, MenuKind};
use crate::boundary::ClickMode;
use mc_data::ItemStack;

fn contents(s: &Session, bundle_stack: &ItemStack) -> Vec<ItemStack> {
    let rules: ItemRules<'_> = s.data.rules();
    bundle::read(&rules, bundle_stack).expect("bundle contents decode")
}

#[test]
fn primary_pickup_clears_bundle_selection_before_cursor_removal() {
    let mut s = Session::new(MenuKind::Player);
    let rules = s.data.rules();
    let older = stack(BUCKET, 2).with_custom_name("kept name");
    let mut full = stack(BUNDLE, 1);
    assert!(bundle::write(
        &rules,
        &mut full,
        &[stack(STONE, 5), older.clone()]
    ));
    bundle::select(&rules, &mut full, 1);
    s.put(9, full);
    s.click(9, 0, ClickMode::Pickup);
    assert_eq!(s.menu.carried.extended().bundle_selection.0, None);
    s.click(10, 1, ClickMode::Pickup);
    assert_eq!(s.slot(10), stack(STONE, 5));
    assert_eq!(contents(&s, &s.menu.carried), vec![older]);
    assert_eq!(s.menu.carried.extended().bundle_selection.0, None);
}

#[test]
fn selected_held_bundle_drops_selected_then_default() {
    use crate::boundary::{ConnectionId, Hand};
    use crate::commands::GameMode;
    use crate::player::PlayerState;
    let s = Session::new(MenuKind::Player);
    let rules = s.data.rules();
    let mut player = PlayerState::new(ConnectionId(1), GameMode::Survival);
    let mut held = stack(BUNDLE, 1);
    assert!(bundle::write(
        &rules,
        &mut held,
        &[stack(STONE, 5), stack(BUCKET, 2)]
    ));
    bundle::select(&rules, &mut held, 1);
    player.inventory.slots[0] = held;
    player.use_item(&rules, Hand::Main);
    assert_eq!(player.tick_item_use(&rules).dropped, vec![stack(BUCKET, 2)]);
    for _ in 0..11 {
        assert!(player.tick_item_use(&rules).dropped.is_empty());
    }
    assert_eq!(player.tick_item_use(&rules).dropped, vec![stack(STONE, 5)]);
}

#[test]
fn selection_toggle_invalid_and_mutation_reset() {
    let s = Session::new(MenuKind::Player);
    let rules = s.data.rules();
    let mut held = stack(BUNDLE, 1);
    let entries = [stack(STONE, 5), stack(BUCKET, 2)];
    assert!(bundle::write(&rules, &mut held, &entries));
    for (index, expected) in [
        (1, Some(1)),
        (1, None),
        (0, Some(0)),
        (9, None),
        (1, Some(1)),
        (-1, None),
    ] {
        bundle::select(&rules, &mut held, index);
        assert_eq!(held.extended().bundle_selection.0, expected);
    }
    bundle::select(&rules, &mut held, 1);
    assert!(bundle::write(&rules, &mut held, &entries[..1]));
    assert_eq!(held.extended().bundle_selection.0, None);
}

#[test]
fn selection_does_not_enter_item_nbt() {
    let s = Session::new(MenuKind::Player);
    let rules = s.data.rules();
    let mut bundle = stack(BUNDLE, 1);
    assert!(bundle::write(
        &rules,
        &mut bundle,
        &[stack(STONE, 3), stack(BUCKET, 2)]
    ));
    let before = mc_data::item_nbt::stack_to_fields(rules.items, &bundle).unwrap();
    bundle::select(&rules, &mut bundle, 1);
    assert_eq!(
        mc_data::item_nbt::stack_to_fields(rules.items, &bundle).unwrap(),
        before
    );
}

#[test]
fn held_bundle_drops_stacks_then_repeats_without_losing_custody() {
    use crate::boundary::{ConnectionId, Hand};
    use crate::commands::GameMode;
    use crate::player::PlayerState;

    let s = Session::new(MenuKind::Player);
    let rules = s.data.rules();
    for (mode, hand, slot) in [
        (GameMode::Survival, Hand::Main, 0),
        (GameMode::Creative, Hand::Off, super::OFF_HAND),
    ] {
        let mut player = PlayerState::new(ConnectionId(1), mode);
        let mut held = stack(BUNDLE, 1);
        assert!(bundle::write(
            &rules,
            &mut held,
            &[stack(STONE, 5), stack(BUCKET, 2)]
        ));
        player.inventory.slots[slot] = held;
        player.use_item(&rules, hand);
        let first = player.tick_item_use(&rules);
        assert_eq!(first.dropped, vec![stack(STONE, 5)]);
        assert_eq!(
            bundle::read(&rules, &player.inventory.slots[slot]).unwrap(),
            vec![stack(BUCKET, 2)]
        );
        for _ in 0..11 {
            assert!(player.tick_item_use(&rules).dropped.is_empty());
        }
        let second = player.tick_item_use(&rules);
        assert_eq!(second.dropped, vec![stack(BUCKET, 2)]);
        assert!(
            bundle::read(&rules, &player.inventory.slots[slot])
                .unwrap()
                .is_empty()
        );
        for _ in 0..200 {
            assert!(player.tick_item_use(&rules).dropped.is_empty());
        }
        assert_eq!(player.inventory.slots[slot], stack(BUNDLE, 1));
    }
}

#[test]
fn cursor_stack_clicked_on_a_bundle_goes_inside_and_the_bundle_stays() {
    let mut s = Session::new(MenuKind::Player);
    s.put(9, stack(BUNDLE, 1));
    s.menu.carried = stack(STONE, 10);
    s.click(9, 0, ClickMode::Pickup);
    assert!(s.menu.carried.is_empty(), "cobblestone left the cursor");
    assert_eq!(
        s.slot(9).item_id,
        BUNDLE,
        "bundle did not swap with the cursor"
    );
    assert_eq!(contents(&s, &s.slot(9)), vec![stack(STONE, 10)]);
}

#[test]
fn bundle_on_the_cursor_clicked_on_a_stack_takes_it_in() {
    let mut s = Session::new(MenuKind::Player);
    s.put(9, stack(STONE, 10));
    s.menu.carried = stack(BUNDLE, 1);
    s.click(9, 0, ClickMode::Pickup);
    assert!(s.slot(9).is_empty());
    assert_eq!(s.menu.carried.item_id, BUNDLE);
    assert_eq!(contents(&s, &s.menu.carried), vec![stack(STONE, 10)]);
}

#[test]
fn weight_caps_insertion_and_merges_equal_stacks() {
    let mut s = Session::new(MenuKind::Player);
    s.put(9, stack(BUNDLE, 1));
    s.menu.carried = stack(STONE, 50);
    s.click(9, 0, ClickMode::Pickup);
    s.menu.carried = stack(STONE, 30);
    s.click(9, 0, ClickMode::Pickup);
    // 64 stone fill a bundle (1/64 each); the surplus stays on the cursor.
    assert_eq!(s.menu.carried, stack(STONE, 16));
    assert_eq!(contents(&s, &s.slot(9)), vec![stack(STONE, 64)]);
    // A full bundle refuses even a single non-stackable bucket (weight 1).
    s.menu.carried = stack(WATER_BUCKET, 1);
    s.click(9, 0, ClickMode::Pickup);
    assert_eq!(s.menu.carried, stack(WATER_BUCKET, 1));
    assert_eq!(contents(&s, &s.slot(9)), vec![stack(STONE, 64)]);
}

#[test]
fn right_click_with_empty_cursor_takes_the_newest_stack_out() {
    let mut s = Session::new(MenuKind::Player);
    s.put(9, stack(BUNDLE, 1));
    s.menu.carried = stack(STONE, 10);
    s.click(9, 0, ClickMode::Pickup);
    s.menu.carried = stack(BUCKET, 2);
    s.click(9, 0, ClickMode::Pickup);
    s.click(9, 1, ClickMode::Pickup);
    assert_eq!(s.menu.carried, stack(BUCKET, 2));
    assert_eq!(contents(&s, &s.slot(9)), vec![stack(STONE, 10)]);
    s.menu.carried = ItemStack::EMPTY;
    s.click(9, 1, ClickMode::Pickup);
    assert_eq!(s.menu.carried, stack(STONE, 10));
    assert!(contents(&s, &s.slot(9)).is_empty());
    assert!(
        s.slot(9).extended().preserved_components.is_empty(),
        "an emptied bundle equals a fresh one"
    );
}

#[test]
fn bundle_on_the_cursor_right_clicked_on_an_empty_slot_places_the_newest_stack() {
    let mut s = Session::new(MenuKind::Player);
    let mut full = stack(BUNDLE, 1);
    let rules = s.data.rules();
    assert!(bundle::write(&rules, &mut full, &[stack(STONE, 5)]));
    s.menu.carried = full;
    s.click(10, 1, ClickMode::Pickup);
    assert_eq!(s.slot(10), stack(STONE, 5));
    assert!(contents(&s, &s.menu.carried).is_empty());
}

#[test]
fn left_click_with_an_empty_cursor_still_picks_the_bundle_up() {
    let mut s = Session::new(MenuKind::Player);
    s.put(9, stack(BUNDLE, 1));
    s.click(9, 0, ClickMode::Pickup);
    assert_eq!(
        s.menu.carried.item_id, BUNDLE,
        "left click with an empty cursor picks it up"
    );
    assert!(s.slot(9).is_empty());
}

#[test]
fn nesting_beyond_what_the_wire_encodes_is_refused_like_a_full_bundle() {
    let s = Session::new(MenuKind::Player);
    let rules = s.data.rules();
    // Each pass wraps the previous bundle in a fresh one: levels of non-empty bundles.
    let mut inner = stack(BUNDLE, 1);
    for level in 1..=8 {
        let mut outer = stack(BUNDLE, 1);
        let contents = bundle::read(&rules, &outer).unwrap();
        assert!(
            bundle::room(&rules, &contents, &inner) > 0,
            "level {level} is within the wire depth"
        );
        let mut contents = contents;
        assert_eq!(bundle::insert(&rules, &mut contents, &mut inner), 1);
        assert!(bundle::write(&rules, &mut outer, &contents));
        inner = outer;
    }
    // `inner` now has 8 nested non-empty levels: one more would not encode.
    let contents = bundle::read(&rules, &stack(BUNDLE, 1)).unwrap();
    assert_eq!(bundle::room(&rules, &contents, &inner), 0);
}

#[test]
fn clicking_a_too_deep_bundle_changes_nothing() {
    let mut s = Session::new(MenuKind::Player);
    let rules = s.data.rules();
    let mut inner = stack(BUNDLE, 1);
    for _ in 0..8 {
        let mut outer = stack(BUNDLE, 1);
        let mut contents = Vec::new();
        bundle::insert(&rules, &mut contents, &mut inner);
        bundle::write(&rules, &mut outer, &contents);
        inner = outer;
    }
    s.put(9, inner.clone());
    s.menu.carried = stack(BUNDLE, 1);
    s.click(9, 0, ClickMode::Pickup);
    // The empty cursor bundle refuses to take the too-deep bundle, and nothing moved.
    assert_eq!(s.slot(9), inner);
    assert_eq!(s.menu.carried, stack(BUNDLE, 1));
}
