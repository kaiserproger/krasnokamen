use super::{
    http::serve_connection,
    latency::tps_of,
    prometheus::prometheus,
    routing::{parse_head, parse_load_query},
};

use super::*;
use tokio::io::duplex;

fn test_state() -> Arc<DashboardState> {
    Arc::new(DashboardState::new(DashboardWorld {
        name: "t".to_owned(),
        motd: "m".to_owned(),
        seed: 81,
        view_distance: 4,
        simulation_distance: 3,
        max_players: 20,
    }))
}

#[test]
fn visibility_observation_keeps_its_own_source_tick() {
    let state = test_state();
    state.record_pump_tick(PumpTick {
        tick: 42,
        ..Default::default()
    });
    state.record_coreperf_visibility(41, 11);
    let snapshot = state.snapshot();
    assert_eq!(snapshot.simulation_tick, 42);
    let visible = snapshot.coreperf_visibility.unwrap();
    assert_eq!(visible.source_tick, 41);
    assert_eq!(visible.tracked_entity_viewer_pairs, 11);
}

#[test]
fn visibility_capture_is_one_shot_per_dashboard_lifetime() {
    let state = test_state();
    state
        .coreperf_visibility_requested
        .store(0, Ordering::Release);
    assert!(
        state
            .coreperf_visibility_requested
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    );
    assert!(state.take_coreperf_visibility_request());
    assert!(!state.take_coreperf_visibility_request());
    assert!(
        state
            .coreperf_visibility_requested
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    );
}

#[test]
fn load_sample_retains_exact_coreperf_tick_and_region_counts() {
    let state = test_state();
    let coverage = mc_core::boundary::CorePerfTick {
        tick: 41,
        resident_entities: 12,
        xp_orbs_processed: 0,
        regions: vec![mc_core::boundary::CorePerfRegion {
            region: 7,
            active_chunks: 16,
            motion_admitted: 12,
            motion_full: 5,
            motion_resting: 7,
            projectile_candidates: 0,
        }],
        ..Default::default()
    };
    state.record_load_tick(
        41,
        TickTimes {
            coreperf: Some(Arc::new(coverage)),
            ..Default::default()
        },
    );
    let snapshot = state.load_snapshot(None).unwrap();
    let json = serde_json::to_value(&snapshot.samples[0]).unwrap();
    assert_eq!(json["tick"], 41);
    assert_eq!(json["coreperf"]["tick"], 41);
    assert_eq!(json["coreperf"]["regions"][0]["region"], 7);
    assert_eq!(json["coreperf"]["regions"][0]["motion_admitted"], 12);
}

#[test]
fn load_samples_preserve_source_times_and_gaps_without_changing_stats() {
    let state = test_state();
    let times = TickTimes {
        wall_start_unix_ns: 1_800_000_000_123_456_789,
        wall_us: 99,
        total_us: 45,
        entity_us: 11,
        merge_us: 12,
        block_us: 13,
        snapshot_us: 14,
        views_us: 15,
        input_us: 16,
        chunks_us: 17,
        commit_us: 18,
        publish_us: 19,
        plugins_us: 20,
        coordination_us: 21,
        journal_build_us: 22,
        journal_queue_us: 23,
        generated_us: 24,
        owner_pre_us: 25,
        owner_post_us: 26,
        coreperf: None,
        battle: None,
        published_at: None,
    };
    state.record_load_tick(41, times.clone());
    state.record_load_skipped(2);
    state.record_load_tick(
        44,
        TickTimes {
            wall_start_unix_ns: times.wall_start_unix_ns + 987_654_321,
            ..times.clone()
        },
    );
    let snapshot = state.load_snapshot(Some(40)).unwrap();
    assert_eq!(snapshot.oldest_tick, Some(41));
    assert_eq!(snapshot.latest_tick, Some(44));
    assert_eq!(snapshot.source_skipped_ticks_total, 2);
    assert!(!snapshot.overrun);
    assert!(snapshot.queues.is_none());
    assert_eq!(snapshot.samples.len(), 2);
    assert_eq!(
        snapshot.samples[0].wall_start_unix_ns,
        times.wall_start_unix_ns
    );
    assert_eq!(snapshot.samples[0].wall_us, 99);
    assert_eq!(snapshot.samples[0].total_us, 45);
    assert_eq!(snapshot.samples[0].phase_us.entity_us, 11);
    assert_eq!(snapshot.samples[0].phase_us.merge_us, 12);
    assert_eq!(snapshot.samples[0].phase_us.block_us, 13);
    assert_eq!(snapshot.samples[0].phase_us.snapshot_us, 14);
    assert_eq!(snapshot.samples[0].phase_us.views_us, 15);
    let phases = serde_json::to_value(snapshot.samples[0].phase_us).unwrap();
    for (name, expected) in [
        ("input_us", 16),
        ("chunks_us", 17),
        ("commit_us", 18),
        ("publish_us", 19),
        ("plugins_us", 20),
        ("coordination_us", 21),
        ("journal_build_us", 22),
        ("journal_queue_us", 23),
        ("generated_us", 24),
        ("owner_pre_us", 25),
        ("owner_post_us", 26),
    ] {
        assert_eq!(phases[name], expected, "{name}");
    }
    assert_eq!(snapshot.samples[1].tick, 44);
    assert_eq!(
        snapshot.samples[1].wall_start_unix_ns,
        times.wall_start_unix_ns + 987_654_321
    );
    assert_eq!(state.snapshot().tick.samples, 0);
    assert_eq!(state.load_snapshot(Some(44)).unwrap().samples.len(), 0);
}

#[test]
fn load_phases_include_the_complete_tick_envelope() {
    let state = test_state();
    state.record_load_tick(1, TickTimes::default());
    let snapshot = state.load_snapshot(None).unwrap();
    let value = serde_json::to_value(&snapshot.samples[0]).unwrap();
    for phase in [
        "input_us",
        "chunks_us",
        "commit_us",
        "publish_us",
        "plugins_us",
        "coordination_us",
        "journal_build_us",
        "journal_queue_us",
        "generated_us",
        "owner_pre_us",
        "owner_post_us",
    ] {
        assert!(value["phase_us"][phase].is_u64(), "missing phase {phase}");
    }
}

#[test]
fn load_ring_is_bounded_and_reports_only_requested_overwrite() {
    let state = test_state();
    for tick in 1..=LOAD_SAMPLE_CAPACITY as u64 + 2 {
        state.record_load_tick(tick, TickTimes::default());
    }
    let snapshot = state.load_snapshot(Some(1)).unwrap();
    assert_eq!(snapshot.capacity, LOAD_SAMPLE_CAPACITY);
    assert_eq!(snapshot.samples.len(), LOAD_SAMPLE_CAPACITY);
    assert_eq!(snapshot.oldest_tick, Some(3));
    assert_eq!(snapshot.latest_tick, Some(LOAD_SAMPLE_CAPACITY as u64 + 2));
    assert_eq!(snapshot.overwritten_samples_total, 2);
    assert!(snapshot.overrun);
    assert!(!state.load_snapshot(Some(2)).unwrap().overrun);
    assert!(!state.load_snapshot(None).unwrap().overrun);
}

#[test]
fn load_query_is_strict_and_unsigned() {
    let default = LoadMetricsQuery {
        after_tick: None,
        wait: false,
        min_samples: 1,
    };
    assert_eq!(parse_load_query(None), Ok(default));
    assert_eq!(parse_load_query(Some("")).unwrap().after_tick, None);
    assert_eq!(
        parse_load_query(Some("after_tick=0")).unwrap().after_tick,
        Some(0)
    );
    assert_eq!(
        parse_load_query(Some("after_tick=18446744073709551615"))
            .unwrap()
            .after_tick,
        Some(u64::MAX)
    );
    assert_eq!(
        parse_load_query(Some("after_tick=42&wait=true&min_samples=20")),
        Ok(LoadMetricsQuery {
            after_tick: Some(42),
            wait: true,
            min_samples: 20
        })
    );
    assert_eq!(
        parse_load_query(Some("wait=false&min_samples=1200"))
            .unwrap()
            .min_samples,
        1200
    );
    for query in [
        "after_tick=",
        "after_tick=-1",
        "after_tick=+1",
        "after_tick=18446744073709551616",
        "after_tick=1&after_tick=2",
        "after_tick=1&unknown=2",
        "after_tick",
        "wait=1",
        "wait=true&wait=false",
        "min_samples=0",
        "min_samples=1201",
        "min_samples=-1",
        "min_samples=1&min_samples=2",
    ] {
        assert_eq!(parse_load_query(Some(query)), Err(()), "{query}");
    }
}

#[test]
fn load_wait_is_push_driven_and_notifies_every_reader_after_batch() {
    use std::task::{Context, Poll, Waker};

    let state = test_state();
    let mut first = std::pin::pin!(state.wait_for_load_samples(Some(40), 2, false));
    let mut second = std::pin::pin!(state.wait_for_load_samples(Some(40), 2, false));
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(first.as_mut().poll(&mut context), Poll::Pending);
    assert_eq!(second.as_mut().poll(&mut context), Poll::Pending);
    state.record_load_tick(41, TickTimes::default());
    assert_eq!(first.as_mut().poll(&mut context), Poll::Pending);
    assert_eq!(second.as_mut().poll(&mut context), Poll::Pending);
    state.record_load_tick(42, TickTimes::default());
    assert_eq!(first.as_mut().poll(&mut context), Poll::Ready(Some(())));
    assert_eq!(second.as_mut().poll(&mut context), Poll::Ready(Some(())));
    let mut already_ready = std::pin::pin!(state.wait_for_load_samples(Some(40), 2, false));
    assert_eq!(
        already_ready.as_mut().poll(&mut context),
        Poll::Ready(Some(()))
    );
}

#[tokio::test]
async fn load_route_streams_raw_json_and_rejects_bad_cursor() {
    let state = test_state();
    let connections = Arc::new(ConnectionRegistry::new());
    state.set_queue_source(connections);
    state.record_load_tick(
        7,
        TickTimes {
            wall_start_unix_ns: 1_800_000_000_123_456_789,
            wall_us: 123,
            total_us: 45,
            ..Default::default()
        },
    );
    for (target, expected_status) in [
        ("/load-metrics?after_tick=6", "200 OK"),
        (
            "/load-metrics?after_tick=6&wait=true&min_samples=1",
            "200 OK",
        ),
        ("/load-metrics?after_tick=-1", "400 Bad Request"),
    ] {
        let (mut client, mut server) = duplex(8192);
        client
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .await
            .unwrap();
        serve_connection(&mut server, &state).await;
        drop(server);
        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with(&format!("HTTP/1.1 {expected_status}")));
        if expected_status == "200 OK" {
            let body = text.split_once("\r\n\r\n").unwrap().1;
            let value: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(value["schema_version"], 1);
            assert_eq!(value["wait_timed_out"], false);
            assert_eq!(value["samples"][0]["tick"], 7);
            assert_eq!(value["samples"][0]["wall_us"], 123);
            assert_eq!(
                value["samples"][0]["wall_start_unix_ns"].as_u64(),
                Some(1_800_000_000_123_456_789)
            );
            assert_eq!(value["queues"]["capabilities"]["queued_bytes"], false);
            assert!(value["queues"]["reliable"]["queued_bytes"].is_null());
            assert!(value["queues"]["world"]["reserved_slots"].is_null());
        }
    }
}

/// One line per retained, fanned-out tick after the cursor, joined by
/// tick number with the host sample and recorder stats of that tick;
/// gaps stay gaps. Battle counts appear only on ticks that carry them,
/// keyed by faction. A tick whose fanout has not finished waits.
#[tokio::test]
async fn telemetry_jsonl_joins_host_samples_by_tick() {
    let state = test_state();
    let host = HostSample {
        sampled_unix_ns: 1_800_000_000_500_000_000,
        cgroup_memory_current_bytes: Some(3 << 30),
        cgroup_cpu_usage_usec: Some(42_000_000),
        cgroup_cpu_nr_throttled: Some(7),
        cgroup_cpu_throttled_usec: Some(1_500),
        cgroup_memory_pressure: Some(crate::host_telemetry::MemoryPressure {
            some_avg10_centi: Some(45),
            some_total_usec: Some(42),
            full_avg10_centi: None,
            full_total_usec: None,
        }),
        cpu_total_ticks: Some(9_000),
        cpu_steal_ticks: Some(12),
        process_rss_bytes: None,
    };
    let battle = std::sync::Arc::new(mc_core::boundary::BattleCounts {
        units: 20_000,
        alive: vec![(1, 9_870), (2, 0)],
        in_weapon_reach: 1_234,
        fighting: 987,
        acting: 321,
        strikes: 47,
        shields_raised: 654,
        shield_blocks: 32,
        weapon_swaps: 21,
        holding_spear: 4_000,
        holding_sword: 3_000,
        holding_bow: 2_000,
    });
    for tick in [19, 20, 21, 23] {
        state.record_load_tick(
            tick,
            TickTimes {
                wall_start_unix_ns: 1_800_000_000_000_000_000 + tick * 50_000_000,
                wall_us: tick * 10,
                battle: (tick == 21).then(|| std::sync::Arc::clone(&battle)),
                ..Default::default()
            },
        );
        if tick == 20 {
            state.record_host_sample(tick, host);
        }
    }
    state.record_host_sample(40, host);
    state.record_fanout_tick(20, None);
    let deferred = RecorderTick {
        connections: 1,
        deferred_updates_sum: 7,
        deferred_updates_max: 7,
    };
    state.record_fanout_tick(21, Some(deferred));
    let (mut client, mut server) = duplex(8192);
    client
        .write_all(b"GET /telemetry.jsonl?after_tick=19 HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    serve_connection(&mut server, &state).await;
    drop(server);
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("HTTP/1.1 200 OK"));
    assert!(text.contains("application/x-ndjson"));
    let lines: Vec<serde_json::Value> = text
        .split_once("\r\n\r\n")
        .unwrap()
        .1
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let ticks: Vec<_> = lines.iter().map(|line| line["tick"].as_u64()).collect();
    assert_eq!(ticks, [Some(20), Some(21)], "tick 23 is not fanned out yet");
    assert_eq!(lines[0]["wall_us"], 200);
    assert_eq!(lines[0]["phase_us"]["entity_us"], 0);
    assert_eq!(lines[0]["host"]["cgroup_memory_current_bytes"], 3u64 << 30);
    assert_eq!(lines[0]["host"]["cpu_steal_ticks"], 12);
    // Saturation travels with the sample: throttle counters and PSI memory pressure.
    assert_eq!(lines[0]["host"]["cgroup_cpu_nr_throttled"], 7);
    assert_eq!(lines[0]["host"]["cgroup_cpu_throttled_usec"], 1_500);
    assert_eq!(
        lines[0]["host"]["cgroup_memory_pressure"]["some_avg10_centi"],
        45
    );
    assert!(lines[0]["host"]["cgroup_memory_pressure"]["full_avg10_centi"].is_null());
    assert!(lines[0]["host"]["process_rss_bytes"].is_null());
    assert!(lines[1]["host"].is_null());
    assert_eq!(
        lines[1]["battle"],
        serde_json::json!({"units": 20_000, "alive": {"1": 9_870, "2": 0},
                "in_weapon_reach": 1_234, "fighting": 987, "acting": 321, "strikes": 47, "shields_raised": 654,
                "shield_blocks": 32, "weapon_swaps": 21, "holding_spear": 4_000,
                "holding_sword": 3_000, "holding_bow": 2_000})
    );
    assert!(lines[0].get("battle").is_none());
    assert_eq!(
        lines[1]["recorders"],
        serde_json::json!({
            "connections": 1,
            "deferred_updates_sum": 7,
            "deferred_updates_max": 7
        })
    );
    assert!(lines[0].get("recorders").is_none());

    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let mut waiting = std::pin::pin!(state.wait_for_load_samples(Some(21), 1, true));
    assert!(waiting.as_mut().poll(&mut context).is_pending());
    state.record_fanout_tick(
        23,
        Some(RecorderTick {
            connections: 2,
            ..RecorderTick::default()
        }),
    );
    assert!(waiting.as_mut().poll(&mut context).is_ready());
    let line: serde_json::Value =
        serde_json::from_slice(&state.telemetry_jsonl(Some(21)).unwrap()).unwrap();
    assert_eq!(line["tick"], 23);
    assert!(line.get("battle").is_none());
    assert_eq!(line["recorders"]["connections"], 2);
    assert_eq!(line["recorders"]["deferred_updates_max"], 0);
    assert_eq!(
        state.telemetry_jsonl(Some(23)).unwrap(),
        Vec::<u8>::new(),
        "nothing after the newest tick"
    );
}

/// A `/telemetry.jsonl` reader woken by a core tick already sees that
/// tick's host sample: it is stored before the tick wakes anyone.
#[test]
fn woken_telemetry_reader_sees_the_host_sample_of_its_tick() {
    struct ReadOnWake {
        state: Arc<DashboardState>,
        seen: std::sync::Mutex<Option<Vec<u8>>>,
    }
    impl std::task::Wake for ReadOnWake {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            *self.seen.lock().unwrap() = self.state.telemetry_jsonl(Some(39));
        }
    }
    let state = test_state();
    state.record_fanout_tick(40, None);
    let reader = Arc::new(ReadOnWake {
        state: Arc::clone(&state),
        seen: std::sync::Mutex::default(),
    });
    let waker = std::task::Waker::from(Arc::clone(&reader));
    let mut context = std::task::Context::from_waker(&waker);
    let mut waiting = std::pin::pin!(state.wait_for_load_samples(Some(39), 1, true));
    assert!(waiting.as_mut().poll(&mut context).is_pending());
    let mut host = crate::host_telemetry::HostSampler::new();
    state.record_core_tick(&mut host, 40, TickTimes::default());
    let seen = reader
        .seen
        .lock()
        .unwrap()
        .take()
        .expect("the tick woke the reader");
    let line: serde_json::Value = serde_json::from_slice(&seen).unwrap();
    assert_eq!(line["tick"], 40);
    assert!(
        line["host"]["sampled_unix_ns"]
            .as_u64()
            .is_some_and(|ns| ns > 0),
        "{line}"
    );
}

#[test]
fn tick_percentiles_report_core_cost_and_publish_time() {
    let state = test_state();
    // Core cost 1..=100 ms, publish 1/10 of it; arrival gaps are ~0 here,
    // so any percentile that leaked from the arrival window would show.
    for ms in 1..=100u64 {
        state.record_pump_tick(PumpTick {
            tick: ms,
            core_us: ms * 1000,
            publish_us: ms * 100,
            ..Default::default()
        });
    }
    let snap = state.snapshot();
    assert_eq!(snap.simulation_tick, 100);
    assert_eq!(snap.tick.samples, 100);
    assert_eq!(snap.tick.p50_us, 50_000);
    assert_eq!(snap.tick.p95_us, 95_000);
    assert_eq!(snap.tick.max_us, 100_000);
    assert_eq!(snap.publish.p99_us, 9_900);
    assert!(!snap.autoscale);
    assert!(snap.warnings.is_empty());
}

#[test]
fn tps_counts_ticks_the_pump_skipped() {
    let at = Instant::now();
    let second = std::time::Duration::from_secs(1);
    // The pump saw only ticks 0 and 20 one second apart (19 skipped):
    // the core still ran at 20 TPS.
    let arrivals = VecDeque::from([(0, at), (20, at + second)]);
    assert!((tps_of(&arrivals) - 20.0).abs() < 1e-6);
    let slow = VecDeque::from([(0, at), (5, at + second)]);
    assert!((tps_of(&slow) - 5.0).abs() < 1e-6);
    assert_eq!(tps_of(&VecDeque::from([(7, at)])), 0.0);
}

#[test]
fn parse_head_preserves_query_for_load_route() {
    let req = parse_head("GET /stats?x=1 HTTP/1.1\r\nHost: a\r\n\r\n").unwrap();
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/stats");
    assert_eq!(req.query.as_deref(), Some("x=1"));
}

#[tokio::test]
async fn stats_route_serves_json() {
    let state = test_state();
    let (mut client, mut server) = duplex(8192);
    client
        .write_all(b"GET /stats HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    serve_connection(&mut server, &state).await;
    drop(server);
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
    assert!(text.contains("\"simulation_tick\""));
}

#[tokio::test]
async fn tps_route_serves_tps_and_mspt_json() {
    let state = test_state();
    state.record_pump_tick(PumpTick {
        tick: 1,
        core_us: 10_000,
        ..Default::default()
    });
    let (mut client, mut server) = duplex(8192);
    client
        .write_all(b"GET /tps HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    serve_connection(&mut server, &state).await;
    drop(server);
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
    let body = text.split_once("\r\n\r\n").unwrap().1;
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["tick"], 1);
    assert!(json["tps"].is_number());
    assert!(json["mspt"].is_number());
}

#[tokio::test]
async fn unknown_route_is_404() {
    let state = test_state();
    let (mut client, mut server) = duplex(8192);
    client
        .write_all(b"GET /nope HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    serve_connection(&mut server, &state).await;
    drop(server);
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    assert!(text.starts_with("HTTP/1.1 404 Not Found"), "{text}");
}

#[tokio::test]
async fn profile_route_carries_schema_marker() {
    let state = test_state();
    let (mut client, mut server) = duplex(8192);
    client
        .write_all(b"GET /profile HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    serve_connection(&mut server, &state).await;
    drop(server);
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
    assert!(text.contains("\"profile_schema\":1"));
    assert!(text.contains("\"captured_unix_ms\""));
}
#[test]
fn prometheus_exposes_the_dashboard_operator_gauges() {
    let state = test_state();
    state.record_pump_tick(PumpTick {
        tick: 1,
        core_us: 10_000,
        bytes_written: 44,
        chunks: DashboardChunks { loaded_total: 3 },
        entities: DashboardEntities { total: 7 },
        players: Some(DashboardPlayers {
            count: 2,
            max: 20,
            names: Vec::new(),
        }),
        ..Default::default()
    });
    let metrics = prometheus(&state);
    for name in [
        "krasnokamen_tps",
        "krasnokamen_tick_duration_microseconds{quantile=\"0.50\"}",
        "krasnokamen_tick_duration_microseconds{quantile=\"0.95\"}",
        "krasnokamen_tick_duration_microseconds{quantile=\"0.99\"}",
        "krasnokamen_players 2",
        "krasnokamen_chunks_loaded 3",
        "krasnokamen_entities 7",
        "krasnokamen_network_bytes_written_total 44",
    ] {
        assert!(metrics.contains(name), "missing {name}: {metrics}");
    }
}

#[test]
fn journal_prune_failures_reach_the_operator_surfaces() {
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("journal");
    std::fs::create_dir_all(&journal).unwrap();
    // Parses as a covered batch but cannot be removed: a directory.
    std::fs::create_dir(journal.join("batch-00000000000000000001-00000000000000000001.bin"))
        .unwrap();
    mc_core::journal::prune_batches_through(&journal, 5);
    let stats = mc_core::journal::journal_prune_stats();
    assert!(stats.failures_total > 0, "the refusal is counted");
    let metrics = prometheus(&state);
    for name in [
        "krasnokamen_journal_prune_failures_total",
        "krasnokamen_journal_prune_retained_bytes",
    ] {
        assert!(metrics.contains(name), "missing {name}: {metrics}");
    }
    assert!(
        metrics.contains(&format!(
            "krasnokamen_journal_prune_failures_total {}\n",
            stats.failures_total
        )),
        "the scrape reports the current count: {metrics}"
    );
    assert_eq!(
        state.snapshot().journal.prune_failures_total,
        stats.failures_total
    );
}

#[tokio::test]
async fn metrics_route_honors_optional_bearer_token() {
    let state = test_state();
    state.set_metrics_bearer_token(Some("secret".into()));
    for (request, status) in [
        (
            "GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n",
            "401 Unauthorized",
        ),
        (
            "GET /metrics HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret\r\n\r\n",
            "200 OK",
        ),
    ] {
        let (mut client, mut server) = duplex(8192);
        client.write_all(request.as_bytes()).await.unwrap();
        serve_connection(&mut server, &state).await;
        drop(server);
        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with(&format!("HTTP/1.1 {status}")), "{text}");
    }
}

#[tokio::test]
async fn coreperf_capture_authorizes_before_consumption_and_waits_for_publication() {
    let state = test_state();
    state
        .coreperf_visibility_requested
        .store(u8::MAX, Ordering::Release);
    let (mut client, mut server) = duplex(8192);
    client
        .write_all(b"GET /coreperf-visibility-proof HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    serve_connection(&mut server, &state).await;
    drop(server);
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    assert!(out.starts_with(b"HTTP/1.1 404 Not Found"));

    state
        .coreperf_visibility_requested
        .store(0, Ordering::Release);
    for (token, configured) in [(Some("secret"), false), (None, true), (Some("wrong"), true)] {
        if configured {
            state.set_metrics_bearer_token(Some("secret".into()));
        }
        let (mut client, mut server) = duplex(8192);
        let auth = token
            .map(|token| format!("Authorization: Bearer {token}\r\n"))
            .unwrap_or_default();
        client
            .write_all(
                format!("GET /coreperf-visibility-proof HTTP/1.1\r\nHost: x\r\n{auth}\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        serve_connection(&mut server, &state).await;
        drop(server);
        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        assert!(out.starts_with(b"HTTP/1.1 401 Unauthorized"));
        assert_eq!(
            state.coreperf_visibility_requested.load(Ordering::Acquire),
            0
        );
    }

    let (mut client, mut server) = duplex(8192);
    client.write_all(b"GET /coreperf-visibility-proof HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret\r\n\r\n").await.unwrap();
    tokio::join!(
        biased;
        serve_connection(&mut server, &state),
        async {
            assert!(state.take_coreperf_visibility_request());
            state.record_coreperf_visibility_proof(CorePerfVisibilityProof {
                source_tick: 77,
                viewers: 2,
                tracked_entity_viewer_pairs: 8,
                unique_tracked_entities: Some(5),
                pair_limit: 1_000_000,
            });
        }
    );
    drop(server);
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("HTTP/1.1 200 OK"));
    let proof: serde_json::Value =
        serde_json::from_str(text.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(proof["source_tick"], 77);
    assert_eq!(proof["viewers"], 2);
    assert_eq!(proof["unique_tracked_entities"], 5);
    assert!(!state.take_coreperf_visibility_request());

    let (mut client, mut server) = duplex(8192);
    client.write_all(b"GET /coreperf-visibility-proof HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret\r\n\r\n").await.unwrap();
    serve_connection(&mut server, &state).await;
    drop(server);
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    assert!(out.starts_with(b"HTTP/1.1 409 Conflict"));
    assert_eq!(
        state
            .inner
            .lock()
            .coreperf_visibility_proof
            .as_ref()
            .unwrap()
            .source_tick,
        77
    );
}

#[test]
fn input_drops_are_polled_on_request_and_exported() {
    let state = test_state();
    let drops = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let exported = Arc::clone(&drops);
    state.set_input_source(Arc::new(move || {
        exported.load(std::sync::atomic::Ordering::Relaxed)
    }));
    assert_eq!(
        state.snapshot().network.input_drops,
        0,
        "a fresh source reads zero"
    );
    drops.store(7, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        state.snapshot().network.input_drops,
        7,
        "the count is read at request time, so a full input queue is \
             visible without waiting for a pump sample"
    );
    let text = prometheus(&state);
    assert!(
        text.contains("krasnokamen_network_input_drops_total 7\n"),
        "Prometheus exports the cumulative drop counter: {text}"
    );
}

/// WAL-FSYNC-LATENCY-AND-BYTES-UNMEASURED and PERF-RECEIPT-HAS-NO-ERROR-COUNTER-
/// ROLLUP: the operator surfaces carry the WAL write cost and every abnormal close
/// cause, so a rung receipt can quote fsync service time and a socket drop.
#[test]
fn stats_carry_wal_write_cost_and_close_causes() {
    let state = test_state();
    let registry = Arc::new(ConnectionRegistry::new());
    state.set_queue_source(Arc::clone(&registry));
    registry.close(mc_core::boundary::ConnectionId(3), "reliable-overflow");
    registry.close(mc_core::boundary::ConnectionId(3), "reliable-overflow");
    registry.close(mc_core::boundary::ConnectionId(4), "world-write-failed");
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot.network.close_causes.get("reliable-overflow"),
        Some(&2),
        "the rollup is read from the registry when the payload is built"
    );
    assert_eq!(
        snapshot.network.close_causes.get("world-write-failed"),
        Some(&1)
    );
    let value = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        value["network"]["close_causes"]["world-write-failed"], 1,
        "the rollup is in the JSON receipt: {value}"
    );
    let write = mc_core::journal::journal_write_stats();
    assert_eq!(
        snapshot.journal.append_bytes_total,
        write.append_bytes_total
    );
    assert_eq!(snapshot.journal.batch_bytes_last, write.batch_bytes_last);
    assert_eq!(snapshot.journal.file_fsyncs_total, write.file_fsyncs_total);
    assert_eq!(snapshot.journal.dir_fsyncs_total, write.dir_fsyncs_total);
    assert_eq!(snapshot.journal.fsync_nanos_total, write.fsync_nanos_total);
    assert_eq!(snapshot.journal.fsync_nanos_max, write.fsync_nanos_max);
    for key in [
        "append_bytes_total",
        "batch_bytes_last",
        "file_fsyncs_total",
        "dir_fsyncs_total",
        "fsync_nanos_total",
        "fsync_nanos_max",
    ] {
        assert!(
            value["journal"][key].is_u64(),
            "missing journal field {key}"
        );
    }
    let metrics = prometheus(&state);
    for name in [
        "krasnokamen_journal_append_bytes_total",
        "krasnokamen_journal_fsyncs_total",
        "krasnokamen_journal_fsync_nanoseconds_max",
        "krasnokamen_network_close_causes_total{cause=\"world-write-failed\"}",
    ] {
        assert!(metrics.contains(name), "missing {name}: {metrics}");
    }
}

/// The rollup counts session ends that are errors, once, and nothing else: a
/// delivery-policy close counts, the lifecycle's ordinary end does not.
#[test]
fn an_abnormal_close_is_counted_by_cause() {
    let registry = crate::server::ConnectionRegistry::new();
    let ordinary = registry.joining_id();
    registry.close(mc_core::boundary::ConnectionId(3), "reliable-overflow");
    registry.remove(ordinary);
    let causes = registry.close_causes();
    assert_eq!(causes.get("reliable-overflow"), Some(&1));
    assert_eq!(
        causes.get("removed"),
        None,
        "a normal end is not an error: {causes:?}"
    );
}
