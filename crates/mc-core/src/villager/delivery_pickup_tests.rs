//! Real item entity pickup arbitration around an actual production obligation.
use super::*;

#[test]
fn matching_ground_pickup_is_deferred_while_unrelated_pickup_still_works() {
    use crate::settlements::{buildings::BuildingKind, commands::EconomyDomain, economy_commands};
    for paused in [false, true] {
        let (mut state, slot, mut v) =
            crate::villager::mining_tests::fixture(BuildingKind::Workshop(Profession::Farmer));
        crate::plugin_startup::apply(
            Arc::make_mut(&mut state.registry),
            &mc_plugin_host::DeploymentContribution::default(),
            None,
            &[],
        )
        .unwrap();
        let settlement = v.settlement.unwrap();
        let worker = state.entities.row(slot).uuid;
        let bread = item(&state, "minecraft:bread");
        let carrot = item(&state, "minecraft:carrot");
        v.inventory[0] = Some(ItemStack::new(bread, 3));
        v.inventory[1] = Some(ItemStack::new(item(&state, "minecraft:wheat"), 3));
        state.entities.row_mut(slot).villager = Some(v);
        state.entities.row_mut(slot).controls.no_ai = true;
        economy_commands::execute(
            &mut state,
            settlement,
            EconomyDomain::Production,
            &[
                "start".into(),
                "pickup-delivery".into(),
                worker.to_string(),
                "solaris-settlements:bread".into(),
                "1".into(),
                ((settlement.0 << 32) | 1).to_string(),
                "3,201,3".into(),
                "3,201,4".into(),
            ],
        )
        .unwrap();
        for _ in 0..500 {
            let _ = crate::tick::run_tick(&mut state, &[]);
            if state
                .settlements
                .get(settlement)
                .unwrap()
                .economy
                .production
                .jobs["pickup-delivery"]
                .done
                == 1
            {
                break;
            }
        }
        assert_eq!(
            state
                .settlements
                .get(settlement)
                .unwrap()
                .economy
                .production
                .jobs["pickup-delivery"]
                .done,
            1
        );
        if paused {
            economy_commands::execute(
                &mut state,
                settlement,
                EconomyDomain::Production,
                &["pause".into(), "pickup-delivery".into()],
            )
            .unwrap();
        }
        let pos = state.entities.pos(slot);
        let matching = crate::tick::spawn_item_entity(&mut state, pos, ItemStack::new(bread, 5));
        let unrelated = crate::tick::spawn_item_entity(&mut state, pos, ItemStack::new(carrot, 2));
        let mut named = ItemStack::new(bread, 1);
        named.custom_name = Some("gift".into());
        let different_components = crate::tick::spawn_item_entity(&mut state, pos, named.clone());
        let items: Vec<_> = [matching, unrelated, different_components]
            .into_iter()
            .map(|id| {
                let item_slot = state.entities.slot_of(id).unwrap();
                state.entities.body_mut(item_slot).pickup_delay = 0;
                item_slot
            })
            .collect();
        let mut food = FoodTable::default();
        food.refresh(&state.registry.items);
        let mut changes = ChangeSet::default();
        let mut v = state.entities.row_mut(slot).villager.take().unwrap();
        v.sleeping = false;
        pickup(&mut state, &food, &items, slot, &mut v, &mut changes);
        assert_eq!(
            v.inventory
                .iter()
                .flatten()
                .filter(|stack| stack.item_id == bread && stack.custom_name.is_none())
                .map(|stack| stack.count)
                .sum::<i32>(),
            4
        );
        assert!(v.inventory.iter().flatten().any(|stack| stack == &named));
        assert_eq!(
            v.inventory
                .iter()
                .flatten()
                .filter(|stack| stack.item_id == carrot)
                .map(|s| s.count)
                .sum::<i32>(),
            2
        );
        let matching_slot = state.entities.slot_of(matching).unwrap();
        assert!(state.entities.body(matching_slot).alive);
        assert_eq!(
            state
                .entities
                .row(matching_slot)
                .item
                .as_ref()
                .unwrap()
                .count,
            5
        );
        assert!(
            !changes.entity_events.iter().any(
                |event| matches!(event, EntityEvent::Pickup { item, .. } if *item == matching)
            )
        );
        state.entities.row_mut(slot).villager = Some(v);
        let debt = state
            .settlements
            .get(settlement)
            .unwrap()
            .economy
            .production
            .jobs["pickup-delivery"]
            .delivery
            .clone()
            .unwrap();
        assert!(!debt.invalidated);
        assert_eq!(debt.lots[0].retained, 3);
        assert_eq!(debt.lots[0].stack.count, 1);
        // The normal pickup was deferred, not discarded: cancellation releases
        // it without modifying or paying the old outstanding obligation.
        economy_commands::execute(
            &mut state,
            settlement,
            EconomyDomain::Production,
            &["cancel".into(), "pickup-delivery".into()],
        )
        .unwrap();
        let mut v = state.entities.row_mut(slot).villager.take().unwrap();
        pickup(
            &mut state,
            &food,
            &[matching_slot],
            slot,
            &mut v,
            &mut changes,
        );
        assert_eq!(
            v.inventory
                .iter()
                .flatten()
                .filter(|stack| stack.item_id == bread && stack.custom_name.is_none())
                .map(|stack| stack.count)
                .sum::<i32>(),
            9
        );
        state.entities.row_mut(slot).villager = Some(v);
        assert_eq!(
            state
                .settlements
                .get(settlement)
                .unwrap()
                .economy
                .production
                .jobs["pickup-delivery"]
                .delivery
                .as_ref(),
            Some(&debt)
        );
    }
}
