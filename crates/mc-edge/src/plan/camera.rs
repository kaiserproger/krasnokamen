//! State-driven projection on the same FIFO as entity pairing and removal.
use super::*;
use mc_protocol::packets::play::ClientboundSetCamera;

pub(super) fn send(
    viewer: &mut ViewerState,
    target: EntityId,
    out: &mut Vec<PlannedPacket>,
    credit: &mut WorldCredit,
) -> bool {
    if viewer.camera == Some(target) {
        return true;
    }
    if !try_push(out, credit, |out| {
        push_best_effort(
            out,
            viewer.connection,
            &ClientboundSetCamera {
                entity_id: target.0,
            },
        );
    }) {
        return false;
    }
    viewer.camera = Some(target);
    tracing::info!(
        viewer = viewer.entity_id.0,
        target = target.0,
        "Set Camera planned on world FIFO"
    );
    true
}

pub(super) fn reconcile(
    snapshot: &WorldSnapshot,
    viewer: &mut ViewerState,
    out: &mut Vec<PlannedPacket>,
    credit: &mut WorldCredit,
) {
    let target = snapshot
        .cameras
        .get(&viewer.entity_id)
        .copied()
        .unwrap_or(viewer.entity_id);
    if target == viewer.entity_id
        || (viewer.tracked_entities.contains_key(&target)
            && !is_undelivered(viewer, target)
            && snapshot.entities.get(target).is_some())
    {
        send(viewer, target, out, credit);
    }
}
