//! Coreperf instrumentation windows: which active chunks, cells and damage
//! observations a tick samples. Byte-exact move out of `entity_phase.rs`; the
//! recorded windows are unchanged.

use super::*;

#[derive(serde::Deserialize)]
struct CorePerfInput {
    marker: String,
    regions: Vec<Vec<[i32; 2]>>,
    #[serde(default)]
    productive_crops: Vec<[i32; 3]>,
}

struct CorePerfIndex {
    marker: String,
    chunks: HashMap<(i32, i32), usize>,
    productive_crops: HashSet<(i32, i32, i32)>,
}

pub(super) fn coreperf_index() -> Option<&'static CorePerfIndex> {
    static INDEX: std::sync::LazyLock<Option<CorePerfIndex>> = std::sync::LazyLock::new(|| {
        let path = std::env::var_os("KRASNOKAMEN_COREPERF_REGIONS")?;
        let input: CorePerfInput = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        let mut chunks = HashMap::default();
        for (region, positions) in input.regions.into_iter().enumerate() {
            for [x, z] in positions {
                if chunks.insert((x, z), region).is_some() {
                    return None;
                }
            }
        }
        Some(CorePerfIndex {
            marker: input.marker,
            chunks,
            productive_crops: input
                .productive_crops
                .into_iter()
                .map(|[x, y, z]| (x, y, z))
                .collect(),
        })
    });
    INDEX.as_ref()
}

pub(super) fn coreperf_slot_region(
    index: &CorePerfIndex,
    state: &TickState,
    slot: EntitySlot,
) -> Option<usize> {
    let chunk = state.entities.chunk_of(slot);
    let region = *index.chunks.get(&(chunk.x, chunk.z))?;
    (state.entities.row(slot).appearance.custom_name.as_deref() == Some(index.marker.as_str()))
        .then_some(region)
}

pub(crate) fn coreperf_block_region(x: i32, z: i32) -> Option<usize> {
    coreperf_index()?
        .chunks
        .get(&(x.div_euclid(16), z.div_euclid(16)))
        .copied()
}

pub(super) fn coreperf_productive_changes(
    changes: &[crate::boundary::BlockChange],
    sample: &mut crate::boundary::CorePerfTick,
) {
    if let Some(index) = coreperf_index() {
        record_productive_changes(&index.productive_crops, changes, sample);
    }
}

fn record_productive_changes(
    crops: &HashSet<(i32, i32, i32)>,
    changes: &[crate::boundary::BlockChange],
    sample: &mut crate::boundary::CorePerfTick,
) {
    if crops.is_empty() {
        return;
    }
    for change in changes {
        let pos = change.position;
        if crops.contains(&(pos.x, pos.y, pos.z)) {
            sample
                .productive_harvested_cells
                .push([pos.x, pos.y, pos.z]);
        }
    }
}

pub(super) fn coreperf_damage_observation(
    sample: &mut crate::boundary::CorePerfTick,
    state: &TickState,
    slot: EntitySlot,
    observation: crate::tick::damage::DamageObservation,
) {
    if let Some(index) = coreperf_index() {
        record_damage_observation(index, sample, state, slot, observation);
    }
}

fn record_damage_observation(
    index: &CorePerfIndex,
    sample: &mut crate::boundary::CorePerfTick,
    state: &TickState,
    slot: EntitySlot,
    observation: crate::tick::damage::DamageObservation,
) {
    if let Some(region) = coreperf_slot_region(index, state, slot) {
        let counts = match observation {
            crate::tick::damage::DamageObservation::AcceptedVictim => {
                &mut sample.damage_victims_by_region
            }
            crate::tick::damage::DamageObservation::AcceptedReactive => {
                &mut sample.damage_reactive_by_region
            }
        };
        *counts.entry(region).or_default() += 1;
    }
}

pub(super) fn coreperf_region(
    counts: &mut HashMap<usize, crate::boundary::CorePerfRegion>,
    region: usize,
) -> &mut crate::boundary::CorePerfRegion {
    counts
        .entry(region)
        .or_insert_with(|| crate::boundary::CorePerfRegion {
            region,
            ..Default::default()
        })
}

pub(super) fn coreperf_active_chunks(
    index: &CorePerfIndex,
    active: impl Iterator<Item = (i32, i32)>,
    counts: &mut HashMap<usize, crate::boundary::CorePerfRegion>,
) {
    for chunk in active {
        if let Some(&region) = index.chunks.get(&chunk) {
            coreperf_region(counts, region).active_chunks += 1;
        }
    }
}

#[cfg(test)]
mod coreperf_counter_tests {
    use super::*;

    #[test]
    fn visits_active_chunks_not_cold_fixture_population() {
        let index = CorePerfIndex {
            marker: String::new(),
            chunks: (0..100_000).map(|x| ((x, 0), x as usize)).collect(),
            productive_crops: HashSet::default(),
        };
        let visits = std::cell::Cell::new(0);
        let active = [(17, 0)]
            .into_iter()
            .inspect(|_| visits.set(visits.get() + 1));
        let mut counts = HashMap::default();
        coreperf_active_chunks(&index, active, &mut counts);
        assert_eq!(visits.get(), 1);
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[&17].active_chunks, 1);
    }

    #[test]
    fn damage_merge_counts_distinct_accepted_groups_in_authoritative_fixture_regions() {
        use crate::intent::DamageSource;
        use crate::survival::DamageKind;
        use crate::tick::DamagePiece;
        use mc_data::{Identifier, ItemEnchantment, ItemStack};

        let (mut state, _, pos) = crate::hostile_tests::setup(8.0);
        state.tick = 20;
        let target = state.spawn_mob(state.registry.zombie_type_id, pos, 100.0);
        let attacker = state.spawn_mob(
            state.registry.zombie_type_id,
            [pos[0] + 16.0, pos[1], pos[2]],
            100.0,
        );
        let blocked = state.spawn_mob(
            state.registry.zombie_type_id,
            [pos[0] + 1.0, pos[1], pos[2]],
            100.0,
        );
        let unmarked = state.spawn_mob(
            state.registry.zombie_type_id,
            [pos[0] + 2.0, pos[1], pos[2]],
            100.0,
        );
        let outside = state.spawn_mob(
            state.registry.zombie_type_id,
            [pos[0] + 48.0, pos[1], pos[2]],
            100.0,
        );
        let slots = [target, attacker, blocked, unmarked, outside]
            .map(|id| state.entities.slot_of(id).unwrap());
        for slot in [slots[0], slots[1], slots[2], slots[4]] {
            std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).custom_name =
                Some("damage-fixture".into());
        }
        state.entities.row_mut(slots[2]).controls.invulnerable = true;
        let armor_id = state
            .registry
            .items
            .id_of(&Identifier::parse("minecraft:diamond_chestplate").unwrap())
            .unwrap();
        let mut armor = ItemStack::new(armor_id, 1);
        armor.enchantments.push(ItemEnchantment {
            id: Identifier::parse("minecraft:thorns").unwrap(),
            level: 10,
        });
        std::sync::Arc::make_mut(&mut state.entities.row_mut(slots[0]).appearance).equipment[4] =
            Some(armor);
        let target_chunk = state.entities.chunk_of(slots[0]);
        let attacker_chunk = state.entities.chunk_of(slots[1]);
        let index = CorePerfIndex {
            marker: "damage-fixture".into(),
            chunks: HashMap::from_iter([
                ((target_chunk.x, target_chunk.z), 7),
                ((attacker_chunk.x, attacker_chunk.z), 19),
            ]),
            productive_crops: HashSet::default(),
        };
        let source = DamageSource::Melee {
            attacker,
            position: state.entities.pos(slots[1]),
            yaw: 0.0,
            bonus_knockback: 0.0,
            sweep: false,
            fire_ticks: 0,
            bane_ticks: 0,
        };
        // Ordinary and reactive pieces share the same final victim group.
        for (slot, amount, source) in [
            (slots[0], 10.0, Some(source)),
            (slots[0], 20.0, None),
            (slots[1], 0.25, None),
            (slots[2], 10.0, None),
            (slots[3], 10.0, None),
            (slots[4], 10.0, None),
        ] {
            state.merge_damage.push((
                slot,
                DamagePiece {
                    amount,
                    kind: DamageKind::Melee,
                    source,
                    physical: None,
                    custom_armor: false,
                },
            ));
        }
        let mut sample = crate::boundary::CorePerfTick::default();
        state.commit_merge_effects_observed(Some(&mut |state, slot, observation| {
            record_damage_observation(&index, &mut sample, state, slot, observation);
        }));
        assert_eq!(sample.damage_victims_by_region.len(), 2);
        assert_eq!(sample.damage_victims_by_region[&7], 1);
        assert_eq!(sample.damage_victims_by_region[&19], 1);
        assert_eq!(sample.damage_reactive_by_region.len(), 1);
        assert_eq!(sample.damage_reactive_by_region[&19], 1);
        assert_eq!(state.entities.row(slots[2]).health, 100.0);
        assert!(state.entities.row(slots[0]).health < 100.0);
        assert!(state.entities.row(slots[1]).health < 99.75);
        assert!(state.entities.row(slots[3]).health < 100.0);
        assert!(state.entities.row(slots[4]).health < 100.0);
        state.commit_merge_effects_observed(Some(&mut |_, _, _| {
            panic!("empty merge produced a damage observation");
        }));
    }

    #[test]
    fn productive_observation_only_records_declared_changed_cells() {
        let crops = HashSet::from_iter([(2, 64, 3)]);
        let changes = [
            crate::boundary::BlockChange {
                position: crate::boundary::BlockPos { x: 2, y: 64, z: 3 },
                state: 1,
            },
            crate::boundary::BlockChange {
                position: crate::boundary::BlockPos { x: 4, y: 64, z: 5 },
                state: 2,
            },
        ];
        let mut sample = crate::boundary::CorePerfTick::default();
        record_productive_changes(&crops, &changes, &mut sample);
        assert_eq!(sample.productive_harvested_cells, vec![[2, 64, 3]]);
    }
}
