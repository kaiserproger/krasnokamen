//! Founding households from ordinary births (D44): household-less adults and
//! the native courtship birth branch only. No test creates a household.
use super::*;
use crate::boundary::ChangeSet;
use crate::entities::EntitySlot;
use crate::settlements::allocator::PoiCache;
use crate::settlements::allocator::tests::{roofed_bed, test_state, villager};
use crate::settlements::households::HouseholdOrigin;
use crate::tick_budget::Consumer;
use mc_domain::EntityId;

struct Town {
    state: TickState,
    id: SettlementId,
    adults: [EntitySlot; 3],
    cache: PoiCache,
    bread: u32,
}

/// Three household-less adults with their own roofed beds and two spare
/// roofed beds for children.
fn town() -> Town {
    let mut state = test_state();
    state.tick = 300;
    state.day_time = 1000;
    let id = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "households".into(),
            "owner".into(),
            [8, 201, 8],
            48,
        )
        .unwrap();
    let adults = [[8.0, 201.0, 8.0], [8.5, 201.0, 8.0], [9.0, 201.0, 8.0]]
        .map(|pos| villager(&mut state, pos));
    for (slot, x) in adults.into_iter().zip([4, 8, 12]) {
        let home = roofed_bed(&mut state, x, 4);
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, id, entity).unwrap();
        state.entities.row_mut(slot).villager.as_mut().unwrap().home = Some(home);
    }
    for x in [4, 8] {
        roofed_bed(&mut state, x, 12);
    }
    state.world.fold_overlay();
    let bread = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:bread").unwrap())
        .unwrap();
    Town {
        state,
        id,
        adults,
        cache: PoiCache::default(),
        bread,
    }
}

/// One native courtship between two adults off cooldown, each holding a
/// breeding ration, that comes due this tick. Returns the child, if any.
fn birth(town: &mut Town, pair: [EntitySlot; 2]) -> Option<EntityId> {
    let ids = pair.map(|slot| town.state.entities.body(slot).id);
    let started = town.state.tick - 300;
    for (slot, partner) in pair.into_iter().zip([ids[1], ids[0]]) {
        let v = town.state.entities.row_mut(slot).villager.as_mut().unwrap();
        v.age = 0;
        v.inventory[0] = Some(mc_data::ItemStack::new(town.bread, 3));
        v.partner = Some(partner);
        v.courtship_until = Some(town.state.tick);
        v.courtship_started_tick = Some(started);
    }
    town.cache.refresh(&town.state);
    town.state.tick_budget.reset(town.state.tick);
    for dimension in crate::dimensions::Dimension::ALL {
        for consumer in [Consumer::Allocator, Consumer::Brain] {
            if dimension != town.state.dimension || consumer != Consumer::Brain {
                town.state.tick_budget.finish(dimension, consumer);
            }
        }
    }
    let mut changes = ChangeSet::default();
    crate::villager::brain::courtship(&mut town.state, &mut town.cache, &mut changes);
    crate::settlements::finish(&mut town.state);
    town.state.tick += 1;
    match changes.entities_spawned[..] {
        [] => None,
        [child] => Some(child),
        _ => panic!("one courtship spawned {:?}", changes.entities_spawned),
    }
}

fn household_of(state: &TickState, entity: EntityId) -> Option<String> {
    let slot = state.entities.slot_of(entity).unwrap();
    state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .household
        .clone()
}

fn eligible(town: &Town) -> Vec<String> {
    let record = town.state.settlements.get(town.id).unwrap();
    record
        .households
        .keys()
        .filter(|household| household_ready(&town.state, town.id, household, false))
        .cloned()
        .collect()
}

fn sorted(mut entities: Vec<EntityId>) -> Vec<EntityId> {
    entities.sort_unstable_by_key(|entity| entity.0);
    entities
}

/// Spain seed 100: after the first family's second birth with a newcomer every
/// adult shared household `1-1`, so no two-or-three-member household was left
/// for daughter founding. A household-less co-parent and a member of a family
/// of three now found their own household instead.
#[test]
fn a_household_less_co_parent_founds_an_eligible_household_by_ordinary_birth() {
    let mut town = town();
    let [a, b, c] = town.adults;
    let [a_id, b_id, c_id] = town.adults.map(|slot| town.state.entities.body(slot).id);
    let first = birth(&mut town, [a, b]).expect("first native birth");
    let family = household_of(&town.state, first).expect("first birth adopts its parents");
    assert_eq!(town.state.settlements.household_size(&family), 3);
    let second = birth(&mut town, [a, c]).expect("second native birth");
    assert_eq!(
        eligible(&town).len(),
        1,
        "exactly one whole household may lead an expedition: {:?}",
        town.state.settlements.get(town.id).unwrap().households
    );
    let split = eligible(&town).remove(0);
    assert_ne!(split, family);
    assert_eq!(
        sorted(crate::settlements::households::members(
            &town.state,
            town.id,
            &split
        )),
        sorted(vec![a_id, c_id, second])
    );
    assert_eq!(
        town.state.settlements.get(town.id).unwrap().households[&split].origin,
        HouseholdOrigin::Split
    );
    assert_eq!(
        sorted(crate::settlements::households::members(
            &town.state,
            town.id,
            &family
        )),
        sorted(vec![b_id, first])
    );
    assert_eq!(town.state.settlements.household_size(&split), 3);
    assert_eq!(town.state.settlements.household_size(&family), 2);
    assert_eq!(town.state.settlements.member_count(town.id), 5);
    assert!(crate::settlements::validate_memberships(&town.state).is_ok());
}

/// The household an expedition holds keeps its members: the courtship that
/// would move one of them out refuses before food, spawn or membership change.
#[test]
fn an_expedition_household_member_refuses_a_birth_that_would_split_it() {
    let mut town = town();
    let [a, b, c] = town.adults;
    let first = birth(&mut town, [a, b]).expect("first native birth");
    let family = household_of(&town.state, first).unwrap();
    let record = town.state.settlements.get_mut(town.id).unwrap();
    record.founding = Some(FoundingExpedition {
        destination: [120, 201, 8],
        household: family.clone(),
    });
    let revision = record.revision;
    assert_eq!(birth(&mut town, [a, c]), None);
    for slot in [a, c] {
        let v = town.state.entities.row(slot).villager.as_ref().unwrap();
        assert_eq!(v.inventory[0].as_ref().map(|stack| stack.count), Some(3));
        assert!(v.partner.is_none(), "the refused courtship is over");
    }
    assert_eq!(
        household_of(&town.state, town.state.entities.body(c).id),
        None
    );
    assert_eq!(town.state.settlements.household_size(&family), 3);
    let record = town.state.settlements.get(town.id).unwrap();
    assert_eq!(record.revision, revision);
    assert_eq!(record.households.len(), 1);
    assert_eq!(
        record.founding.as_ref().map(|founding| &founding.household),
        Some(&family)
    );
}
