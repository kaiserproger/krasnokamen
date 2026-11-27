//! The embedded required item catalog that `CoreRegistry::load(None)` builds
//! for items, facts, tags and recipes, for tests that need real vanilla ids.
use std::sync::LazyLock;

use mc_data::item_components::ItemFactsTable;
use mc_data::items::ItemRegistry;
use mc_data::recipes::Recipe;
use mc_data::tags::TagsData;

use crate::ItemRules;

pub(crate) struct Builtin {
    pub(crate) items: ItemRegistry,
    pub(crate) item_facts: ItemFactsTable,
    pub(crate) tags: TagsData,
    pub(crate) recipes: Vec<Recipe>,
}

impl Builtin {
    pub(crate) fn item_rules(&self) -> ItemRules<'_> {
        ItemRules {
            items: &self.items,
            facts: &self.item_facts,
            tags: &self.tags,
            recipes: &self.recipes,
        }
    }
}

pub(crate) fn builtin() -> &'static Builtin {
    static BUILTIN: LazyLock<Builtin> = LazyLock::new(|| {
        let items = mc_data::items::krasnokamen_required_items();
        let report = mc_data::blocks::krasnokamen_required_blocks_report();
        Builtin {
            item_facts: mc_data::item_components::krasnokamen_required_item_facts(),
            tags: mc_data::tags::krasnokamen_required_client_tags(&items, &report),
            recipes: mc_data::recipes::krasnokamen_required_recipes(),
            items,
        }
    });
    &BUILTIN
}
