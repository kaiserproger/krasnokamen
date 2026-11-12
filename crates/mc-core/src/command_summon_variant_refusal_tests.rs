//! Climate/nautilus variants: a corrupt value fails the native checkpoint
//! closed, an unknown entry is ignored with a warning like vanilla (records an
//! older importer stored still hold them), the WAL image is strict, and Anvil
//! import drops an ignored value.
use super::*;
use crate::journal::{EntityDataValueImage as Image, Replay};
use mc_nbt::Tag;

const SPECIES: [(&str, &str, u8); 4] = [
    ("cow", "cold", 18),
    ("pig", "warm", 19),
    ("chicken", "cold", 18),
    ("zombie_nautilus", "warm", 21),
];

/// The native entity record a checkpoint would store for a summoned `kind`.
fn record(kind: &str, variant: &str) -> (TickState, EntityId, mc_data::entity_nbt::VanillaEntity) {
    let (state, id) = summon(kind, &format!("{{variant:'minecraft:{variant}'}}"));
    let uuid = state.entities.row(state.entities.slot_of(id).unwrap()).uuid;
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let native = chunks
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|record| record.uuid == uuid)
        .unwrap()
        .clone();
    (state, id, native)
}

fn hydrate(
    state: &TickState,
    native: &mc_data::entity_nbt::VanillaEntity,
) -> Result<Replay, crate::CoreError> {
    let (_, mut chunks, _) = crate::core::checkpoint_data_for(state, state.tick);
    for chunk in chunks.values_mut() {
        if let Some(record) = chunk
            .entities
            .iter_mut()
            .find(|record| record.uuid == native.uuid)
        {
            *record = native.clone();
        }
    }
    let encoded = crate::native_checkpoint::encode_entities(
        crate::dimensions::Dimension::Overworld,
        &chunks,
        &state.registry.items,
        state.tick,
    )
    .unwrap();
    let payload = encoded
        .iter()
        .find(|entity| entity.identity == *native.uuid.as_bytes())
        .and_then(|entity| entity.payload.as_ref())
        .unwrap();
    let mut replay = Replay::default();
    crate::native_checkpoint::hydrate_entity(&state.registry, &mut replay, payload, state.tick)?;
    Ok(replay)
}

fn set_variant(native: &mut mc_data::entity_nbt::VanillaEntity, value: Tag) {
    native.extra_fields.retain(|(key, _)| key != "variant");
    native.extra_fields.push(("variant".into(), value));
}

#[test]
fn checkpoint_refuses_corrupt_variant_values_and_defaults_absent_or_unknown_entries() {
    for (kind, variant, index) in SPECIES {
        let (state, id, native) = record(kind, variant);
        let good = hydrate(&state, &native).unwrap();
        assert!(
            good.entities[&id.0]
                .appearance
                .extra
                .iter()
                .any(|(at, _)| *at == index),
            "{kind}: valid variant restored"
        );

        let mut absent = native.clone();
        absent.extra_fields.retain(|(key, _)| key != "variant");
        let default = hydrate(&state, &absent).unwrap();
        assert!(
            default.entities[&id.0].appearance.extra.is_empty(),
            "{kind}: an absent field keeps the vanilla default"
        );

        for (what, bad) in [
            ("non-string", Tag::Int(1)),
            ("unparsable id", Tag::String("BAD:UPPERCASE".into())),
        ] {
            let mut corrupt = native.clone();
            set_variant(&mut corrupt, bad);
            let error = hydrate(&state, &corrupt).unwrap_err().to_string();
            assert!(error.contains("variant"), "{kind} {what}: {error}");
        }

        for (what, unknown) in [
            ("unknown entry", "minecraft:not_a_variant"),
            ("custom datapack entry", "custom:pink"),
            ("other registry's entry", "minecraft:tabby"),
        ] {
            let mut ignored = native.clone();
            set_variant(&mut ignored, Tag::String(unknown.into()));
            let restored =
                hydrate(&state, &ignored).unwrap_or_else(|error| panic!("{kind} {what}: {error}"));
            assert!(
                restored.entities[&id.0]
                    .appearance
                    .extra
                    .iter()
                    .all(|(at, _)| *at != index),
                "{kind} {what}: the default is kept"
            );
        }
    }
}

/// A store written before the importer dropped ignored values holds the raw
/// imported record; the world must still open, with the default variant.
#[test]
fn checkpoint_store_with_raw_imported_unknown_variants_opens_with_defaults() {
    let (mut state, cow) = summon("cow", "{}");
    let chicken = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:chicken").unwrap());
    assert!(chicken.is_some());
    let chicken = {
        let type_id = chicken.unwrap();
        let health = state.registry.blueprint(type_id).unwrap().params.max_health;
        state.spawn_mob(type_id, [1.5, 80.0, 1.5], health)
    };
    let uuid_of =
        |state: &TickState, id| state.entities.row(state.entities.slot_of(id).unwrap()).uuid;
    let (cow_uuid, chicken_uuid) = (uuid_of(&state, cow), uuid_of(&state, chicken));
    let (_, mut chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    for record in chunks
        .values_mut()
        .flat_map(|chunk| chunk.entities.iter_mut())
    {
        let raw = if record.uuid == cow_uuid {
            "custom:pink"
        } else if record.uuid == chicken_uuid {
            "minecraft:not_a_variant"
        } else {
            continue;
        };
        set_variant(record, Tag::String(raw.into()));
    }
    let native = crate::native_checkpoint::encode_entities(
        crate::dimensions::Dimension::Overworld,
        &chunks,
        &state.registry.items,
        state.tick,
    )
    .unwrap();
    let root = std::env::temp_dir().join(format!("solaris-variant-raw-{}", uuid::Uuid::new_v4()));
    let config = crate::CoreConfig::new(root.join("checkpoint"), 42);
    crate::native_checkpoint::tests::write_manifest(&config, &Default::default()).unwrap();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let base = store.read_checkpoint().unwrap();
    store
        .commit_checkpoint(
            mc_world::native::NativeCheckpoint {
                tick: state.tick,
                previous_tick: base.tick,
                chunks: &[],
                entities: &native,
                players: &[],
                globals: &base.globals,
            },
            &state.registry.blocks,
            &state.registry.items,
        )
        .unwrap();
    drop(store);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let replay = &recovered.dimensions[&crate::dimensions::Dimension::Overworld];
    for (id, index) in [(cow, 18), (chicken, 18)] {
        let entity = &replay.entities[&id.0];
        assert!(
            entity.appearance.extra.iter().all(|(at, _)| *at != index),
            "{id:?}: the default variant is kept"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn journal_image_refuses_variants_that_do_not_belong_to_the_species() {
    let (state, id, native) = record("cow", "cold");
    let replay = hydrate(&state, &native).unwrap();
    let good = replay.entities[&id.0].clone();
    crate::dimensions::recovery::validate_entity(&state, id.0, &good).unwrap();
    let cold = mc_data::required_registry_entry_id(
        "cow_variant",
        &mc_data::Identifier::parse("minecraft:cold").unwrap(),
    )
    .unwrap();

    for (what, extra) in [
        ("pig value on a cow", vec![(19, Image::PigVariant(0))]),
        (
            "cow value at the pig index",
            vec![(19, Image::CowVariant(cold))],
        ),
        (
            "chicken value on a cow",
            vec![(18, Image::ChickenVariant(cold))],
        ),
        ("id out of range", vec![(18, Image::CowVariant(u32::MAX))]),
        (
            "non-variant at the variant index",
            vec![(18, Image::Byte(1))],
        ),
    ] {
        let mut corrupt = good.clone();
        corrupt.appearance.extra = extra;
        let error = crate::dimensions::recovery::validate_entity(&state, id.0, &corrupt)
            .unwrap_err()
            .to_string();
        assert!(error.contains("variant"), "{what}: {error}");
    }

    let (sheep_state, sheep) = summon("sheep", "{}");
    let sheep_native = {
        let uuid = sheep_state
            .entities
            .row(sheep_state.entities.slot_of(sheep).unwrap())
            .uuid;
        let (_, chunks, _) = crate::core::checkpoint_data_for(&sheep_state, sheep_state.tick);
        chunks
            .values()
            .flat_map(|chunk| &chunk.entities)
            .find(|record| record.uuid == uuid)
            .unwrap()
            .clone()
    };
    let mut on_sheep = hydrate(&sheep_state, &sheep_native).unwrap().entities[&sheep.0].clone();
    on_sheep
        .appearance
        .extra
        .push((18, Image::CowVariant(cold)));
    assert!(
        crate::dimensions::recovery::validate_entity(&sheep_state, sheep.0, &on_sheep)
            .unwrap_err()
            .to_string()
            .contains("variant")
    );
}

#[test]
fn anvil_import_drops_an_ignored_variant_like_vanilla_and_keeps_a_valid_one() {
    for (kind, variant, index) in SPECIES {
        let (state, id, native) = record(kind, variant);
        let mut valid = native.clone();
        crate::native_checkpoint::drop_ignored_variant(&mut valid);
        assert_eq!(valid.extra_fields, native.extra_fields, "{kind}");

        for bad in [
            Tag::Int(1),
            Tag::String("BAD:UPPERCASE".into()),
            Tag::String("minecraft:not_a_variant".into()),
        ] {
            let mut ignored = native.clone();
            set_variant(&mut ignored, bad);
            assert_eq!(
                hydrate(&state, &ignored).is_err(),
                !matches!(&ignored.extra_fields.last(), Some((_, Tag::String(name))) if name == "minecraft:not_a_variant"),
                "{kind}: a corrupt value is refused, an unknown entry tolerated"
            );
            crate::native_checkpoint::drop_ignored_variant(&mut ignored);
            assert!(ignored.extra_fields.iter().all(|(key, _)| key != "variant"));
            let restored = hydrate(&state, &ignored).unwrap();
            assert!(
                !restored.entities[&id.0]
                    .appearance
                    .extra
                    .iter()
                    .any(|(at, _)| *at == index),
                "{kind}: the default is kept"
            );
        }
    }
}
