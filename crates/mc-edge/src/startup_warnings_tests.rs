//! Regression tests for the startup configuration warnings.

use crate::config::EdgeConfig;
use crate::startup_warnings::{carries_secret, is_loopback_host, offline_listener_impersonable};

fn config(text: &str) -> EdgeConfig {
    EdgeConfig::from_toml(text, None).expect("test config parses")
}

#[test]
fn offline_public_listener_without_identity_check_is_reported() {
    let cfg = config("[network]\nbind_address = \"0.0.0.0\"\n");
    assert!(offline_listener_impersonable(&cfg, false));
}

#[test]
fn loopback_bind_is_never_reported() {
    for bind in ["127.0.0.1", "::1", "localhost", "127.0.0.53"] {
        let cfg = config(&format!("[network]\nbind_address = \"{bind}\"\n"));
        assert!(
            !offline_listener_impersonable(&cfg, false),
            "{bind} must count as loopback"
        );
    }
    assert!(!offline_listener_impersonable(
        &EdgeConfig::from_toml("", None).unwrap(),
        false
    ));
}

#[test]
fn identity_checks_silence_the_warning() {
    let public = "[network]\nbind_address = \"0.0.0.0\"\n";
    let online = config(&format!("{public}[auth]\nonline_mode = true\n"));
    assert!(!offline_listener_impersonable(&online, false));
    let velocity = config(&format!(
        "{public}[proxy]\nmode = \"velocity\"\nsecret = \"abc\"\n"
    ));
    assert!(!offline_listener_impersonable(&velocity, false));
    let plugin = config(public);
    assert!(!offline_listener_impersonable(&plugin, true));
    // The decision trusts the effective gate the caller resolved from the
    // core (which also turns it on for a deployed auth package); the raw
    // `auth.require_plugin_login` key is not read here.
    let flagged = config(&format!("{public}[auth]\nrequire_plugin_login = true\n"));
    assert!(offline_listener_impersonable(&flagged, false));
}

#[test]
fn loopback_host_recognizes_ip_literals_and_localhost_only() {
    assert!(is_loopback_host("127.0.0.1"));
    assert!(is_loopback_host("::1"));
    assert!(is_loopback_host("localhost"));
    assert!(is_loopback_host("LocalHost"));
    assert!(!is_loopback_host("0.0.0.0"));
    assert!(!is_loopback_host("::"));
    assert!(!is_loopback_host("mc.example.com"));
    assert!(!is_loopback_host("10.0.0.5"));
}

#[test]
fn every_secret_field_is_detected() {
    let plain = config("[network]\nbind_address = \"0.0.0.0\"\n");
    assert!(!carries_secret(&plain));
    let proxy =
        config("[auth]\nonline_mode = false\n[proxy]\nmode = \"velocity\"\nsecret = \"s\"\n");
    assert!(carries_secret(&proxy));
    let msmp = config("[msmp]\nenabled = true\nbearer_token = \"s\"\n");
    assert!(carries_secret(&msmp));
    let metrics = config("[dashboard]\nmetrics_bearer_token = \"s\"\n");
    assert!(carries_secret(&metrics));
    let empty_metrics = config("[dashboard]\nmetrics_bearer_token = \"\"\n");
    assert!(!carries_secret(&empty_metrics));
    let rcon = config("[rcon]\nenabled = true\npassword = \"s\"\n");
    assert!(carries_secret(&rcon));
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::startup_warnings::loose_file_mode;

    fn file_with_mode(mode: u32) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("server.toml");
        std::fs::write(&path, "[network]\nbind_address = \"0.0.0.0\"\n").expect("write config");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("set mode");
        (dir, path)
    }

    #[test]
    fn owner_only_config_is_not_reported() {
        for mode in [0o600, 0o400, 0o700] {
            let (_dir, path) = file_with_mode(mode);
            assert_eq!(loose_file_mode(&path), None, "{mode:03o} is owner-only");
        }
    }

    #[test]
    fn group_or_other_bits_are_reported() {
        for mode in [0o640, 0o604, 0o644, 0o660, 0o606, 0o777] {
            let (_dir, path) = file_with_mode(mode);
            assert_eq!(
                loose_file_mode(&path),
                Some(mode),
                "{mode:03o} must be reported"
            );
        }
    }

    #[test]
    fn a_config_path_records_the_loaded_file() {
        let (_dir, path) = file_with_mode(0o600);
        let cfg = EdgeConfig::from_toml("[network]\nbind_address = \"0.0.0.0\"\n", Some(&path))
            .expect("parses");
        assert_eq!(cfg.config_path.as_deref(), Some(path.as_path()));
        assert_eq!(
            EdgeConfig::from_toml("", None).unwrap().config_path,
            None,
            "defaults and library callers have no file"
        );
    }
}
