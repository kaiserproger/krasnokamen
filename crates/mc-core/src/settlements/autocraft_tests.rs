use super::*;
use crate::settlements::allocator::tests::{block, test_state, villager};
use crate::settlements::buildings::{self, Building, BuildingId};
use crate::settlements::{RecordImage, validate_record};
use crate::villager::VillagerProfession as Profession;
use mc_data::Identifier;
use mc_script::ScriptWorldBounds;
use std::sync::Arc;

const TABLE: [i32; 3] = [5, 201, 5];
const CHEST: [i32; 3] = [10, 201, 12];

struct Village {
    state: TickState,
    id: SettlementId,
    worker: Uuid,
    slot: crate::entities::EntitySlot,
    building: ScriptSettlementBuildingRef,
    stand_at: [i32; 3],
}

fn place(state: &mut TickState, at: [i32; 3], name: &str, props: &[(&str, &str)]) {
    let id = block(state, name, props);
    state.world.set_block(at[0], at[1], at[2], id);
    state.world.fold_overlay();
}

fn item(state: &TickState, name: &str, count: i32) -> ItemStack {
    let id = state
        .registry
        .items
        .id_of(&Identifier::parse(name).unwrap())
        .unwrap();
    ItemStack::new(id, count)
}

/// A town with a certified farmer workshop (crafting table inside), one bound
/// unemployed resident and one storage chest holding `stored`.
fn village(stored: &[(&str, i32)]) -> Village {
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
    let id = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "forge".into(),
            "owner".into(),
            [8, 201, 8],
            48,
        )
        .unwrap();
    place(&mut state, [4, 201, 4], "minecraft:composter", &[]);
    place(&mut state, TABLE, "minecraft:crafting_table", &[]);
    let bounds = ScriptWorldBounds::new([3, 200, 3], [7, 204, 7]).unwrap();
    let kind = BuildingKind::Workshop(Profession::Farmer);
    let building = buildings::certify(&mut state, id, kind, bounds).unwrap();
    let building = ScriptSettlementBuildingRef::new(
        "building".into(),
        building.0.to_string(),
        format!("solaris:{}", kind.name()),
    );
    let stand_at = production::stand_at(&state, TABLE, bounds).unwrap();
    let slot = villager(&mut state, [5.5, 201.0, 7.5]);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    let worker = state.entities.row(slot).uuid;
    place(&mut state, CHEST, "minecraft:chest", &[("type", "single")]);
    let mut slots = vec![ItemStack::EMPTY; 27];
    for (cell, (name, count)) in slots.iter_mut().zip(stored) {
        *cell = item(&state, name, *count);
    }
    state.block_entities.insert(
        (CHEST[0], CHEST[1], CHEST[2]),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
    state.settlements.get_mut(id).unwrap().bootstrap_storage = Some(CHEST);
    Village {
        state,
        id,
        worker,
        slot,
        building,
        stand_at,
    }
}

impl Village {
    fn jobs(&self) -> &std::collections::BTreeMap<String, production::Job> {
        &self
            .state
            .settlements
            .get(self.id)
            .unwrap()
            .economy
            .production
            .jobs
    }

    fn order(&self, job: &str, recipe: &str, count: u32) -> Order {
        Order {
            job: job.into(),
            worker: self.worker,
            recipe: recipe.into(),
            count,
            building: self.building.clone(),
            station: TABLE,
            stand_at: self.stand_at,
        }
    }

    /// The owner's autocraft request through the real admission path.
    fn submit(&mut self, job: &str, recipe: &str, count: u32) -> Result<Vec<Step>, Refusal> {
        let request = self.order(job, recipe, count);
        order(&mut self.state, self.id, request)
    }
    fn equip(&mut self, name: &str) {
        let stack = item(&self.state, name, 1);
        Arc::make_mut(&mut self.state.entities.row_mut(self.slot).appearance).equipment[0] =
            Some(stack);
    }

    /// Total of `name` in the worker's carry.
    fn carried(&self, name: &str) -> i32 {
        self.state
            .entities
            .row(self.slot)
            .villager
            .as_ref()
            .unwrap()
            .inventory
            .iter()
            .flatten()
            .filter(|stack| production::resource(&self.state, stack) == Some(name))
            .map(|stack| stack.count)
            .sum()
    }

    fn stored(&self, name: &str) -> i32 {
        self.stored_at(CHEST, name)
    }

    /// Total of `name` in the block entity at `at` (one chest half).
    fn stored_at(&self, at: [i32; 3], name: &str) -> i32 {
        self.state.block_entities[&(at[0], at[1], at[2])]
            .slots
            .iter()
            .filter(|stack| production::resource(&self.state, stack) == Some(name))
            .map(|stack| stack.count)
            .sum()
    }

    fn cancel(&mut self, job: &str) {
        let cancel = production::Request::Cancel { job: job.into() };
        economy::execute(
            &mut self.state,
            self.id,
            economy::Request::Production(cancel),
        )
        .unwrap();
    }

    /// The native economy tick with the worker standing on its live route
    /// target (bodies walk there in game; navigation is not under test).
    fn step(&mut self) {
        self.state.tick += 5;
        if let Some(target) = economy::target(&self.state, Some(self.id), self.worker) {
            let pos = [
                f64::from(target.x) + 0.5,
                f64::from(target.y),
                f64::from(target.z) + 0.5,
            ];
            self.state.entities.set_pos(self.slot, pos);
        }
        economy::tick(&mut self.state);
    }

    fn run_until(&mut self, job: &str) {
        for _ in 0..2_000 {
            if self.jobs()[job].done >= self.jobs()[job].count {
                return;
            }
            self.step();
        }
        panic!("{job} did not finish: {:?}", self.jobs());
    }

    /// Persist and reload the settlement image exactly like a restart does.
    fn restart(&mut self) {
        let record = self.state.settlements.get(self.id).unwrap().clone();
        let bytes = serde_json::to_vec(&record).unwrap();
        let restored: crate::settlements::SettlementRecord =
            serde_json::from_slice(&bytes).unwrap();
        validate_record(&restored).unwrap();
        self.state.settlements.apply_image(RecordImage {
            id: self.id,
            value: Some(restored),
        });
    }
}

#[test]
fn missing_wheat_is_crafted_from_stored_hay_before_bread_and_survives_restart() {
    let mut town = village(&[("minecraft:hay_block", 1)]);
    let steps = town.submit("bake", "solaris-settlements:bread", 3).unwrap();
    assert_eq!(describe(&steps), "open_hay x1 -> bread x3");
    assert_eq!(town.jobs()["bake-1"].after, None);
    assert_eq!(town.jobs()["bake"].after.as_deref(), Some("bake-1"));
    assert_eq!(town.jobs()["bake"].order.as_deref(), Some("bake"));
    // The gated final step neither routes nor protects carry yet.
    assert_eq!(production::target_for_job(&town.jobs()["bake"]), None);

    town.run_until("bake-1");
    assert_eq!(town.stored("minecraft:hay_block"), 0);
    // Released in the very tick its predecessor completed.
    assert_eq!(town.jobs()["bake"].after, None);
    assert_eq!(town.jobs()["bake"].done, 0);
    town.restart();

    town.run_until("bake");
    let jobs = town.jobs();
    assert_eq!(jobs["bake-1"].receipts.len(), 1);
    assert_eq!(jobs["bake"].receipts.len(), 3);
    assert_eq!(
        town.carried("minecraft:bread") + town.stored("minecraft:bread"),
        3
    );
    assert_eq!(town.carried("minecraft:wheat"), 0);
    assert_eq!(town.carried("minecraft:hay_block"), 0);
    let policy = &town
        .state
        .settlements
        .get(town.id)
        .unwrap()
        .economy
        .production;
    assert_eq!((policy.sequence, policy.completed), (2, 2));
    validate_record(town.state.settlements.get(town.id).unwrap()).unwrap();

    // A retried order id never re-plans or duplicates work.
    assert_eq!(
        town.submit("bake", "solaris-settlements:bread", 3),
        Err(Refusal::Exists("bake".into()))
    );
    assert_eq!(town.jobs().len(), 2);
    for _ in 0..100 {
        town.step();
    }
    assert_eq!(
        town.carried("minecraft:bread") + town.stored("minecraft:bread"),
        3
    );
}

#[test]
fn pickaxe_chain_makes_planks_then_sticks_then_pickaxe_with_exact_stock() {
    let mut town = village(&[("minecraft:oak_log", 2), ("minecraft:wooden_pickaxe", 1)]);
    town.equip("minecraft:wooden_axe");
    let steps = town
        .submit("pick", "solaris-settlements:wooden_pickaxe_ordinary", 1)
        .unwrap();
    // 3 planks for the pickaxe + 2 for one stick craft = 2 plank crafts; the
    // stored pickaxe is never salvaged for sticks.
    assert_eq!(
        describe(&steps),
        "oak_planks x2 -> stick x1 -> wooden_pickaxe_ordinary x1"
    );
    town.run_until("pick");
    assert_eq!(town.stored("minecraft:oak_log"), 0);
    assert_eq!(town.stored("minecraft:wooden_pickaxe"), 1);
    assert_eq!(town.carried("minecraft:wooden_pickaxe"), 1);
    assert_eq!(town.carried("minecraft:oak_planks"), 8 - 3 - 2);
    assert_eq!(town.carried("minecraft:stick"), 4 - 2);
    let axe = town.state.entities.row(town.slot).appearance.equipment[0]
        .clone()
        .unwrap();
    assert_eq!(axe.damage, Some(2));
    let jobs = town.jobs();
    assert_eq!(
        (
            jobs["pick-1"].receipts.len(),
            jobs["pick-2"].receipts.len(),
            jobs["pick"].receipts.len()
        ),
        (2, 1, 1)
    );
}

#[test]
fn missing_raw_material_cycle_and_tool_refuse_without_admitting_anything() {
    let mut town = village(&[("minecraft:oak_log", 1)]);
    let mut refused = |recipe: &str| town.submit("plan", recipe, 1).unwrap_err();
    assert_eq!(
        refused("solaris-settlements:sugar"),
        Refusal::Missing {
            item: "minecraft:sugar_cane".into(),
            count: 1
        }
    );
    let cycle = refused("solaris-settlements:hay_block");
    assert_eq!(
        cycle,
        Refusal::Cycle {
            item: "minecraft:hay_block".into(),
            count: 1,
            recipes: vec!["hay_block".into(), "open_hay".into(), "hay_block".into()],
        }
    );
    assert_eq!(
        cycle.text(Lang::En),
        "missing 1 minecraft:hay_block: its recipes form a cycle (hay_block -> open_hay -> hay_block)"
    );
    assert_eq!(
        refused("solaris-settlements:wooden_pickaxe_ordinary"),
        Refusal::Tool {
            recipe: "oak_planks".into(),
            tool: "minecraft:wooden_axe".into()
        }
    );
    assert_eq!(
        refused("solaris-settlements:nothing"),
        Refusal::UnknownRecipe("solaris-settlements:nothing".into())
    );
    assert!(town.jobs().is_empty());
    assert_eq!(town.stored("minecraft:oak_log"), 1);
}

#[test]
fn cancelling_any_step_stops_the_whole_order_and_admission_is_idempotent() {
    let mut town = village(&[("minecraft:hay_block", 1)]);
    let steps = town.submit("bake", "solaris-settlements:bread", 3).unwrap();
    let worker = town.worker;
    let again = move |steps: Vec<Step>| {
        economy::Request::Production(production::Request::Order {
            order: "bake".into(),
            worker,
            steps,
        })
    };
    let same = again(steps.clone());
    economy::execute(&mut town.state, town.id, same).unwrap();
    assert_eq!(town.jobs().len(), 2);
    let mut bigger = steps;
    bigger.last_mut().unwrap().count = 4;
    assert_eq!(
        economy::execute(&mut town.state, town.id, again(bigger)),
        Err(Failure::OperationConflict)
    );

    let cancel = production::Request::Cancel { job: "bake".into() };
    economy::execute(
        &mut town.state,
        town.id,
        economy::Request::Production(cancel),
    )
    .unwrap();
    let jobs = town.jobs();
    assert!(jobs["bake"].cancelled && jobs["bake-1"].cancelled);
    let policy = &town
        .state
        .settlements
        .get(town.id)
        .unwrap()
        .economy
        .production;
    assert_eq!(policy.cancelled, 2);
    assert!(!production::active(policy));
    assert_eq!(
        economy::target(&town.state, Some(town.id), town.worker),
        None
    );
    validate_record(town.state.settlements.get(town.id).unwrap()).unwrap();
}

#[test]
fn a_gate_on_a_missing_or_foreign_step_is_a_corrupt_image() {
    let mut town = village(&[("minecraft:hay_block", 1)]);
    town.submit("bake", "solaris-settlements:bread", 3).unwrap();
    let mut state = town
        .state
        .settlements
        .get(town.id)
        .unwrap()
        .economy
        .production
        .clone();
    production::validate(&state).unwrap();
    state.jobs.get_mut("bake").unwrap().after = Some("elsewhere".into());
    assert_eq!(
        production::validate(&state),
        Err("invalid production order step")
    );
    // An earlier step whose final order step is gone.
    state.jobs.remove("bake");
    assert_eq!(
        production::validate(&state),
        Err("invalid production order")
    );
}

#[test]
fn a_double_chest_is_one_inventory_so_the_missing_wheat_is_planned_from_hay() {
    // Audit scenario: one double chest, both halves storage points, holding
    // 3 wheat + 1 hay block; order 2 bread (6 wheat). Counting each half's
    // merged 54-slot view saw 6 wheat, omitted the hay and stalled.
    let mut town = village(&[("minecraft:wheat", 3)]);
    let right = [CHEST[0] + 1, CHEST[1], CHEST[2]];
    place(
        &mut town.state,
        CHEST,
        "minecraft:chest",
        &[("facing", "north"), ("type", "left")],
    );
    place(
        &mut town.state,
        right,
        "minecraft:chest",
        &[("facing", "north"), ("type", "right")],
    );
    let mut slots = vec![ItemStack::EMPTY; 27];
    slots[0] = item(&town.state, "minecraft:hay_block", 1);
    town.state.block_entities.insert(
        (right[0], right[1], right[2]),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
    let record = town.state.settlements.get_mut(town.id).unwrap();
    record.building_sequence += 1;
    let hut = BuildingId((town.id.0 << 32) | record.building_sequence);
    record.buildings.insert(
        hut,
        Building {
            id: hut,
            settlement: town.id,
            kind: BuildingKind::Storage,
            level: 1,
            bounds: ScriptWorldBounds::new(right, right).unwrap(),
            work_points: vec![right],
            revision: 1,
            natural_zone: false,
        },
    );
    assert_eq!(
        buildings::storage_points(&town.state, town.id),
        vec![CHEST, right]
    );
    assert_eq!(inventory::town_storage(&town.state, town.id).len(), 1);

    let steps = town.submit("bake", "solaris-settlements:bread", 2).unwrap();
    assert_eq!(describe(&steps), "open_hay x1 -> bread x2");
    town.run_until("bake");
    assert_eq!(
        town.carried("minecraft:bread")
            + town.stored("minecraft:bread")
            + town.stored_at(right, "minecraft:bread"),
        2
    );
    assert_eq!(town.stored_at(right, "minecraft:hay_block"), 0);
    // 3 stored + 9 opened - 6 baked, every unit accounted for.
    let wheat = town.carried("minecraft:wheat")
        + town.stored("minecraft:wheat")
        + town.stored_at(right, "minecraft:wheat");
    assert_eq!(wheat, 6);
}

#[test]
fn a_long_chain_puts_its_intermediate_away_when_the_carry_fills_and_finishes() {
    // Audit scenario: open_hay x57 -> bread x171. 57 hay open into 513 wheat,
    // more than the eight-slot carry holds, long before the dependency is
    // done. The worker walks surplus wheat into town storage instead of
    // wedging; the product then fetches it back portion by portion.
    let mut town = village(&[("minecraft:hay_block", 57)]);
    let steps = town
        .submit("bake", "solaris-settlements:bread", 171)
        .unwrap();
    assert_eq!(describe(&steps), "open_hay x57 -> bread x171");
    let mut stored_peak = 0;
    let mut reasons = std::collections::BTreeSet::new();
    for _ in 0..20_000 {
        if town.jobs()["bake"].done >= 171 {
            break;
        }
        town.step();
        stored_peak = stored_peak.max(town.stored("minecraft:wheat"));
        reasons.extend(town.jobs().values().filter_map(|job| job.paused.clone()));
    }
    let jobs = town.jobs();
    assert_eq!(
        (jobs["bake-1"].done, jobs["bake-1"].receipts.len()),
        (57, 57),
        "{reasons:?}"
    );
    assert_eq!(
        (jobs["bake"].done, jobs["bake"].receipts.len()),
        (171, 171),
        "{reasons:?}"
    );
    assert!(stored_peak > 0, "the full carry never put wheat away");
    assert!(reasons.contains("awaiting storage reach"), "{reasons:?}");
    assert!(
        !reasons.contains("no storage") && !reasons.contains("missing input"),
        "{reasons:?}"
    );
    // Nothing dropped, nothing fabricated.
    assert_eq!(
        town.carried("minecraft:bread") + town.stored("minecraft:bread"),
        171
    );
    assert_eq!(
        town.carried("minecraft:wheat") + town.stored("minecraft:wheat"),
        0
    );
    assert_eq!(
        town.carried("minecraft:hay_block") + town.stored("minecraft:hay_block"),
        0
    );
    validate_record(town.state.settlements.get(town.id).unwrap()).unwrap();
}

#[test]
fn cancelling_a_finished_dependency_cancels_the_unfinished_rest_of_its_order() {
    let mut town = village(&[("minecraft:hay_block", 1)]);
    town.submit("bake", "solaris-settlements:bread", 3).unwrap();
    town.run_until("bake-1");
    assert_eq!(town.jobs()["bake"].done, 0);

    town.cancel("bake-1");
    let jobs = town.jobs();
    assert!(jobs["bake"].cancelled);
    // The finished step keeps its receipt; it is not rewritten as cancelled.
    assert!(!jobs["bake-1"].cancelled);
    assert_eq!(jobs["bake-1"].receipts.len(), 1);
    let policy = &town
        .state
        .settlements
        .get(town.id)
        .unwrap()
        .economy
        .production;
    assert_eq!(policy.cancelled, 1);
    assert!(!production::active(policy));
    assert_eq!(
        economy::target(&town.state, Some(town.id), town.worker),
        None
    );
    validate_record(town.state.settlements.get(town.id).unwrap()).unwrap();

    for _ in 0..100 {
        town.step();
    }
    assert_eq!(town.jobs()["bake"].done, 0);
    assert_eq!(town.carried("minecraft:bread"), 0);
    assert_eq!(town.carried("minecraft:wheat"), 9);
    // Naming either step again changes nothing.
    town.cancel("bake-1");
    town.cancel("bake");
    let policy = &town
        .state
        .settlements
        .get(town.id)
        .unwrap()
        .economy
        .production;
    assert_eq!(policy.cancelled, 1);
}

#[path = "autocraft_delivery_tests.rs"]
mod output_delivery;
