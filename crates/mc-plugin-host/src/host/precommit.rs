//! Pre-commit hook requests answered by the live instances.

use super::*;

use super::delivery::retire;
use mc_script::ScriptHostEndpoint;
use mc_script::precommit::{
    DamageContext, DamageTarget, HookActor, HookDecision, HookFailure, HookFailurePolicy, Request,
};

/// Run the published roster as one ordered pre-commit chain.
///
/// Retired instances remain represented by their registrations: they answer a
/// handler failure under that registration's configured policy instead of being
/// silently removed from the protection the operator installed.
pub(super) fn deliver_precommit<S: HostServices + 'static>(
    hosted: &mut [Hosted<S>],
    endpoint: &ScriptHostEndpoint,
    request: Request,
) {
    if request.is_expired() {
        let _ = request.fail(HookFailure::Expired);
        return;
    }
    let mut storage_versions = Vec::new();
    match request.context() {
        mc_script::precommit::HookContext::Build(context) => {
            if !is_supported_hook_actor(context.actor()) {
                let _ = request.fail(HookFailure::Invalid);
                return;
            }
            for registration in request.hooks() {
                if Instant::now() >= request.deadline() {
                    let _ = request.fail(HookFailure::Expired);
                    return;
                }
                let result = hosted
                    .iter_mut()
                    .find(|candidate| candidate.id == registration.plugin_id())
                    .filter(|candidate| !candidate.retired)
                    .ok_or(())
                    .and_then(|candidate| {
                        candidate
                            .instance
                            .before_build(context, request.deadline())
                            .map_err(|error| {
                                if candidate.instance.retired_because().is_some() {
                                    retire(candidate, endpoint, &error);
                                }
                            })
                    })
                    .map(|answer| {
                        if !answer.expected_plugin_storage_versions.is_empty() {
                            storage_versions.push(
                                mc_script::precommit::HookStorageVersionGuards::new(
                                    registration.plugin_id().to_owned(),
                                    answer.expected_plugin_storage_versions,
                                ),
                            );
                        }
                        answer.decision
                    });
                match result {
                    Ok(HookDecision::Keep) => {}
                    Ok(HookDecision::Cancel) => {
                        let _ = request.answer(HookDecision::Cancel, Vec::new());
                        return;
                    }
                    Ok(HookDecision::Replace(_)) | Err(())
                        if registration.on_failure() == HookFailurePolicy::Deny =>
                    {
                        let _ = request.answer(HookDecision::Cancel, Vec::new());
                        return;
                    }
                    Ok(HookDecision::Replace(_)) | Err(()) => {}
                    Ok(_) => {
                        let _ = request.answer(HookDecision::Cancel, Vec::new());
                        return;
                    }
                }
            }
            let _ = request.answer(HookDecision::Keep, storage_versions);
        }
        mc_script::precommit::HookContext::Damage(context) => {
            if !is_supported_hook_actor(context.source())
                || !is_supported_damage_target(context.target())
            {
                let _ = request.fail(HookFailure::Invalid);
                return;
            }
            let mut current = context.clone();
            for registration in request.hooks() {
                if Instant::now() >= request.deadline() {
                    let _ = request.fail(HookFailure::Expired);
                    return;
                }
                let result = hosted
                    .iter_mut()
                    .find(|candidate| candidate.id == registration.plugin_id())
                    .filter(|candidate| !candidate.retired)
                    .ok_or(())
                    .and_then(|candidate| {
                        candidate
                            .instance
                            .before_damage(&current, request.deadline())
                            .map_err(|error| {
                                if candidate.instance.retired_because().is_some() {
                                    retire(candidate, endpoint, &error);
                                }
                            })
                    })
                    .map(|answer| {
                        if !answer.expected_plugin_storage_versions.is_empty() {
                            storage_versions.push(
                                mc_script::precommit::HookStorageVersionGuards::new(
                                    registration.plugin_id().to_owned(),
                                    answer.expected_plugin_storage_versions,
                                ),
                            );
                        }
                        answer.decision
                    });
                match result {
                    Ok(HookDecision::Keep) => {}
                    Ok(HookDecision::Cancel) => {
                        let _ = request.answer(HookDecision::Cancel, Vec::new());
                        return;
                    }
                    Ok(HookDecision::Replace(amount)) => {
                        current = DamageContext::try_new(
                            current.source().clone(),
                            current.target().clone(),
                            current.kind().to_owned(),
                            current.dimension().to_owned(),
                            current.position(),
                            amount,
                        )
                        .expect("a valid replacement preserves a valid damage context");
                    }
                    Ok(_) => {
                        let _ = request.answer(HookDecision::Cancel, Vec::new());
                        return;
                    }
                    Err(()) if registration.on_failure() == HookFailurePolicy::Deny => {
                        let _ = request.answer(HookDecision::Cancel, Vec::new());
                        return;
                    }
                    Err(()) => {}
                }
            }
            let decision = HookDecision::replacement(current.amount())
                .expect("a damage context always carries a valid amount");
            let _ = request.answer(decision, storage_versions);
        }
        _ => {
            let _ = request.fail(HookFailure::Invalid);
        }
    }
}

fn is_supported_hook_actor(actor: &HookActor) -> bool {
    matches!(
        actor,
        HookActor::Player(_) | HookActor::Entity(_) | HookActor::Plugin(_) | HookActor::Environment
    )
}

fn is_supported_damage_target(target: &DamageTarget) -> bool {
    matches!(target, DamageTarget::Player(_) | DamageTarget::Entity(_))
}
