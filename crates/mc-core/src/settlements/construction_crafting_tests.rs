use super::{can_craft, can_fund, craft, missing_base};
use crate::settlements::allocator::tests::test_state;
use crate::settlements::inventory;
use crate::tick::TickState;
use mc_data::ItemStack;
use mc_script::ScriptInventoryFence;

fn state() -> TickState {
    test_state()
}

fn item(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap()
}

fn image(slots: Vec<ItemStack>) -> inventory::Image {
    inventory::Image {
        endpoint: inventory::Endpoint::Container {
            dimension: "minecraft:overworld".into(),
            position: [0, 0, 0],
        },
        slots,
        fence: ScriptInventoryFence::try_new(
            1,
            "0000000000000000000000000000000000000000000000000000000000000000".into(),
        )
        .unwrap(),
    }
}

fn stock(item: u32, count: i32) -> ItemStack {
    ItemStack::new(item, count)
}

fn sum(slots: &[Vec<ItemStack>], item: u32) -> i32 {
    let mut total = 0;
    for image in slots {
        for stack in image {
            if !stack.is_empty() && stack.item_id == item {
                total += stack.count.max(0);
            }
        }
    }
    total
}

fn chest(slots: Vec<ItemStack>) -> Vec<inventory::Image> {
    let mut padded = slots;
    padded.resize(27, ItemStack::EMPTY);
    vec![image(padded)]
}

#[test]
fn stone_stairs_are_not_reserved_as_a_wood_family() {
    let state = state();
    let stairs = item(&state, "minecraft:cobblestone_stairs");
    let cobble = item(&state, "minecraft:cobblestone");
    let oak = item(&state, "minecraft:oak_stairs");
    let images = chest(vec![stock(cobble, 6)]);
    let post: Vec<_> = images.iter().map(|image| image.slots.clone()).collect();
    assert!(super::wood_specific(&state, oak));
    assert!(!super::wood_specific(&state, stairs));
    assert!(
        super::reserve_wood(&state, &[(stairs, 4)], &post)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn logs_to_door_exact_yield_and_surplus() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let planks = item(&state, "minecraft:oak_planks");
    let door = item(&state, "minecraft:oak_door");
    // Two logs -> 8 planks; one door batch consumes 6 planks and yields 3.
    let images = chest(vec![stock(log, 2)]);
    let post = craft(&state, door, &images).expect("two logs cover one door");
    assert_eq!(sum(&post, door), 3);
    assert_eq!(sum(&post, planks), 2);
    assert_eq!(sum(&post, log), 0);
    assert!(can_craft(&state, door, &images));
}

#[test]
fn wool_bed_chain() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let red_bed = item(&state, "minecraft:red_bed");
    let wool = item(&state, "minecraft:red_wool");
    // Bed needs 3 wool + 3 planks; one log covers the planks leg.
    let images = chest(vec![stock(wool, 3), stock(log, 1)]);
    let post = craft(&state, red_bed, &images).expect("wool plus log covers a bed");
    assert_eq!(sum(&post, red_bed), 1);
    assert_eq!(sum(&post, wool), 0);
}

#[test]
fn missing_ingredient_refuses_atomically() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let door = item(&state, "minecraft:oak_door");
    let planks = item(&state, "minecraft:oak_planks");
    // One log -> 4 planks, but a door needs 6: must refuse with None.
    let images = chest(vec![stock(log, 1)]);
    assert!(craft(&state, door, &images).is_none());
    assert!(!can_craft(&state, door, &images));
    // Pure stock gap: the raw log is not planks, so all six are still missing.
    assert_eq!(
        missing_base(&state, &[(planks, 6)], &images),
        vec![(planks, 6)]
    );
}

#[test]
fn no_room_refuses() {
    let state = state();
    let stone = item(&state, "minecraft:stone");
    let door = item(&state, "minecraft:oak_door");
    let log = item(&state, "minecraft:oak_log");
    // Every slot full of stone: consumption impossible (logs stuck) and no
    // empty/mergeable room for intermediates either.
    let full: Vec<ItemStack> = (0..27).map(|_| stock(stone, 64)).collect();
    let mut images = chest(full);
    images[0].slots[0] = stock(log, 6);
    assert!(craft(&state, door, &images).is_none());
}

#[test]
fn existing_item_returns_unchanged() {
    let state = state();
    let door = item(&state, "minecraft:oak_door");
    let images = chest(vec![stock(door, 1)]);
    let post = craft(&state, door, &images).expect("stocked item needs no craft");
    assert_eq!(post.len(), 1);
    assert_eq!(post[0].len(), images[0].slots.len());
    assert_eq!(sum(&post, door), 1);
}

#[test]
fn whole_upgrade_funding_never_double_spends_recipe_surplus_or_logs() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let planks = item(&state, "minecraft:oak_planks");
    let door = item(&state, "minecraft:oak_door");
    let images = chest(vec![stock(log, 2)]);
    assert!(can_fund(&state, &[(door, 3), (planks, 2)], &images));
    assert!(!can_fund(&state, &[(door, 3), (planks, 3)], &images));
    assert_eq!(
        images[0].slots[0],
        stock(log, 2),
        "funding proofs never mutate stock"
    );
}

#[test]
fn whole_storage_funding_accepts_real_raw_stock_across_many_cell_crafts() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let coal = item(&state, "minecraft:coal");
    let cobble = item(&state, "minecraft:cobblestone");
    let images = chest(vec![
        stock(log, 64),
        stock(log, 64),
        stock(log, 64),
        stock(log, 43),
        stock(coal, 24),
        stock(cobble, 64),
        stock(cobble, 38),
    ]);
    let materials = &crate::settlements::blueprints::get(&state.registry, "storage")
        .unwrap()
        .level(1)
        .unwrap()
        .materials;
    assert!(
        can_fund(&state, materials, &images),
        "real logs, coal and cobble cover Storage L1 through the same per-cell recipes"
    );
    assert_eq!(
        images[0].slots[0],
        stock(log, 64),
        "funding remains read-only"
    );
}

#[test]
fn house_l1_raw_materials_account_for_whole_recipe_batches() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let coal = item(&state, "minecraft:coal");
    let cobble = item(&state, "minecraft:cobblestone");
    let wool = item(&state, "minecraft:white_wool");
    let dirt = item(&state, "minecraft:dirt");
    let materials = &crate::settlements::blueprints::get(&state.registry, "house")
        .unwrap()
        .level(1)
        .unwrap()
        .materials;
    let mut images = chest(vec![
        stock(coal, 1),
        stock(cobble, 51),
        stock(dirt, 64),
        stock(log, 64),
        stock(log, 24),
        stock(wool, 3),
    ]);
    assert!(
        !can_fund(&state, materials, &images),
        "88 logs cannot cover whole batches"
    );
    images[0].slots[4].count += 1;
    assert!(
        can_fund(&state, materials, &images),
        "89 logs must fund the real house bill"
    );
}

#[test]
fn installed_recipe_index_matches_every_original_scan_in_order() {
    let root = std::env::var_os("KRASNOKAMEN_CONTENT_CACHE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla")
        });
    let registry = crate::registry::CoreRegistry::load(Some(&root))
        .expect("the full installed content cache is required for recipe equivalence");
    let index = &registry.construction_crafting;
    let mut item_ids: Vec<_> = registry.items.iter().map(|(_, item)| item).collect();
    if let Some(tags) = registry.tags.registries.get(mc_data::tags::ITEM_REGISTRY) {
        item_ids.extend(
            tags.values()
                .flatten()
                .filter_map(|&item| u32::try_from(item).ok()),
        );
    }
    item_ids.extend([0, u32::MAX]);
    item_ids.sort_unstable();
    item_ids.dedup();
    let scanned: Vec<_> = registry
        .recipes
        .iter()
        .enumerate()
        .filter(|(_, recipe)| super::is_grid(recipe))
        .filter_map(|(position, recipe)| {
            super::result_identity(&registry.items, &registry.item_facts, recipe)
                .map(|(item, count)| (position, item, count))
        })
        .collect();
    assert!(!scanned.is_empty());
    assert_eq!(
        index
            .recipes
            .iter()
            .map(|plan| (plan.recipe, plan.result, plan.count))
            .collect::<Vec<_>>(),
        scanned,
        "compiled rows retain every valid grid result, including duplicate outputs"
    );
    for (_, item) in registry.items.iter() {
        let expected: Vec<_> = scanned
            .iter()
            .filter(|(_, result, _)| *result == item)
            .map(|(position, _, count)| (*position, *count))
            .collect();
        let actual: Vec<_> = index
            .for_item(item)
            .iter()
            .map(|&candidate| {
                let plan = &index.recipes[candidate];
                (plan.recipe, plan.count)
            })
            .collect();
        assert_eq!(actual, expected, "result candidates differ for item {item}");
    }
    for plan in &index.recipes {
        let expected = super::expanded(&registry.recipes[plan.recipe]);
        let actual = plan.ingredients.as_ref().map(|ingredients| {
            ingredients
                .iter()
                .map(|&ingredient| index.ingredients[ingredient].ingredient.clone())
                .collect::<Vec<_>>()
        });
        assert_eq!(actual, expected, "ingredient cell order changed");
    }
    for plan in &index.ingredients {
        for &item in &item_ids {
            assert_eq!(
                plan.accepted_items.binary_search(&item).is_ok(),
                mc_data::recipes::ingredient_accepts_item(
                    &registry.items,
                    &registry.tags,
                    item,
                    &plan.ingredient,
                ),
                "compiled membership changed for item {item}: {:?}",
                plan.ingredient,
            );
        }
        let expected_items: Vec<_> = registry
            .items
            .iter()
            .filter_map(|(_, item)| {
                mc_data::recipes::ingredient_accepts_item(
                    &registry.items,
                    &registry.tags,
                    item,
                    &plan.ingredient,
                )
                .then_some(item)
            })
            .collect();
        assert_eq!(
            plan.items, expected_items,
            "extraction-source order changed"
        );
        let expected: Vec<_> = scanned
            .iter()
            .filter(|(_, item, _)| {
                mc_data::recipes::ingredient_accepts_item(
                    &registry.items,
                    &registry.tags,
                    *item,
                    &plan.ingredient,
                )
            })
            .map(|(position, _, _)| *position)
            .collect();
        let actual: Vec<_> = plan
            .recipes
            .iter()
            .map(|&candidate| index.recipes[candidate].recipe)
            .collect();
        assert_eq!(actual, expected, "generic ingredient recipe order changed");
    }
}

#[test]
fn funding_reuses_loaded_index_without_resolving_recipe_results() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let door = item(&state, "minecraft:oak_door");
    let planks = item(&state, "minecraft:oak_planks");
    let images = chest(vec![stock(log, 2)]);
    super::reset_scan_stats();
    assert_eq!(sum(&craft(&state, door, &images).unwrap(), door), 3);
    assert!(can_fund(&state, &[(door, 3), (planks, 2)], &images));
    let stats = super::scan_stats();
    assert!(
        stats.index_queries > 0,
        "runtime must consume compiled candidates"
    );
    assert_eq!(
        stats.index_builds, 0,
        "a funding query must not rebuild content"
    );
    assert_eq!(
        stats.result_resolutions, 0,
        "no per-query catalog identity scan"
    );
    assert!(stats.recipes_scanned < state.registry.recipes.len());
}

#[test]
fn recipe_index_rebuild_replaces_recipe_and_tag_candidates() {
    use mc_data::recipes::{Ingredient, IngredientAlternative, RecipeKind};
    use std::collections::BTreeMap;
    let id = |name: &str| mc_data::Identifier::parse(name).unwrap();
    let mut registry = crate::registry::CoreRegistry::load(None).unwrap();
    let oak = registry.items.id_of(&id("minecraft:oak_log")).unwrap();
    let birch = registry.items.id_of(&id("minecraft:birch_log")).unwrap();
    let planks = registry.items.id_of(&id("minecraft:oak_planks")).unwrap();
    let stone = registry.items.id_of(&id("minecraft:stone")).unwrap();
    let tag = id("test:construction_index");
    let mut recipe = mc_data::recipes::krasnokamen_required_recipes()
        .into_iter()
        .find(|recipe| recipe.result.item == id("minecraft:oak_planks"))
        .unwrap();
    recipe.id = id("minecraft:construction_index");
    recipe.kind = RecipeKind::Shapeless(mc_data::recipes::ShapelessRecipe {
        ingredients: vec![Ingredient {
            alternatives: vec![IngredientAlternative::Tag(tag.clone())],
        }],
    });
    registry.recipes = vec![recipe];
    registry.tags =
        std::sync::Arc::new(mc_data::tags::TagsData::from_registries(BTreeMap::from([
            (
                id(mc_data::tags::ITEM_REGISTRY),
                BTreeMap::from([(tag.clone(), vec![oak as i32, -1, i32::MAX])]),
            ),
        ])));
    registry.refresh_recipe_indexes();
    assert_eq!(registry.construction_crafting.ingredients[0].items, [oak]);
    assert_eq!(
        registry.construction_crafting.ingredients[0].accepted_items,
        [oak, i32::MAX as u32],
    );
    assert_eq!(registry.construction_crafting.for_item(planks).len(), 1);
    std::sync::Arc::make_mut(&mut registry.tags)
        .registries
        .get_mut(&id(mc_data::tags::ITEM_REGISTRY))
        .unwrap()
        .insert(tag, vec![birch as i32]);
    registry.recipes[0].result.item = id("minecraft:stone");
    registry.refresh_recipe_indexes();
    assert_eq!(registry.construction_crafting.ingredients[0].items, [birch]);
    assert!(registry.construction_crafting.for_item(planks).is_empty());
    assert_eq!(registry.construction_crafting.for_item(stone).len(), 1);
    registry.recipes.clear();
    registry.refresh_recipe_indexes();
    assert!(registry.construction_crafting.recipes.is_empty());
    assert!(registry.construction_crafting.ingredients.is_empty());
}

#[test]
fn recursive_failure_restores_each_slot_and_stack_metadata() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let door = item(&state, "minecraft:oak_door");
    let mut original = stock(log, 1);
    original.damage = Some(7);
    let images = chest(vec![original]);
    let mut post: Vec<_> = images.iter().map(|image| image.slots.clone()).collect();
    let before = post.clone();
    assert!(!super::ensure_units(
        &state,
        &mut post,
        door,
        3,
        &std::collections::BTreeMap::new(),
    ));
    assert_eq!(
        post, before,
        "failed recursive crafting is an exact rollback"
    );
    let mut work = super::Work {
        totals: super::SlotTotals::from_slots(&post),
        ..super::Work::default()
    };
    assert!(!super::ensure(&state, &mut post, &mut work, door, 3, 0));
    assert_eq!(post, before);
    for item in [log, door, item(&state, "minecraft:oak_planks")] {
        assert_eq!(work.totals.get(item), super::total(&post, item));
    }
}

#[test]
fn recursive_success_preserves_exact_surplus_slot_order() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let planks = item(&state, "minecraft:oak_planks");
    let door = item(&state, "minecraft:oak_door");
    let images = chest(vec![stock(log, 2)]);
    let post = craft(&state, door, &images).unwrap();
    assert_eq!(post[0][0], stock(planks, 2));
    assert_eq!(post[0][1], stock(door, 3));
    assert!(post[0][2..].iter().all(ItemStack::is_empty));
}

#[test]
fn impossible_raw_bill_is_rejected_before_recursive_boolean_proof() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let door = item(&state, "minecraft:oak_door");
    let images = chest(vec![stock(log, 2)]);
    let bill = [(door, 3), (log, 3)];
    let mut original = images.iter().map(|image| image.slots.clone()).collect();
    assert!(!super::simulate_bill(&state, &bill, &mut original, false, true).0);
    super::reset_scan_stats();
    assert!(!can_fund(&state, &bill, &images));
    assert_eq!(super::scan_stats().recipes_scanned, 0);
    assert_eq!(
        super::needed_gatherer(&state, &bill, &images),
        Some(crate::villager::VillagerProfession::Lumberjack)
    );
    assert!(can_fund(
        &state,
        &[(door, 3), (log, 1)],
        &chest(vec![stock(log, 3)])
    ));
}

#[test]
fn funding_memo_matches_every_input_class_by_value_and_retains_index_identity() {
    let mut state = state();
    let log = item(&state, "minecraft:oak_log");
    let door = item(&state, "minecraft:oak_door");
    let bill = [(door, 3)];
    let mut images = chest(vec![stock(log, 2)]);
    assert!(can_fund(&state, &bill, &images));
    super::reset_scan_stats();
    assert!(can_fund(&state, &bill, &images));
    assert_eq!(super::scan_stats().memo_hits, 1);
    assert_eq!(super::scan_stats().funding_calls, 0);

    images[0].slots[0].count = 1;
    super::reset_scan_stats();
    assert!(!can_fund(&state, &bill, &images));
    assert_eq!(
        super::scan_stats().memo_misses,
        1,
        "stock is part of the key"
    );
    images[0].slots[0].damage = Some(7);
    super::reset_scan_stats();
    assert!(!can_fund(&state, &bill, &images));
    assert_eq!(
        super::scan_stats().memo_misses,
        1,
        "components are part of the key"
    );
    images[0].slots.swap(0, 1);
    super::reset_scan_stats();
    assert!(!can_fund(&state, &bill, &images));
    assert_eq!(
        super::scan_stats().memo_misses,
        1,
        "physical slot order is part of the key"
    );
    let ordered = [(door, 3), (log, 1)];
    assert!(!can_fund(&state, &ordered, &images));
    super::reset_scan_stats();
    assert!(!can_fund(&state, &[(log, 1), (door, 3)], &images));
    assert_eq!(
        super::scan_stats().memo_misses,
        1,
        "ordered bills are not multisets"
    );

    let original = chest(vec![stock(log, 2)]);
    assert!(can_fund(&state, &bill, &original));
    let previous_index = std::sync::Arc::clone(&state.registry.construction_crafting);
    let registry = std::sync::Arc::get_mut(&mut state.registry).expect("fixture owns registry");
    registry
        .recipes
        .retain(|recipe| recipe.result.item.as_str() == "minecraft:oak_door");
    registry.refresh_recipe_indexes();
    assert!(!std::sync::Arc::ptr_eq(
        &previous_index,
        &registry.construction_crafting
    ));
    super::reset_scan_stats();
    assert!(
        !can_fund(&state, &bill, &original),
        "reloaded content cannot reuse a funded answer"
    );
    assert_eq!(super::scan_stats().memo_misses, 1);
    assert!(
        state
            .settlements
            .funding
            .lock()
            .unwrap()
            .entries
            .iter()
            .any(|entry| { std::sync::Arc::ptr_eq(&entry.index, &previous_index) }),
        "the memo retains strong ownership, not a reusable numeric address"
    );
}

#[test]
fn funding_memo_is_bounded_and_keeps_traced_proofs() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let door = item(&state, "minecraft:oak_door");
    let images = chest(vec![stock(log, 2)]);
    for count in 1..=17 {
        assert_eq!(can_fund(&state, &[(door, count)], &images), count <= 3);
    }
    assert_eq!(state.settlements.funding.lock().unwrap().entries.len(), 16);
    let evicted = {
        let memo = state.settlements.funding.lock().unwrap();
        (1..=17)
            .find(|&count| {
                !memo
                    .entries
                    .iter()
                    .any(|entry| entry.bill == [(door, count)])
            })
            .unwrap()
    };
    super::reset_scan_stats();
    assert_eq!(can_fund(&state, &[(door, evicted)], &images), evicted <= 3);
    assert_eq!(
        super::scan_stats().memo_misses,
        1,
        "one of 17 complete keys was evicted"
    );
    super::reset_scan_stats();
    for _ in 0..2 {
        assert_eq!(
            super::needed_gatherer(&state, &[(log, 3)], &images),
            Some(crate::villager::VillagerProfession::Lumberjack)
        );
    }
    assert_eq!(super::scan_stats().funding_calls, 2);
    assert_eq!(super::scan_stats().memo_hits, 0);
}

#[test]
fn wood_reservation_memo_preserves_exact_results_and_all_key_classes() {
    let mut state = state();
    let log = item(&state, "minecraft:oak_log");
    let planks = item(&state, "minecraft:oak_planks");
    let bill = [(planks, 4), (log, 1)];
    let images = chest(vec![stock(log, 2)]);
    assert!(can_fund(&state, &bill, &images));
    let mut post = vec![images[0].slots.clone()];
    let expected = super::reserve_strict_wood_uncached(&state, &bill, &post);
    assert_eq!(super::reserve_wood(&state, &bill, &post), expected);
    super::reset_scan_stats();
    assert_eq!(super::reserve_wood(&state, &bill, &post), expected);
    assert_eq!(
        super::scan_stats().wood_memo_hits,
        1,
        "probe kind separates bool from reservation"
    );
    assert_eq!(super::scan_stats().wood_memo_misses, 0);

    post[0][0].count = 1;
    assert!(!can_fund(&state, &bill, &chest(vec![stock(log, 1)])));
    super::reset_scan_stats();
    assert_eq!(
        super::reserve_wood(&state, &bill, &post),
        Err(Some(crate::villager::VillagerProfession::Lumberjack))
    );
    assert_eq!(super::scan_stats().wood_memo_misses, 1);
    assert_eq!(
        super::scan_stats().wood_memo_hits,
        0,
        "probe kind separates bool from reservation"
    );
    super::reset_scan_stats();
    assert_eq!(
        super::reserve_wood(&state, &bill, &post),
        Err(Some(crate::villager::VillagerProfession::Lumberjack))
    );
    assert_eq!(
        super::scan_stats().wood_memo_hits,
        1,
        "first missing profession is preserved"
    );
    post[0][0].damage = Some(9);
    super::reset_scan_stats();
    assert_eq!(
        super::reserve_wood(&state, &bill, &post),
        super::reserve_strict_wood_uncached(&state, &bill, &post)
    );
    assert_eq!(
        super::scan_stats().wood_memo_misses,
        1,
        "every component participates"
    );
    super::reset_scan_stats();
    let reversed = [(log, 1), (planks, 4)];
    assert_eq!(
        super::reserve_wood(&state, &reversed, &post),
        super::reserve_strict_wood_uncached(&state, &reversed, &post)
    );
    assert_eq!(
        super::scan_stats().wood_memo_misses,
        1,
        "sub-bill order participates"
    );

    post[0][0] = stock(log, 2);
    assert!(super::reserve_wood(&state, &bill, &post).is_ok());
    post[0][0].count = 1;
    post[0][0].damage = Some(9);
    let registry = std::sync::Arc::get_mut(&mut state.registry).expect("fixture owns registry");
    registry.recipes.clear();
    registry.refresh_recipe_indexes();
    super::reset_scan_stats();
    assert!(super::reserve_wood(&state, &bill, &post).is_err());
    assert_eq!(
        super::scan_stats().wood_memo_misses,
        1,
        "content/index identity participates"
    );
}

#[test]
fn house_bed_color_probes_share_the_canonical_wood_proof_within_sixteen_entries() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let coal = item(&state, "minecraft:coal");
    let cobble = item(&state, "minecraft:cobblestone");
    let wool = item(&state, "minecraft:white_wool");
    let dirt = item(&state, "minecraft:dirt");
    let white = item(&state, "minecraft:white_bed");
    let images = chest(vec![
        stock(log, 64),
        stock(log, 6),
        stock(coal, 1),
        stock(cobble, 51),
        stock(wool, 3),
        stock(dirt, 64),
    ]);
    let materials = &crate::settlements::blueprints::get(&state.registry, "house")
        .unwrap()
        .level(1)
        .unwrap()
        .materials;
    super::reset_scan_stats();
    for color in crate::blueprint::DYE_COLORS {
        let bed = item(&state, &format!("minecraft:{color}_bed"));
        let bill: Vec<_> = materials
            .iter()
            .map(|&(item, count)| (if item == white { bed } else { item }, count))
            .collect();
        assert_eq!(
            can_fund(&state, &bill, &images),
            super::funding(&state, &bill, &images, false).0,
        );
    }
    let stats = super::scan_stats();
    assert!(stats.unreachable_rejections > 0);
    assert!(
        stats.wood_memo_hits > 0,
        "bed colors share one wood sub-bill proof"
    );
    assert!(state.settlements.funding.lock().unwrap().entries.len() <= 16);
}

#[test]
fn slot_totals_preserve_saturation_unknown_ids_and_nested_rollback() {
    let mut post = vec![vec![
        stock(7, i32::MAX),
        stock(7, 4),
        stock(u32::MAX, 9),
        stock(0, 3),
        stock(7, -2),
        ItemStack::EMPTY,
    ]];
    let original = post.clone();
    let mut work = super::Work {
        totals: super::SlotTotals::from_slots(&post),
        ..super::Work::default()
    };
    for replacement in [stock(7, 0), stock(u32::MAX, 6)] {
        work.undo.push((0, 0, post[0][0].clone()));
        work.totals
            .replace(&post[0][0], replacement.item_id, replacement.count);
        post[0][0] = replacement;
        for item in [0, 7, u32::MAX, 11] {
            assert_eq!(work.totals.get(item), super::total(&post, item));
        }
    }
    for mark in [1, 0] {
        work.rollback(&mut post, mark);
        for item in [0, 7, u32::MAX, 11] {
            assert_eq!(work.totals.get(item), super::total(&post, item));
        }
    }
    assert_eq!(post, original);
}

#[test]
fn unavailable_recipe_dependencies_reject_without_changing_the_canonical_answer() {
    let state = state();
    let log = item(&state, "minecraft:oak_log");
    let wool = item(&state, "minecraft:white_wool");
    let red = item(&state, "minecraft:red_bed");
    let white = item(&state, "minecraft:white_bed");
    let mut images = chest(vec![stock(log, 8), stock(wool, 3)]);
    assert!(!super::funding(&state, &[(red, 1)], &images, false).0);
    super::reset_scan_stats();
    assert!(!can_fund(&state, &[(red, 1)], &images));
    assert_eq!(super::scan_stats().unreachable_rejections, 1);
    assert_eq!(super::scan_stats().funding_calls, 0);
    assert!(can_fund(&state, &[(white, 1)], &images));
    assert!(super::funding(&state, &[(white, 1)], &images, false).0);
    images[0].slots[0].item_model = Some(
        mc_data::Identifier::parse("test:custom_ingredient")
            .unwrap()
            .into(),
    );
    super::reset_scan_stats();
    assert!(!can_fund(&state, &[(red, 1)], &images));
    assert_eq!(super::scan_stats().unreachable_rejections, 0);
    assert_eq!(
        super::scan_stats().funding_calls,
        1,
        "custom models use the canonical matcher"
    );
}

#[test]
fn dependency_admission_is_conservative_for_cycles_depth_and_model_outputs() {
    let ingredient = |items: Vec<u32>| super::IngredientPlan {
        ingredient: mc_data::recipes::Ingredient {
            alternatives: Vec::new(),
        },
        recipes: Vec::new(),
        items: items.clone(),
        accepted_items: items,
    };
    let recipe = |result, ingredients| super::RecipePlan {
        recipe: 0,
        result,
        count: 1,
        ingredients: Some(ingredients),
    };
    let mut index = super::Index {
        recipes: vec![recipe(1, vec![0]), recipe(2, vec![1]), recipe(1, vec![2])],
        by_result: std::collections::BTreeMap::from([(1, vec![0, 2]), (2, vec![1])]),
        ingredients: vec![
            ingredient(vec![2]),
            ingredient(vec![1]),
            ingredient(vec![4]),
        ],
        has_model_outputs: false,
    };
    let images = vec![image(vec![stock(4, 1)])];
    assert!(!super::unreachable_requirement(
        &index,
        &[(1, 1), (2, 1)],
        &images
    ));
    assert!(
        !super::unreachable_requirement(&index, &[(1, 1)], &[]),
        "an unseeded cycle is possible, not a negative proof"
    );
    index.recipes[1].ingredients.as_mut().unwrap().push(3);
    index.ingredients.push(ingredient(vec![3]));
    assert!(super::unreachable_requirement(&index, &[(2, 1)], &images));
    assert!(
        !super::unreachable_requirement(&index, &[(1, 1)], &images),
        "a later alternative remains reachable after a failed cyclic alternative"
    );
    assert!(super::possibly_reachable(
        &index,
        3,
        &mut Default::default(),
        super::MAX_DEPTH + 1
    ));
    index.has_model_outputs = true;
    assert!(!super::unreachable_requirement(&index, &[(3, 1)], &[]));
}
