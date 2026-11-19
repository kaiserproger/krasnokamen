//! Starting, stopping and reloading a deployment's host thread.

use super::*;

use super::clock::deliver_timers;
use super::clock::timer_refusal;
use super::conversion::contract_events;
use super::delivery::deliver;
use super::delivery::interested;
use super::delivery::submit;
use super::precommit::deliver_precommit;
use super::reload::build_reload_candidate;
use crate::PluginStartup;
use crate::adapter::PlayerSessions;
use crate::bindings::exports::krasnokamen::plugin::lifecycle::InitContext;
use crate::startup::aggregate_deployment;
use crate::timers;
use mc_script::{
    ClientBundle, PluginWorldgenOreProfile, ScriptEventKind, ScriptHostInput,
    ScriptReloadCommitError,
};
use std::sync::atomic::Ordering;

impl PluginHost {
    /// The boundary the server keeps: events go in, admitted commands come out.
    #[must_use]
    pub fn boundary(&self) -> &ScriptBoundary {
        &self.boundary
    }

    /// What every package of this deployment answered to `configure`.
    ///
    /// Per package it is the plugin id, the validated startup rules when the
    /// package declared a contribution the startup contract accepts, a refusal
    /// when it declared one the contract does not, and neither when it declared
    /// none: a caller that has to fail startup closed reads
    /// [`DeploymentContribution::refusal`], and a package with no contribution is
    /// not a failure.
    #[must_use]
    pub fn contribution(&self) -> &DeploymentContribution {
        &self.contribution
    }

    /// Every client bundle this deployment declared.
    ///
    /// The packages' own bundles, concatenated in the order the packages were
    /// started. Each one carries its owner, its validated artifact and hash, its
    /// loaders and the content kinds it declares; the Loader decides what to stage
    /// from them, under the permission each content kind needs.
    #[must_use]
    pub fn client_bundles(&self) -> &[ClientBundle] {
        self.surface.client_bundles()
    }

    /// The one ore profile this deployment declared, if any.
    #[must_use]
    pub fn worldgen_ore_profile(&self) -> Option<PluginWorldgenOreProfile> {
        self.surface.ore_profile()
    }

    /// Build and atomically activate an already-discovered replacement deployment.
    ///
    /// The candidate crosses the same FIFO as events, so events admitted before
    /// this request reach the old generation and later events reach the replacement.
    /// Its stores, configuration and initialization are all owned by the host
    /// thread; callers never need a second script boundary or session view.
    pub async fn reload(
        &self,
        packages: Vec<LoadedPackage>,
    ) -> Result<PluginReloadReport, PluginReloadError> {
        let (response, received) = tokio::sync::oneshot::channel();
        self.reload_sender
            .send_reload(Box::new(PluginReloadRequest { packages, response }))
            .await
            .map_err(|_| PluginReloadError::HostClosed)?;
        received.await.map_err(|_| PluginReloadError::HostClosed)?
    }

    /// Stop the host, run every live guest's `shutdown` and answer the counters.
    ///
    /// The host reclaims what it owns whether or not a guest's `shutdown`
    /// succeeds: a plugin never keeps a route, a timer or a registration alive by
    /// failing to clean up.
    pub fn stop(mut self) -> Vec<(String, InstanceDiagnostics)> {
        self.stop.store(true, Ordering::Release);
        self.boundary.close_event_admission();
        let counters = self
            .thread
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        // The watchdog stops with the host, never before it.
        self.ticker.take();
        counters
    }
}

/// Start every package of the deployment on its own instance.
///
/// The packages are started before this function returns, so a deployment that
/// cannot start fails the caller instead of half-running.
pub fn start_deployment(
    packages: Vec<LoadedPackage>,
    limits: PluginLimits,
    queues: HostQueues,
    sessions: Arc<dyn PlayerSessions + Send + Sync>,
) -> Result<PluginHost, HostStartError> {
    start_deployment_with(packages, limits, queues, sessions, |id: &str| Tracing {
        id: id.to_owned(),
    })
}

/// The same, with the contract's host-side imports supplied by the caller.
///
/// A guest spends fuel in its own code, so fuel bounds a guest that computes and
/// nothing else: only the epoch deadline ends a guest blocked inside a host call.
/// Tests hold a real guest inside `log` through this seam to prove the deadline is
/// live.
pub fn start_deployment_with<S, F>(
    packages: Vec<LoadedPackage>,
    limits: PluginLimits,
    queues: HostQueues,
    sessions: Arc<dyn PlayerSessions + Send + Sync>,
    services: F,
) -> Result<PluginHost, HostStartError>
where
    S: HostServices + 'static,
    F: Fn(&str) -> S + Send + 'static,
{
    let engine =
        crate::engine(&limits).map_err(|error| HostStartError::Engine(error.to_string()))?;
    let ticker = EpochTicker::start(engine.clone(), EPOCH_INTERVAL)
        .map_err(|error| HostStartError::Engine(format!("epoch watchdog: {error}")))?;
    let linker =
        crate::linker::<S>(&engine).map_err(|error| HostStartError::Engine(error.to_string()))?;
    let (mut boundary, mut endpoint) = script_boundary_pair(
        NonZeroUsize::new(queues.events.max(1)).expect("non-zero"),
        NonZeroUsize::new(queues.commands.max(1)).expect("non-zero"),
    );
    let reload_sender = boundary.host_input_sender();

    // The deployment's own declarations are settled before the first guest runs:
    // core reads the catalog through this boundary, the Loader stages the bundles,
    // and a world opens against exactly one ore profile.
    // The aggregation is the same one `check_deployment` performs, so a duplicate
    // declaration is refused identically on both paths.
    let surface =
        aggregate_deployment(&packages).map_err(|conflict| HostStartError::WorldgenConflict {
            kind: conflict.kind,
            first: conflict.first,
            second: conflict.second,
        })?;
    let manifests = packages
        .iter()
        .map(|package| package.manifest().clone())
        .collect::<Vec<_>>();
    // The catalog core's authored-data subsystems read is the host's own
    // discovery result, published on the boundary before it is handed on: a
    // package's directory and required features are what discovery validated, and
    // nothing here re-reads the manifest.
    let catalog = packages
        .iter()
        .map(LoadedPackage::to_plugin_package)
        .collect::<Vec<_>>();
    boundary.set_deployed_packages(catalog.clone());

    let mut precommit_hooks = packages
        .iter()
        .flat_map(|package| package.precommit_registrations().iter().cloned())
        .collect::<Vec<_>>();
    precommit_hooks.sort_by(mc_script::precommit::by_roster_order);
    boundary
        .set_precommit_hooks(precommit_hooks.clone())
        .map_err(|error| HostStartError::Engine(format!("pre-commit roster: {error}")))?;

    let mut hosted = Vec::new();
    let mut contribution = DeploymentContribution::default();
    for package in packages {
        let id = package.manifest().plugin_id().to_owned();
        let refuse = |message: String| HostStartError::Package {
            id: id.clone(),
            message,
        };
        let batch_limit = NonZeroUsize::new(limits.commands_per_call.max(1)).expect("non-zero");
        let compiled = crate::package::compile_package(&engine, &package, &limits)
            .map_err(|error| refuse(error.to_string()))?;
        let admission = HostCommandAdmission::from_manifest(package.manifest());
        let config = crate::check::read_config(package.root(), &limits).map_err(refuse)?;
        // The startup phase runs in a store of its own, and that store is dropped
        // here - before the runtime store below exists - so no state the guest
        // wrote while answering startup rules can reach `init`. The sequence is
        // the check's own, so the two paths cannot disagree about either phase.
        let configured = {
            let mut startup =
                PluginStartup::instantiate(&linker, compiled.component(), services(&id), limits)
                    .map_err(|error| refuse(error.to_string()))?;
            startup
                .configure(&config)
                .map_err(|error| refuse(error.to_string()))?
        };
        // The recorded contribution is this answer, not a second run of the
        // phase: a guest's `configure` is called exactly once per instance, and
        // the rules the server opens the world with have to be the rules this
        // instance answered.
        contribution.record(&id, configured.as_ref());
        if configured.is_some() {
            // Startup rules are materialized by the server, not by the host: P4
            // hands the contribution to the existing validators and `StartupData`.
            tracing::info!(plugin = %id, "startup contribution reported");
        }
        let mut instance =
            PluginInstance::instantiate(&linker, compiled.component(), services(&id), limits)
                .map_err(|error| refuse(error.to_string()))?;
        if package.manifest().is_permission_provider() {
            instance
                .state()
                .set_command_permissions(Some(boundary.command_permissions()));
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
        let mut diagnostics = InstanceDiagnostics::default();
        // `init` has no simulation clock yet. Legacy deployments keep its zero
        // origin; native-store deployments rebase this bounded initial schedule
        // once, when the server explicitly pushes its restored tick before any
        // lifecycle or result callback.
        let (batch, mutations) = batch
            .split_timers(0)
            .map_err(|error| refuse(timer_refusal(error)))?;
        // The timers `init` asked for are staged on a copy exactly as every later
        // answer's are, and this instance's own schedule starts with them.
        let timers = timers::stage(&TimerSchedule::default(), mutations)
            .map_err(|error| refuse(timer_refusal(error)))?
            .unwrap_or_default();
        // Nothing `init` asked for publishes before the whole opening preparation
        // has succeeded: the routes this instance answers on are registered and its
        // batch is admitted first, and the instance is kept only once both did. A
        // deployment refused here stops with neither the batch nor the staged
        // timers having reached the boundary.
        endpoint
            .register_plugin_routes(package.manifest())
            .map_err(|error| refuse(format!("{error:?}")))?;
        submit(
            &endpoint,
            &admission,
            batch,
            batch_limit,
            sessions.as_ref(),
            &mut diagnostics,
        )
        .map_err(|error| refuse(format!("{error:?}")))?;
        let commands = package
            .manifest()
            .player_command_roots()
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
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
            diagnostics,
            callback_latency: CallbackLatencyWindow::default(),
            limits,
            timers,
            observed_tick: 0,
            initial_timer_clock_pending: queues.defer_initial_timer_clock,
            retired: false,
        });
    }
    let reload_contract = PluginReloadContract::from_deployment(
        &manifests,
        catalog,
        precommit_hooks,
        surface.clone(),
        contribution.clone(),
    );

    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread_sessions = Arc::clone(&sessions);
    let thread = std::thread::Builder::new()
        .name("mc-plugin-host".to_owned())
        .spawn(move || {
            let sessions = thread_sessions;
            let mut hosted = hosted;
            let mut tick = 0_u64;
            while !thread_stop.load(Ordering::Acquire) {
                let Some(input) = endpoint.recv_input_blocking() else {
                    break;
                };
                let event = match input {
                    ScriptHostInput::Event(event) => event,
                    ScriptHostInput::Reload(payload) => {
                        let Ok(request) = payload.downcast::<PluginReloadRequest>() else {
                            tracing::warn!("component host dropped control input it does not own");
                            continue;
                        };
                        let PluginReloadRequest { packages, response } = *request;
                        let candidate = match build_reload_candidate(
                            packages,
                            &engine,
                            &linker,
                            limits,
                            ReloadGeneration {
                                current_tick: tick,
                                contract: &reload_contract,
                            },
                            sessions.as_ref(),
                            &services,
                        ) {
                            Ok(candidate) => candidate,
                            Err(error) => {
                                let _ = response.send(Err(error));
                                continue;
                            }
                        };
                        let PluginReloadCandidate {
                            hosted: replacement,
                            manifests,
                            batches,
                            permissions,
                        } = candidate;
                        let loaded_packages = replacement.len();
                        let mut replacement = Some(replacement);
                        let mut replaced = None;
                        match endpoint.commit_reload(&manifests, batches, || {
                            // No old callback can run after this synchronous
                            // worker cutover; shutdown must not publish either.
                            for previous in &mut hosted {
                                previous.instance.state().set_command_permissions(None);
                            }
                            let live_permissions = endpoint.command_permissions();
                            live_permissions.replace_from(&permissions);
                            for (candidate, manifest) in replacement
                                .as_mut()
                                .expect("candidate generation is present")
                                .iter_mut()
                                .zip(&manifests)
                            {
                                candidate.instance.state().set_command_permissions(
                                    manifest
                                        .is_permission_provider()
                                        .then(|| Arc::clone(&live_permissions)),
                                );
                            }
                            replaced = Some(std::mem::replace(
                                &mut hosted,
                                replacement
                                    .take()
                                    .expect("reload swap consumes the candidate generation"),
                            ));
                        }) {
                            Ok(()) => {
                                let replaced = shutdown_hosts(
                                    replaced.expect("a successful reload swaps the generation"),
                                );
                                let _ = response.send(Ok(PluginReloadReport {
                                    loaded_packages,
                                    replaced,
                                }));
                            }
                            Err(ScriptReloadCommitError::QueueClosed) => {
                                shutdown_hosts(
                                    replacement
                                        .take()
                                        .expect("a refused reload retains the candidate"),
                                );
                                let _ = response.send(Err(PluginReloadError::HostClosed));
                                break;
                            }
                            Err(ScriptReloadCommitError::QueueFull) => {
                                shutdown_hosts(
                                    replacement
                                        .take()
                                        .expect("a refused reload retains the candidate"),
                                );
                                let _ = response.send(Err(PluginReloadError::CommitRefused {
                                    message: "command queue is full".to_owned(),
                                }));
                            }
                            Err(ScriptReloadCommitError::Rejected { error }) => {
                                shutdown_hosts(
                                    replacement
                                        .take()
                                        .expect("a refused reload retains the candidate"),
                                );
                                let _ = response.send(Err(PluginReloadError::CommitRefused {
                                    message: format!("{error:?}"),
                                }));
                            }
                            Err(ScriptReloadCommitError::Ownership { error }) => {
                                shutdown_hosts(
                                    replacement
                                        .take()
                                        .expect("a refused reload retains the candidate"),
                                );
                                let _ = response.send(Err(PluginReloadError::CommitRefused {
                                    message: format!("{error:?}"),
                                }));
                            }
                            Err(error) => {
                                shutdown_hosts(
                                    replacement
                                        .take()
                                        .expect("a refused reload retains the candidate"),
                                );
                                let _ = response.send(Err(PluginReloadError::CommitRefused {
                                    message: format!("{error:?}"),
                                }));
                            }
                        }
                        continue;
                    }
                    ScriptHostInput::Precommit(request) => {
                        deliver_precommit(&mut hosted, &endpoint, request);
                        continue;
                    }
                    _ => continue,
                };
                // The tick a guest sees is the server's own simulation tick, taken
                // from the events that carry it. Counting deliveries instead would
                // make "every 20 ticks" mean twenty events.
                if let ScriptEventKind::ServerTick { tick: server_tick } = event.kind() {
                    tick = tick.max(*server_tick);
                    // A pushed simulation tick is the only clock a plugin's timers
                    // have. The tick is not an event of this contract - no
                    // subscription names it and no guest callback is called for it -
                    // so it reaches a guest only as the timers it made due, and
                    // every live instance is driven whether or not it subscribed to
                    // anything.
                    for hosted in &mut hosted {
                        if !hosted.retired {
                            deliver_timers(hosted, &endpoint, sessions.as_ref(), tick);
                        }
                    }
                    continue;
                }
                let Some((context, events)) = contract_events(&event, tick) else {
                    continue;
                };
                let name = event.event_name();
                let target = event.target_plugin_id();
                for hosted in &mut hosted {
                    if hosted.retired {
                        continue;
                    }
                    if let Some(target) = target {
                        if target != hosted.id {
                            continue;
                        }
                    } else if !interested(hosted, name, &event) {
                        continue;
                    }
                    deliver(hosted, &endpoint, sessions.as_ref(), context, &events);
                }
            }
            shutdown_hosts(hosted)
        })
        .map_err(|error| HostStartError::Engine(error.to_string()))?;

    Ok(PluginHost {
        boundary,
        stop,
        reload_sender,
        thread: Some(thread),
        contribution,
        surface,
        ticker: Some(ticker),
    })
}

pub(super) fn shutdown_hosts<S: HostServices + 'static>(
    mut hosted: Vec<Hosted<S>>,
) -> Vec<(String, InstanceDiagnostics)> {
    for candidate in &mut hosted {
        candidate.instance.state().set_command_permissions(None);
        if let Err(error) = candidate.instance.shutdown() {
            tracing::warn!(plugin = %candidate.id, %error, "shutdown failed");
        }
        candidate.diagnostics.callback_latency = candidate.callback_latency.snapshot();
    }
    hosted
        .into_iter()
        .map(|candidate| (candidate.id, candidate.diagnostics))
        .collect()
}

/// The command bound one submission enforces: the same count the staging area
/// already holds an answer to, never a second, larger number.
pub(super) fn batch_limit(limits: &PluginLimits) -> NonZeroUsize {
    NonZeroUsize::new(limits.commands_per_call.max(1)).expect("non-zero")
}
