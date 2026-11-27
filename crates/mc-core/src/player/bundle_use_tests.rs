//! Held-bundle use: `PlayerState` drops bundle contents through the
//! mc-inventory bundle rules (moved from mc-inventory's bundle tests, which
//! cannot name the core player). Same three-item catalog as those tests: the
//! embedded `CoreRegistry::load(None)` catalog has no item facts for bundles,
//! so `use_item` would refuse them there.
use crate::commands::GameMode;
use crate::player::PlayerState;
use mc_data::item_components::{ItemFacts, ItemFactsTable};
use mc_data::items::{ItemRegistry, ItemReport};
use mc_data::tags::TagsData;
use mc_data::{Identifier, ItemStack};
use mc_domain::{ConnectionId, Hand};
use mc_inventory::{ItemRules, OFF_HAND, bundle};

const STONE: u32 = 3;
const BUCKET: u32 = 6;
const BUNDLE: u32 = 12;

struct Catalog {
    items: ItemRegistry,
    facts: ItemFactsTable,
    tags: TagsData,
}

impl Catalog {
    fn new() -> Self {
        let id = |name: &str| Identifier::parse(format!("minecraft:{name}")).unwrap();
        let entries = [
            (STONE, "stone", 64),
            (BUCKET, "bucket", 16),
            (BUNDLE, "bundle", 1),
        ];
        let items = ItemRegistry::from_report(
            &entries
                .iter()
                .map(|&(protocol_id, name, _)| ItemReport {
                    id: id(name),
                    protocol_id,
                })
                .collect::<Vec<_>>(),
        );
        let facts = ItemFactsTable::from_entries(entries.iter().map(|&(_, name, max)| {
            (
                id(name),
                ItemFacts {
                    max_stack_size: Some(max),
                    ..ItemFacts::default()
                },
            )
        }));
        Self {
            items,
            facts,
            tags: TagsData::default(),
        }
    }

    fn rules(&self) -> ItemRules<'_> {
        ItemRules {
            items: &self.items,
            facts: &self.facts,
            tags: &self.tags,
            recipes: &[],
        }
    }
}

fn stack(item: u32, count: i32) -> ItemStack {
    ItemStack::new(item, count)
}

#[test]
fn selected_held_bundle_drops_selected_then_default() {
    let catalog = Catalog::new();
    let rules = catalog.rules();
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
fn held_bundle_drops_stacks_then_repeats_without_losing_custody() {
    let catalog = Catalog::new();
    let rules = catalog.rules();
    for (mode, hand, slot) in [
        (GameMode::Survival, Hand::Main, 0),
        (GameMode::Creative, Hand::Off, OFF_HAND),
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
