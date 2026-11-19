//! The host runtime: one thread that owns the endpoint, the instances and the
//! mapping from the server's events to the contract's events.
//!
//! The server keeps producing `ScriptEvent`s on the boundary it already owns; this
//! loop is the only place a guest is called. It maps each event to the contract's
//! own shape, delivers it to the instances that subscribed, converts what they
//! answer back into server commands and submits those through the same admission
//! every other plugin command takes. A guest that traps, runs out of budget or
//! answers something invalid is retired *and* loses its command routes, so a dead
//! instance cannot keep answering with a stale registration.
//!
//! No package's phases share a store. `configure` runs first, in a store of its
//! own that is dropped before the runtime store exists; `init` and every callback
//! after it run in the runtime store the host keeps. The deployment's own
//! contributions - the package catalog core reads, the client bundles, and the two
//! worldgen profiles - are settled before the first guest runs, so a deployment
//! refused for two owners of one worldgen profile never half-starts.
//!
//! One pushed simulation tick is not an event of this contract and is not delivered
//! as one: it is the clock of the timers each instance owns, which only the host
//! can schedule, fire and cancel, and the loop drives those here, under the same
//! discipline every other callback takes.

use crate::bindings::exports::krasnokamen::plugin::events::{CommandInvoked, Event};
use crate::bindings::krasnokamen::plugin::operation_types::OperationFailure;
use crate::bindings::krasnokamen::plugin::types::LogLevel;
use crate::discovery::DiscoveryError;
use crate::package::LoadedPackage;
use crate::startup::{DeploymentContribution, DeploymentSurface};
use crate::timers::TimerSchedule;
use crate::{CommandBatch, EpochTicker, HostServices, PluginInstance, PluginLimits};
use mc_script::{
    HostCommandAdmission, ScriptBoundary, ScriptEvent, ScriptHostInputSender,
    ScriptOperationFailure, ScriptPlayerId, ValidatedScriptPluginManifest, script_boundary_pair,
};
use std::collections::{BTreeSet, VecDeque};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(test)]
use conversion::contract_events;

mod clock;
mod conversion;
mod delivery;
mod lifecycle;
mod precommit;
mod reload;

pub use lifecycle::{start_deployment, start_deployment_with};

/// Bounded capacities of one host: the same queues the boundary enforces, chosen
/// here so a deployment states them once.
#[derive(Debug, Clone, Copy)]
pub struct HostQueues {
    pub events: usize,
    pub commands: usize,
    /// Anchor timers created by `init` to the first explicitly pushed simulation
    /// tick. Native-store runtimes use this to restore their clock before any
    /// lifecycle or result callback; legacy runtimes retain the zero origin.
    pub defer_initial_timer_clock: bool,
}

impl Default for HostQueues {
    fn default() -> Self {
        Self {
            events: 1024,
            commands: 256,
            defer_initial_timer_clock: false,
        }
    }
}

/// How often the epoch watchdog advances. An expired deadline traps at the next
/// Wasm epoch check; it cannot preempt a native host import while that import runs.
pub const EPOCH_INTERVAL: Duration = Duration::from_millis(25);

const CALLBACK_LATENCY_SAMPLE_CAPACITY: usize = 1_200;

/// Percentiles from the bounded most-recent callback sample window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallbackLatencyPercentiles {
    pub samples: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

#[derive(Debug)]
struct CallbackLatencyWindow {
    samples: VecDeque<u64>,
}

impl Default for CallbackLatencyWindow {
    fn default() -> Self {
        Self {
            samples: VecDeque::with_capacity(CALLBACK_LATENCY_SAMPLE_CAPACITY),
        }
    }
}

impl CallbackLatencyWindow {
    fn record(&mut self, elapsed: Duration) {
        if self.samples.len() == CALLBACK_LATENCY_SAMPLE_CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(
            elapsed
                .as_micros()
                .min(u128::from(u64::MAX))
                .try_into()
                .expect("bounded microseconds fit u64"),
        );
    }

    fn snapshot(&self) -> CallbackLatencyPercentiles {
        let mut samples = self.samples.iter().copied().collect::<Vec<_>>();
        samples.sort_unstable();
        let Some(&max_us) = samples.last() else {
            return CallbackLatencyPercentiles::default();
        };
        let percentile = |percent: usize| {
            let rank = (percent * samples.len()).div_ceil(100);
            samples[rank.saturating_sub(1)]
        };
        CallbackLatencyPercentiles {
            samples: samples.len() as u64,
            p50_us: percentile(50),
            p95_us: percentile(95),
            p99_us: percentile(99),
            max_us,
        }
    }
}

/// Operator diagnostics of one hosted instance.
#[derive(Debug, Default)]
pub struct InstanceDiagnostics {
    /// Every guest `on-events` callback attempted by the runtime store.
    pub calls: u64,
    pub events_delivered: u64,
    pub commands_submitted: u64,
    pub commands_refused: u64,
    pub callback_latency: CallbackLatencyPercentiles,
}

/// Services a hosted guest gets: `tracing` under the plugin's own target, and the
/// id the host bound to it.
struct Tracing {
    id: String,
}

impl HostServices for Tracing {
    fn log(&mut self, level: LogLevel, message: &str) {
        match level {
            LogLevel::Error => tracing::error!(plugin = %self.id, "{message}"),
            LogLevel::Warn => tracing::warn!(plugin = %self.id, "{message}"),
            LogLevel::Info => tracing::info!(plugin = %self.id, "{message}"),
            LogLevel::Debug => tracing::debug!(plugin = %self.id, "{message}"),
            LogLevel::Trace => tracing::trace!(plugin = %self.id, "{message}"),
        }
    }

    fn plugin_id(&self) -> &str {
        &self.id
    }
}

/// One instance this host runs, with what it is allowed to answer.
struct Hosted<S: HostServices + 'static> {
    id: String,
    subscriptions: BTreeSet<String>,
    /// Player command roots this package claimed. A command is delivered to the
    /// package that owns its root, exactly as the server's own router does; it is
    /// not a subscription, so a package cannot receive commands it never declared.
    commands: BTreeSet<String>,
    admission: HostCommandAdmission,
    auth_provider: bool,
    instance: PluginInstance<S>,
    diagnostics: InstanceDiagnostics,
    callback_latency: CallbackLatencyWindow,
    limits: PluginLimits,
    /// The timers this package owns, and the simulation tick it last observed.
    ///
    /// Both are the host's memory, not the server's: the schedule is this
    /// instance's own, and a retired instance never advances either again. The
    /// observed tick only moves forward, whatever order the server's events arrive
    /// in and whatever the server coalesced, so a stale tick can neither fire a
    /// timer early nor have a tick already delivered fire again.
    timers: TimerSchedule,
    observed_tick: u64,
    initial_timer_clock_pending: bool,
    /// Set once a callback failed: the instance keeps its place for the final
    /// report, but it is never called again and it holds no routes.
    retired: bool,
}

impl<S: HostServices + 'static> Hosted<S> {
    fn record_callback(&mut self, started: Instant) {
        self.diagnostics.calls += 1;
        self.callback_latency.record(started.elapsed());
    }
}

/// Why a host could not start.
#[derive(Debug, thiserror::Error)]
pub enum HostStartError {
    /// The deployment was refused.
    #[error(transparent)]
    Deployment(#[from] DiscoveryError),
    /// The engine could not be built.
    #[error("host engine: {0}")]
    Engine(String),
    /// A package could not be compiled or started.
    #[error("plugin {id:?} failed to start: {message}")]
    Package { id: String, message: String },
    /// Two packages of one deployment declared the same worldgen profile.
    ///
    /// A world opens against exactly one ore profile, so
    /// a second declaration has no second slot. It is refused before any guest
    /// runs rather than resolved by keeping the later declaration. `kind` is the
    /// profile the two packages both declared.
    #[error("plugins {first} and {second} both declare a worldgen {kind} profile")]
    WorldgenConflict {
        kind: &'static str,
        first: String,
        second: String,
    },
}

/// The restart-only part of a component deployment's contract.
///
/// These values belong to the server's already-open world and routing boundary,
/// so changing one needs a full server restart rather than an in-place guest swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginReloadContractField {
    PluginIdentities,
    PackageCatalog,
    CommandAndChannelOwnership,
    CommandGrants,
    PrecommitRegistrations,
    DeploymentSurface,
    StartupContribution,
}

impl std::fmt::Display for PluginReloadContractField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::PluginIdentities => "ordered plugin identities",
            Self::PackageCatalog => "package catalog",
            Self::CommandAndChannelOwnership => "command and channel ownership",
            Self::CommandGrants => "effective command grants",
            Self::PrecommitRegistrations => "pre-commit registrations",
            Self::DeploymentSurface => "deployment surface",
            Self::StartupContribution => "startup contribution",
        })
    }
}

/// The outcome of replacing one live component generation.
#[derive(Debug)]
pub struct PluginReloadReport {
    /// Number of packages in the newly active generation.
    pub loaded_packages: usize,
    /// Final diagnostics from the generation retired after the switch.
    pub replaced: Vec<(String, InstanceDiagnostics)>,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginReloadError {
    /// The host has stopped, or stopped while the request waited in its FIFO.
    #[error("plugin host is closed")]
    HostClosed,
    /// The live and candidate stores together exceed the static reload budget.
    #[error(
        "reload candidate requires {requested_bytes} guest-memory bytes, exceeding the {limit_bytes}-byte budget"
    )]
    CandidateMemory {
        requested_bytes: usize,
        limit_bytes: usize,
    },
    /// The candidate changes a contract that the running server has already used.
    #[error("reload requires restart: {field} changed")]
    RestartContractChanged { field: PluginReloadContractField },
    /// A candidate package could not be fully configured, instantiated, or initialized.
    #[error("reload candidate setup refused: {message}")]
    CandidateSetup { message: String },
    /// The boundary refused the otherwise-ready candidate atomically.
    #[error("reload commit refused: {message}")]
    CommitRefused { message: String },
}

struct PluginReloadRequest {
    packages: Vec<LoadedPackage>,
    response: tokio::sync::oneshot::Sender<Result<PluginReloadReport, PluginReloadError>>,
}

/// One package's admission policy inside a reload contract.
#[derive(Clone, PartialEq, Eq)]
struct PackageRoutes {
    player_roots: Vec<String>,
    operator_roots: Vec<String>,
    channels: Vec<String>,
    /// Declared command root → permission node.
    command_permissions: Vec<(String, String)>,
    permission_provider: bool,
}

#[derive(Clone)]
struct PluginReloadContract {
    identities: Vec<String>,
    catalog: Vec<mc_script::PluginPackage>,
    routes: Vec<PackageRoutes>,
    grants: Vec<mc_script::CommandCapabilities>,
    precommit_hooks: Vec<mc_script::precommit::HookRegistration>,
    surface: DeploymentSurface,
    contribution: DeploymentContribution,
}

/// A running deployment.
pub struct PluginHost {
    boundary: ScriptBoundary,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Vec<(String, InstanceDiagnostics)>>>,
    reload_sender: ScriptHostInputSender,
    /// What each package's `configure` contributed, as the host recorded it while
    /// it started the deployment. The server reads it before it opens a world:
    /// startup rules are materialized by the world's owners, not by this host.
    contribution: DeploymentContribution,
    /// The deployment's own declarations - the client bundles and the two
    /// worldgen profiles - settled before the first guest ran.
    surface: DeploymentSurface,
    /// The watchdog of every store this host started. It lives exactly as long as
    /// the host does: dropping it earlier would leave every callback deadline
    /// armed against an epoch nobody advances.
    ticker: Option<EpochTicker>,
}

struct PluginReloadCandidate<S: HostServices + 'static> {
    hosted: Vec<Hosted<S>>,
    manifests: Vec<ValidatedScriptPluginManifest>,
    batches: Vec<(HostCommandAdmission, mc_script::CommandBatch)>,
    permissions: Arc<mc_script::ScriptCommandPermissions>,
}

/// The active generation facts a staged candidate must preserve.
struct ReloadGeneration<'a> {
    current_tick: u64,
    contract: &'a PluginReloadContract,
}

/// Why one callback's batch did not reach the server.
#[derive(Debug)]
enum SubmissionFailure {
    /// Nothing is wrong with the guest: the batch was refused for a state that
    /// can change - a full queue, a full admission ledger, a player who left.
    Transient(String),
    /// The guest's batch violated the contract.
    Contract(String),
}

/// The server's own reason an operation did not commit, in the contract's
/// vocabulary.
///
/// Every member of the server's enum has a member here, so the reasons stay
/// distinguishable: a batch refused for a stale revision is not reported as one
/// refused for a reused operation id, and a lookup that found nothing is not
/// reported as a commit. The server's enum is `non_exhaustive`, and a reason this
/// contract does not name yet is reported as `unknown` rather than as one of the
/// reasons that mean something else: either way the operation did not commit.
pub(crate) fn operation_failure(failure: ScriptOperationFailure) -> OperationFailure {
    match failure {
        ScriptOperationFailure::InvalidRequest => OperationFailure::InvalidRequest,
        ScriptOperationFailure::Forbidden => OperationFailure::Forbidden,
        ScriptOperationFailure::StaleRevision => OperationFailure::StaleRevision,
        ScriptOperationFailure::NotFound => OperationFailure::NotFound,
        ScriptOperationFailure::Unloaded => OperationFailure::Unloaded,
        ScriptOperationFailure::Blocked => OperationFailure::Blocked,
        ScriptOperationFailure::InsufficientItems => OperationFailure::InsufficientItems,
        ScriptOperationFailure::Capacity => OperationFailure::Capacity,
        ScriptOperationFailure::Busy => OperationFailure::Busy,
        ScriptOperationFailure::RuntimeUnavailable => OperationFailure::RuntimeUnavailable,
        ScriptOperationFailure::OperationConflict => OperationFailure::OperationConflict,
        ScriptOperationFailure::CursorExpired => OperationFailure::CursorExpired,
        _ => OperationFailure::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E5: the publication import is the provider's alone, and a refused entry
    /// leaves the table command admission reads untouched.
    #[test]
    fn only_the_provider_handle_publishes_command_permissions() {
        use crate::bindings::krasnokamen::plugin::command_permissions::Host as HostImport;

        const PLAYER: &str = "069a79f4-44e9-4726-a5be-fca90e38aaf5";
        let (boundary, endpoint) =
            script_boundary_pair(NonZeroUsize::new(1).unwrap(), NonZeroUsize::new(1).unwrap());
        let provider = mc_script::ScriptPluginManifest::new(
            "krasnokamen-permissions",
            "Permissions",
            "0.1.0",
            mc_script::COMPONENT_PLUGIN_API_VERSION,
        )
        .declare_player_command_root("perm")
        .declare_permission_provider()
        .validate()
        .unwrap();
        endpoint.register_plugin_routes(&provider).unwrap();

        let limits = PluginLimits::default();
        let mut ordinary = crate::InstanceState::new(
            Tracing {
                id: "krasnokamen-essentials".into(),
            },
            &limits,
        );
        assert!(!HostImport::publish(
            &mut ordinary,
            vec![format!("{PLAYER} krasnokamen.essentials.use")]
        ));
        assert!(
            !boundary
                .command_permissions()
                .allows(PLAYER, "krasnokamen.essentials.use")
        );

        let mut published = crate::InstanceState::new(
            Tracing {
                id: "krasnokamen-permissions".into(),
            },
            &limits,
        );
        published.set_command_permissions(Some(boundary.command_permissions()));
        assert!(HostImport::publish(
            &mut published,
            vec![format!("{PLAYER} krasnokamen.essentials.use")]
        ));
        assert!(
            boundary
                .command_permissions()
                .allows(PLAYER, "krasnokamen.essentials.use")
        );
        assert!(!HostImport::publish(
            &mut published,
            vec!["not an entry".into()]
        ));
        assert!(
            boundary
                .command_permissions()
                .allows(PLAYER, "krasnokamen.essentials.use")
        );
    }

    #[test]
    fn operator_root_change_requires_command_ownership_restart() {
        let active_manifest = mc_script::ScriptPluginManifest::new(
            "clock",
            "Clock",
            "0.1.0",
            mc_script::COMPONENT_PLUGIN_API_VERSION,
        )
        .declare_operator_command_root("clock_status")
        .validate()
        .unwrap();
        let candidate_manifest = mc_script::ScriptPluginManifest::new(
            "clock",
            "Clock",
            "0.2.0",
            mc_script::COMPONENT_PLUGIN_API_VERSION,
        )
        .declare_operator_command_root("clock_daytime")
        .validate()
        .unwrap();
        let active = PluginReloadContract::from_deployment(
            &[active_manifest],
            Vec::new(),
            Vec::new(),
            DeploymentSurface::default(),
            DeploymentContribution::default(),
        );
        let candidate = PluginReloadContract::from_deployment(
            &[candidate_manifest],
            Vec::new(),
            Vec::new(),
            DeploymentSurface::default(),
            DeploymentContribution::default(),
        );

        assert_eq!(
            active.incompatibility(&candidate),
            Some(PluginReloadContractField::CommandAndChannelOwnership)
        );
    }

    #[test]
    fn command_events_carry_server_operator_authority() {
        let event = ScriptEvent::try_player_command_with_context(
            "permissions",
            ScriptPlayerId::new(7),
            mc_script::ScriptPlayerContext::new(
                "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                "Ada",
                true,
                "minecraft:the_nether",
                0.0,
                64.0,
                0.0,
            ),
            "perm",
            "set admin",
        )
        .expect("bounded server command event");

        let (_, events) = contract_events(&event, 42).expect("component event");
        let [Event::CommandInvoked(invoked)] = events.as_slice() else {
            panic!("expected one command event");
        };
        assert_eq!(invoked.player, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        assert_eq!(invoked.session, 7);
        assert_eq!(invoked.username, "Ada");
        assert!(invoked.operator);
        assert_eq!(invoked.dimension, "minecraft:the_nether");
        assert_eq!(invoked.position.x, 0.0);
        assert_eq!(invoked.position.y, 64.0);
        assert_eq!(invoked.position.z, 0.0);
        assert_eq!(invoked.name, "perm");
        assert_eq!(invoked.arguments, ["set", "admin"]);
        assert_eq!(invoked.raw_arguments, "set admin");
    }

    #[test]
    fn callback_latency_window_reports_bounded_nearest_rank_percentiles() {
        let mut window = CallbackLatencyWindow::default();
        for micros in 0..=CALLBACK_LATENCY_SAMPLE_CAPACITY {
            window.record(Duration::from_micros(micros as u64));
        }

        assert_eq!(
            window.snapshot(),
            CallbackLatencyPercentiles {
                samples: CALLBACK_LATENCY_SAMPLE_CAPACITY as u64,
                p50_us: 600,
                p95_us: 1_140,
                p99_us: 1_188,
                max_us: 1_200,
            }
        );
    }
}
