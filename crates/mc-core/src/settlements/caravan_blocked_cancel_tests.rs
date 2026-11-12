//! Native pickup -> real failure -> explicit policy-only cancellation.
use super::*;
use crate::settlements::{buildings, economy, production};

#[derive(Clone, Copy, Debug)]
enum Cause {
    StaleDestination,
    RouteBudget,
}

fn native(f: &mut Fixture, request: Request) {
    economy::execute(
        &mut f.state,
        f.settlement,
        economy::Request::Caravan(request),
    )
    .unwrap();
}

fn conflict_destination(f: &mut Fixture) {
    let before = load(&f.state, &container_endpoint(&f.destination)).unwrap();
    let mut after = before.slots.clone();
    let stone = f
        .state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    after[0] = ItemStack::new(stone, 1);
    commit(&mut f.state, vec![Change { before, after }]).unwrap();
}

fn begin(f: &mut Fixture, cause: Cause) {
    stock(&mut f.state, SRC_CHEST, "minecraft:bread", 3);
    let mut request = open(f, "blocked-custody", 3, None);
    if let Request::Open(open) = &mut request
        && matches!(cause, Cause::RouteBudget)
    {
        open.max_ticks = 2;
    }
    native(f, request);
    teleport(&mut f.state, f.carrier, SRC_DOOR);
}

fn after_pickup(f: &mut Fixture, cause: Cause) {
    assert!(
        f.state
            .settlements
            .get(f.settlement)
            .unwrap()
            .economy
            .caravan
            .shipments["blocked-custody"]
            .loaded
    );
    match cause {
        Cause::StaleDestination => {
            conflict_destination(f);
            teleport(&mut f.state, f.carrier, DST_DOOR);
        }
        Cause::RouteBudget => {} // Normal successive ticks exhaust the admitted two-tick budget.
    }
}

#[derive(Debug, PartialEq)]
struct Physical {
    carry: Vec<ItemStack>,
    carry_fence: ScriptInventoryFence,
    source: Vec<ItemStack>,
    source_fence: ScriptInventoryFence,
    destination: Vec<ItemStack>,
    destination_fence: ScriptInventoryFence,
}

fn physical(f: &Fixture) -> Physical {
    let carry = load(&f.state, &carry_endpoint(f.carrier_uuid)).unwrap();
    let source = load(&f.state, &container_endpoint(&f.source)).unwrap();
    let destination = load(&f.state, &container_endpoint(&f.destination)).unwrap();
    Physical {
        carry: carry.slots,
        carry_fence: carry.fence,
        source: source.slots,
        source_fence: source.fence,
        destination: destination.slots,
        destination_fence: destination.fence,
    }
}

fn production_after_cancel(f: &mut Fixture) {
    let table = block(&f.state, "minecraft:crafting_table", &[]);
    f.state.world.set_block(8, 201, 20, table);
    f.state.world.fold_overlay();
    let sequence = f
        .state
        .settlements
        .get(f.settlement)
        .unwrap()
        .building_sequence
        .checked_add(1)
        .unwrap();
    assert!(sequence <= u64::from(u32::MAX));
    let bid = buildings::BuildingId((f.settlement.0 << 32) | sequence);
    let kind = buildings::BuildingKind::Workshop(crate::villager::VillagerProfession::Farmer);
    let record = f.state.settlements.get_mut(f.settlement).unwrap();
    assert!(!record.buildings.contains_key(&bid));
    record.building_sequence = sequence;
    assert!(
        record
            .buildings
            .insert(
                bid,
                buildings::Building {
                    id: bid,
                    settlement: f.settlement,
                    kind,
                    level: 1,
                    bounds: mc_script::ScriptWorldBounds::new([7, 200, 19], [10, 204, 23]).unwrap(),
                    work_points: vec![[8, 201, 20]],
                    revision: 1,
                    natural_zone: false,
                }
            )
            .is_none()
    );
    // Domain admission alone does not validate durable record invariants.
    // The same full validator used by recovery must accept this test fixture
    // before its policy can enter a WAL frame.
    crate::settlements::validate_record(record)
        .expect("post-cancel workshop must be a durable-valid fixture");
    let building = mc_script::ScriptSettlementBuildingRef::new(
        "building".into(),
        bid.0.to_string(),
        format!("krasnokamen:{}", kind.name()),
    );
    economy::execute(
        &mut f.state,
        f.settlement,
        economy::Request::Production(production::Request::Start {
            job: "after-blocked-cancel".into(),
            worker: f.carrier_uuid,
            recipe: "krasnokamen-settlements:bread".into(),
            count: 1,
            station: [8, 201, 20],
            stand_at: [8, 201, 21],
            building,
        }),
    )
    .unwrap();
}

#[test]
fn blocked_loaded_can_cancel_without_moving_cargo_or_rewriting_history() {
    for cause in [Cause::StaleDestination, Cause::RouteBudget] {
        let mut f = fixture();
        begin(&mut f, cause);
        f.state.tick += 1;
        economy::tick(&mut f.state);
        after_pickup(&mut f, cause);
        for _ in 0..8 {
            f.state.tick += 1;
            economy::tick(&mut f.state);
            if f.state
                .settlements
                .get(f.settlement)
                .unwrap()
                .economy
                .caravan
                .shipments["blocked-custody"]
                .status
                == Status::Blocked
            {
                break;
            }
        }
        let shipment = f
            .state
            .settlements
            .get(f.settlement)
            .unwrap()
            .economy
            .caravan
            .shipments["blocked-custody"]
            .clone();
        assert_eq!(shipment.status, Status::Blocked);
        assert!(shipment.loaded && shipment.load.is_some());
        assert!(shipment.unload.is_none() && shipment.delivery.is_none());
        let reason = shipment.blocked.clone().unwrap();
        match cause {
            Cause::StaleDestination => assert_eq!(reason, Failure::StaleRevision.as_str()),
            Cause::RouteBudget => assert_eq!(reason, "finite route tick budget exhausted"),
        }
        assert!(economy::caravan_reserved(
            &f.state,
            Some(f.settlement),
            f.carrier_uuid
        ));
        let held = physical(&f);
        native(
            &mut f,
            Request::Cancel {
                id: "blocked-custody".into(),
            },
        );
        assert_eq!(
            physical(&f),
            held,
            "cancel must not refund, unload, clear cargo or change fences"
        );
        let mut expected = shipment;
        expected.status = Status::Cancelled;
        assert_eq!(
            f.state
                .settlements
                .get(f.settlement)
                .unwrap()
                .economy
                .caravan
                .shipments["blocked-custody"],
            expected
        );
        assert!(!economy::caravan_reserved(
            &f.state,
            Some(f.settlement),
            f.carrier_uuid
        ));
        assert!(!delivered(
            &f.state
                .settlements
                .get(f.settlement)
                .unwrap()
                .economy
                .caravan,
            "blocked-custody"
        ));
        validate(
            &f.state
                .settlements
                .get(f.settlement)
                .unwrap()
                .economy
                .caravan,
        )
        .unwrap();
        native(
            &mut f,
            Request::Cancel {
                id: "blocked-custody".into(),
            },
        );
        assert_eq!(physical(&f), held);
        production_after_cancel(&mut f);
        assert_eq!(
            physical(&f),
            held,
            "new job admission itself must not pay or move cancelled cargo"
        );
    }
}

#[test]
fn cancel_does_not_make_delivered_shipments_mutable() {
    let mut f = fixture();
    stock(&mut f.state, SRC_CHEST, "minecraft:bread", 3);
    let mut policy = State::default();
    let request = open(&f, "delivered", 3, None);
    run(&mut f.state, f.settlement, &mut policy, request);
    teleport(&mut f.state, f.carrier, SRC_DOOR);
    assert!(tick(&mut f.state, f.settlement, &mut policy));
    teleport(&mut f.state, f.carrier, DST_DOOR);
    assert!(tick(&mut f.state, f.settlement, &mut policy));
    assert!(delivered(&policy, "delivered"));
    let before = policy.clone();
    assert_eq!(
        execute(
            &mut f.state,
            f.settlement,
            OWNER,
            &mut policy,
            Request::Cancel {
                id: "delivered".into()
            }
        ),
        Err(Failure::OperationConflict)
    );
    assert_eq!(policy, before);
}

#[path = "caravan_blocked_cancel_disk_tests.rs"]
mod disk;
