//! The restart-only reload contract and building a candidate generation.

use super::*;

use super::clock::timer_refusal;
use super::lifecycle::batch_limit;
use super::lifecycle::shutdown_hosts;
use crate::PluginStartup;
use crate::adapter::{PlayerSessions, to_script_batch};
use crate::bindings::exports::krasnokamen::plugin::lifecycle::InitContext;
use crate::startup::aggregate_deployment;
use crate::timers;

impl PluginReloadContract {
    pub(super) fn from_deployment(
        manifests: &[ValidatedScriptPluginManifest],
        catalog: Vec<mc_script::PluginPackage>,
        precommit_hooks: Vec<mc_script::precommit::HookRegistration>,
        surface: DeploymentSurface,
        contribution: DeploymentContribution,
    ) -> Self {
        Self {
            identities: manifests
                .iter()
                .map(|manifest| manifest.plugin_id().to_owned())
                .collect(),
            catalog,
            routes: manifests
                .iter()
                .map(|manifest| PackageRoutes {
                    player_roots: manifest.player_command_roots().to_vec(),
                    operator_roots: manifest.operator_command_roots().to_vec(),
                    channels: manifest
                        .custom_payload_channels()
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                    // Node requirements and the provider role are admission
                    // policy: a reload that changes either is not the same
                    // deployment contract.
                    command_permissions: manifest
                        .command_permissions()
                        .iter()
                        .map(|(root, node)| (root.clone(), node.clone()))
                        .collect(),
                    permission_provider: manifest.is_permission_provider(),
                })
                .collect(),
            grants: manifests
                .iter()
                .map(ValidatedScriptPluginManifest::to_command_capabilities)
                .collect(),
            precommit_hooks,
            surface,
            contribution,
        }
    }

    pub(super) fn incompatibility(&self, candidate: &Self) -> Option<PluginReloadContractField> {
        if self.identities != candidate.identities {
            return Some(PluginReloadContractField::PluginIdentities);
        }
        if self.catalog != candidate.catalog {
            return Some(PluginReloadContractField::PackageCatalog);
        }
        if self.routes != candidate.routes {
            return Some(PluginReloadContractField::CommandAndChannelOwnership);
        }
        if self.grants != candidate.grants {
            return Some(PluginReloadContractField::CommandGrants);
        }
        if self.precommit_hooks != candidate.precommit_hooks {
            return Some(PluginReloadContractField::PrecommitRegistrations);
        }
        if self.surface != candidate.surface {
            return Some(PluginReloadContractField::DeploymentSurface);
        }
        if self.contribution != candidate.contribution {
            return Some(PluginReloadContractField::StartupContribution);
        }
        None
    }
}

fn ensure_reload_candidate_memory(
    limits: PluginLimits,
    active_packages: usize,
    candidate_packages: usize,
) -> Result<(), PluginReloadError> {
    let requested_bytes = active_packages
        .checked_add(candidate_packages)
        .and_then(|packages| packages.checked_mul(limits.memories))
        .and_then(|memories| memories.checked_mul(limits.guest_memory_bytes))
        .unwrap_or(usize::MAX);
    if requested_bytes > limits.reload_candidate_memory_bytes {
        return Err(PluginReloadError::CandidateMemory {
            requested_bytes,
            limit_bytes: limits.reload_candidate_memory_bytes,
        });
    }
    Ok(())
}

pub(super) fn build_reload_candidate<S, F>(
    packages: Vec<LoadedPackage>,
    engine: &wasmtime::Engine,
    linker: &wasmtime::component::Linker<crate::InstanceState<S>>,
    limits: PluginLimits,
    active: ReloadGeneration<'_>,
    sessions: &dyn PlayerSessions,
    services: &F,
) -> Result<PluginReloadCandidate<S>, PluginReloadError>
where
    S: HostServices + 'static,
    F: Fn(&str) -> S,
{
    ensure_reload_candidate_memory(limits, active.contract.identities.len(), packages.len())?;
    let surface =
        aggregate_deployment(&packages).map_err(|error| PluginReloadError::CandidateSetup {
            message: error.to_string(),
        })?;
    let manifests = packages
        .iter()
        .map(|package| package.manifest().clone())
        .collect::<Vec<_>>();
    let catalog = packages
        .iter()
        .map(LoadedPackage::to_plugin_package)
        .collect::<Vec<_>>();
    let mut precommit_hooks = packages
        .iter()
        .flat_map(|package| package.precommit_registrations().iter().cloned())
        .collect::<Vec<_>>();
    precommit_hooks.sort_by(mc_script::precommit::by_roster_order);
    let mut candidate_contract = PluginReloadContract::from_deployment(
        &manifests,
        catalog,
        precommit_hooks,
        surface,
        DeploymentContribution::default(),
    );
    // Manifest-only constraints are known before configure/init can execute.
    if let Some(field) = active.contract.incompatibility(&candidate_contract)
        && field != PluginReloadContractField::StartupContribution
    {
        return Err(PluginReloadError::RestartContractChanged { field });
    }
    let permissions = Arc::new(
        manifests
            .iter()
            .find(|manifest| manifest.is_permission_provider())
            .map_or_else(mc_script::ScriptCommandPermissions::default, |manifest| {
                mc_script::ScriptCommandPermissions::for_provider(manifest.plugin_id())
            }),
    );

    let mut hosted = Vec::with_capacity(packages.len());
    let mut batches = Vec::with_capacity(packages.len());
    let mut contribution = DeploymentContribution::default();
    for package in packages {
        let id = package.manifest().plugin_id().to_owned();
        let refuse = |message: String| PluginReloadError::CandidateSetup {
            message: format!("plugin {id:?}: {message}"),
        };
        let compiled = crate::package::compile_package(engine, &package, &limits)
            .map_err(|error| refuse(error.to_string()))?;
        let admission = HostCommandAdmission::from_manifest(package.manifest());
        let config = crate::check::read_config(package.root(), &limits).map_err(refuse)?;
        let configured = {
            let mut startup =
                PluginStartup::instantiate(linker, compiled.component(), services(&id), limits)
                    .map_err(|error| refuse(error.to_string()))?;
            startup
                .configure(&config)
                .map_err(|error| refuse(error.to_string()))?
        };
        contribution.record(&id, configured.as_ref());

        let mut instance =
            PluginInstance::instantiate(linker, compiled.component(), services(&id), limits)
                .map_err(|error| refuse(error.to_string()))?;
        if package.manifest().is_permission_provider() {
            instance
                .state()
                .set_command_permissions(Some(Arc::clone(&permissions)));
        }
        let subscriptions = package
            .manifest()
            .event_subscriptions()
            .iter()
            .map(|subscription| subscription.event_name().to_owned())
            .collect::<BTreeSet<_>>();
        let batch = instance
            .init(
                &config,
                InitContext {
                    plugin_id: id.clone(),
                    api_version: compiled.api_version().to_owned(),
                    world_fingerprint: String::new(),
                },
            )
            .map_err(|error| refuse(error.to_string()))?;
        let (batch, mutations) = batch
            .split_timers(active.current_tick)
            .map_err(|error| refuse(timer_refusal(error)))?;
        let timers = timers::stage(&TimerSchedule::default(), mutations)
            .map_err(|error| refuse(timer_refusal(error)))?
            .unwrap_or_default();
        let batch = to_script_batch(
            batch,
            batch_limit(&limits),
            sessions,
            admission.capabilities(),
        )
        .map_err(|error| refuse(format!("{error:?}")))?;
        let commands = package
            .manifest()
            .player_command_roots()
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        batches.push((admission.clone(), batch));
        hosted.push(Hosted {
            id,
            subscriptions,
            commands,
            admission,
            auth_provider: package
                .requested_capabilities()
                .iter()
                .any(|capability| capability == "auth"),
            instance,
            diagnostics: InstanceDiagnostics::default(),
            callback_latency: CallbackLatencyWindow::default(),
            limits,
            timers,
            observed_tick: active.current_tick,
            initial_timer_clock_pending: false,
            retired: false,
        });
    }

    candidate_contract.contribution = contribution;
    if let Some(field) = active.contract.incompatibility(&candidate_contract) {
        shutdown_hosts(hosted);
        return Err(PluginReloadError::RestartContractChanged { field });
    }
    Ok(PluginReloadCandidate {
        hosted,
        manifests,
        batches,
        permissions,
    })
}
