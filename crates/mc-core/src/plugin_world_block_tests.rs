use super::WorldBlockFixture;
use mc_script::{
    CommandBatch, HostCommandAdmission, ScriptAxisAlignedZone, ScriptCommand, ScriptEventKind,
    ScriptPluginManifest, ScriptPosition, ScriptWorldBlockSetFailure, ScriptZoneProtection,
};
use std::num::NonZeroUsize;

fn remote_target(fixture: &mut WorldBlockFixture, dimension: crate::dimensions::Dimension) {
    let mut target = super::fixture();
    target.dimension = dimension;
    target
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter([(
            crate::boundary::ChunkPos { x: 0, z: 0 },
            0,
        )]));
    fixture
        .state
        .other_dimensions
        .insert(dimension, Box::new(target));
    fixture.request = mc_script::ScriptWorldBlockSetRequest::try_new(
        "remote-block",
        dimension.name(),
        "minecraft:stone",
        1,
        64,
        1,
    )
    .unwrap();
}

fn zone(fixture: &mut WorldBlockFixture, owner: &str, dimension: &str, protected: bool, x: f64) {
    let manifest = ScriptPluginManifest::new(
        owner,
        "Zone owner",
        "1",
        mc_script::COMPONENT_PLUGIN_API_VERSION,
    )
    .declare_zones()
    .declare_world_blocks()
    .validate()
    .unwrap();
    if fixture.boundary.plugin_target(owner).is_none() {
        fixture.host.register_plugin_routes(&manifest).unwrap();
    }
    let allowed = uuid::Uuid::from_u128(91).to_string();
    let protection =
        protected.then(|| ScriptZoneProtection::try_new([allowed.as_str()], false).unwrap());
    let zone = ScriptAxisAlignedZone::try_new_with_protection(
        "claim",
        dimension,
        ScriptPosition::try_new(x, 0.0, 0.0).unwrap(),
        ScriptPosition::try_new(x + 3.0, 100.0, 3.0).unwrap(),
        protection,
    )
    .unwrap();
    let admission = HostCommandAdmission::from_manifest(&manifest);
    let mut batch = CommandBatch::new(NonZeroUsize::new(1).unwrap());
    batch
        .try_push_authorized(ScriptCommand::UpsertZone { zone }, admission.capabilities())
        .unwrap();
    fixture
        .host
        .try_submit_plugin_batch(&admission, batch)
        .unwrap();
    crate::plugin::pump(&mut fixture.state);
    let events = &mut fixture
        .state
        .plugin_runtime
        .as_mut()
        .unwrap()
        .pending_events;
    assert!(events.iter().any(|event| matches!(
        event.kind(),
        ScriptEventKind::ZoneCommandResult { accepted: true, .. }
    )));
    events.clear();
}

#[test]
fn plugin_block_sets_refuse_foreign_protected_zones_but_keep_owned_and_event_zones() {
    for (owner, dimension, protected, allowed) in [
        ("protector", "minecraft:overworld", true, false),
        ("builder", "minecraft:overworld", true, true),
        ("protector", "minecraft:overworld", false, true),
        ("protector", "minecraft:the_nether", true, true),
    ] {
        let mut fixture = WorldBlockFixture::new(false);
        zone(&mut fixture, owner, dimension, protected, 0.0);
        let before = fixture.state.world.block_at(1, 64, 1);
        fixture.submit();
        crate::plugin::pump(&mut fixture.state);
        if allowed {
            assert_eq!(fixture.results(), vec![(true, None)]);
            assert_eq!(fixture.state.plugin_block_writes.len(), 1);
        } else {
            assert_eq!(
                fixture.results(),
                vec![(false, Some(ScriptWorldBlockSetFailure::Rejected))]
            );
            assert_eq!(fixture.state.world.block_at(1, 64, 1), before);
            assert!(fixture.state.plugin_block_writes.is_empty());
        }
    }
}

#[test]
fn plugin_block_sets_recheck_zone_protection_after_hook_wait() {
    let mut fixture = WorldBlockFixture::new(true);
    zone(&mut fixture, "protector", "minecraft:overworld", true, 10.0);
    fixture.submit();
    crate::plugin::pump(&mut fixture.state);
    let question = fixture.question();
    zone(&mut fixture, "protector", "minecraft:overworld", true, 0.0);
    let before = fixture.state.world.block_at(1, 64, 1);
    question
        .answer(mc_script::precommit::HookDecision::Keep, Vec::new())
        .unwrap();
    crate::plugin::pump(&mut fixture.state);
    assert_eq!(
        fixture.results(),
        vec![(false, Some(ScriptWorldBlockSetFailure::Rejected))]
    );
    assert_eq!(fixture.state.world.block_at(1, 64, 1), before);
    assert!(fixture.state.plugin_block_writes.is_empty());
}

#[test]
fn remote_plugin_block_sets_use_root_zone_authority() {
    for dimension in [
        crate::dimensions::Dimension::Nether,
        crate::dimensions::Dimension::End,
    ] {
        let mut fixture = WorldBlockFixture::new(false);
        remote_target(&mut fixture, dimension);
        zone(&mut fixture, "protector", dimension.name(), true, 0.0);
        let before = fixture.state.other_dimensions[&dimension]
            .world
            .block_at(1, 64, 1);
        fixture.submit();
        crate::plugin::pump(&mut fixture.state);
        assert_eq!(
            fixture.results(),
            vec![(false, Some(ScriptWorldBlockSetFailure::Rejected))]
        );
        let target = &fixture.state.other_dimensions[&dimension];
        assert_eq!(target.world.block_at(1, 64, 1), before);
        assert!(target.plugin_block_writes.is_empty());
    }
}

#[test]
fn remote_plugin_block_sets_recheck_root_zones_after_hook_wait() {
    for dimension in [
        crate::dimensions::Dimension::Nether,
        crate::dimensions::Dimension::End,
    ] {
        let mut fixture = WorldBlockFixture::new(true);
        remote_target(&mut fixture, dimension);
        zone(&mut fixture, "protector", dimension.name(), true, 10.0);
        fixture.submit();
        crate::plugin::pump(&mut fixture.state);
        let question = fixture.question();
        zone(&mut fixture, "protector", dimension.name(), true, 0.0);
        let before = fixture.state.other_dimensions[&dimension]
            .world
            .block_at(1, 64, 1);
        question
            .answer(mc_script::precommit::HookDecision::Keep, Vec::new())
            .unwrap();
        crate::plugin::pump(&mut fixture.state);
        assert_eq!(
            fixture.results(),
            vec![(false, Some(ScriptWorldBlockSetFailure::Rejected))]
        );
        let target = &fixture.state.other_dimensions[&dimension];
        assert_eq!(target.world.block_at(1, 64, 1), before);
        assert!(target.plugin_block_writes.is_empty());
    }
}
