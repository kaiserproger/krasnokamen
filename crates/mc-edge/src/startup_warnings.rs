//! Startup configuration warnings.
//!
//! One function, called once from [`crate::server::EdgeServer::new`], reports
//! the configurations that serve correctly but leave the operator exposed: an
//! offline listener reachable from other hosts with nothing verifying
//! identities, and a secret-bearing config file readable by other local
//! users. Both are warnings, never refusals: LAN offline play is a supported
//! setup (owner decision, 2026-11-17).

use std::path::Path;

use mc_core::boundary::ConnectionId;
use tracing::warn;

use crate::config::{EdgeConfig, ProxyMode};

/// Report every finding once, at startup.
///
/// The login gate is read from the core, not only from `auth.require_plugin_login`:
/// a deployed auth-capable package turns the gate on by itself (`Core::open`).
pub fn warn_unsafe_startup(cfg: &EdgeConfig, core: &mc_core::Core) {
    let plugin_login = login_gate_active(core);
    if offline_listener_impersonable(cfg, plugin_login) {
        warn!(
            bind = %cfg.bind_addr(),
            "offline mode listening beyond loopback with no proxy forwarding or plugin login: \
             any client can claim any player name, operators included. Set auth.online_mode = \
             true, put a Velocity proxy in front (proxy.mode = \"velocity\"), or bind \
             network.bind_address to 127.0.0.1"
        );
    }
    for path in secret_files(cfg) {
        if let Some(mode) = loose_file_mode(path) {
            let mode = format!("{mode:03o}");
            warn!(
                file = %path.display(),
                mode = %mode,
                "file holds a secret (proxy.secret, msmp.bearer_token, \
                 dashboard.metrics_bearer_token or rcon.password) and is readable or writable \
                 by group/other; restrict it with chmod 600"
            );
        }
    }
}

/// Whether the core holds new sessions locked until a plugin unlocks them.
///
/// The edge's own fail-closed fence answers this for any connection the core
/// has never seen, and startup runs before the listener accepts one, so no id
/// can be unlocked yet.
fn login_gate_active(core: &mc_core::Core) -> bool {
    core.session_locked(ConnectionId(0))
}

/// `true` when the listener is offline, not loopback-bound, and nothing
/// (Velocity forwarding or a plugin login gate) verifies a claimed name.
#[must_use]
pub fn offline_listener_impersonable(cfg: &EdgeConfig, plugin_login: bool) -> bool {
    !cfg.auth.online_mode
        && !is_loopback_host(&cfg.network.bind_address)
        && cfg.proxy.mode == ProxyMode::None
        && !plugin_login
}

/// Loopback like the dashboard and MSMP bind rules: an IP literal on the
/// loopback interface, or the `localhost` name. Anything else may be reachable
/// from other hosts.
fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Any secret the TOML file itself carries; the values are never logged. An
/// effective `rcon.password` imported from a properties file is not the TOML's.
fn carries_secret(cfg: &EdgeConfig) -> bool {
    !cfg.proxy.secret.is_empty()
        || !cfg.msmp.bearer_token.is_empty()
        || cfg
            .dashboard
            .metrics_bearer_token
            .as_deref()
            .is_some_and(|token| !token.is_empty())
        || (!cfg.rcon.password.is_empty() && !cfg.rcon_password_imported)
}

/// Every loaded file holding a secret: the TOML when it carries one itself,
/// and an imported properties file with a non-empty `rcon.password`.
fn secret_files(cfg: &EdgeConfig) -> impl Iterator<Item = &Path> {
    cfg.config_path
        .as_deref()
        .filter(|_| carries_secret(cfg))
        .into_iter()
        .chain(cfg.secret_properties_path.as_deref())
}

/// The file's permission bits when group or other may read or write it;
/// always `None` off Unix, which has no such bits.
fn loose_file_mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).ok()?.permissions().mode() & 0o777;
        (mode & 0o077 != 0).then_some(mode)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
#[path = "startup_warnings_tests.rs"]
mod tests;
