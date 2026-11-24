//! The Prometheus text exposition of the dashboard state.

use super::*;

fn prometheus_gauge(out: &mut String, name: &str, value: impl std::fmt::Display) {
    out.push_str(name);
    out.push(' ');
    out.push_str(&value.to_string());
    out.push('\n');
}

/// Render a deliberately small, stable Prometheus surface from the dashboard
/// snapshot. The latency gauges are microseconds, matching `/stats`.
pub(super) fn prometheus(state: &DashboardState) -> String {
    let snapshot = state.snapshot();
    let mut out = String::new();
    out.push_str("# TYPE krasnokamen_tps gauge\n");
    prometheus_gauge(&mut out, "krasnokamen_tps", snapshot.tps);
    out.push_str("# TYPE krasnokamen_tick_duration_microseconds gauge\n");
    for (quantile, value) in [
        ("0.50", snapshot.tick.p50_us),
        ("0.95", snapshot.tick.p95_us),
        ("0.99", snapshot.tick.p99_us),
    ] {
        out.push_str("krasnokamen_tick_duration_microseconds{quantile=\"");
        out.push_str(quantile);
        out.push_str("\"} ");
        out.push_str(&value.to_string());
        out.push('\n');
    }
    prometheus_gauge(&mut out, "krasnokamen_players", snapshot.players.count);
    prometheus_gauge(
        &mut out,
        "krasnokamen_chunks_loaded",
        snapshot.chunks.loaded_total,
    );
    prometheus_gauge(&mut out, "krasnokamen_entities", snapshot.entities.total);
    prometheus_gauge(
        &mut out,
        "krasnokamen_network_bytes_written_total",
        snapshot.network.bytes_written,
    );
    prometheus_gauge(
        &mut out,
        "krasnokamen_network_reliable_drops_total",
        snapshot.network.reliable_drops,
    );
    prometheus_gauge(
        &mut out,
        "krasnokamen_network_input_drops_total",
        snapshot.network.input_drops,
    );
    prometheus_gauge(
        &mut out,
        "krasnokamen_pump_skipped_ticks_total",
        snapshot.pump_skipped_ticks,
    );
    prometheus_gauge(
        &mut out,
        "krasnokamen_pump_stale_skipped_ticks_total",
        snapshot.pump_stale_skipped_ticks,
    );
    out.push_str("# TYPE krasnokamen_journal_prune_failures_total counter\n");
    prometheus_gauge(
        &mut out,
        "krasnokamen_journal_prune_failures_total",
        snapshot.journal.prune_failures_total,
    );
    out.push_str("# TYPE krasnokamen_journal_prune_retained_bytes gauge\n");
    prometheus_gauge(
        &mut out,
        "krasnokamen_journal_prune_retained_bytes",
        snapshot.journal.prune_retained_bytes,
    );
    out.push_str("# TYPE krasnokamen_journal_append_bytes_total counter\n");
    prometheus_gauge(
        &mut out,
        "krasnokamen_journal_append_bytes_total",
        snapshot.journal.append_bytes_total,
    );
    out.push_str("# TYPE krasnokamen_journal_fsyncs_total counter\n");
    prometheus_gauge(
        &mut out,
        "krasnokamen_journal_fsyncs_total",
        snapshot.journal.file_fsyncs_total + snapshot.journal.dir_fsyncs_total,
    );
    out.push_str("# TYPE krasnokamen_journal_fsync_nanoseconds_max gauge\n");
    prometheus_gauge(
        &mut out,
        "krasnokamen_journal_fsync_nanoseconds_max",
        snapshot.journal.fsync_nanos_max,
    );
    out.push_str("# TYPE krasnokamen_network_close_causes_total counter\n");
    for (cause, count) in &snapshot.network.close_causes {
        out.push_str("krasnokamen_network_close_causes_total{cause=\"");
        out.push_str(cause);
        out.push_str("\"} ");
        out.push_str(&count.to_string());
        out.push('\n');
    }
    if let Some(bytes) = crate::host_telemetry::resident_memory_bytes() {
        prometheus_gauge(&mut out, "krasnokamen_process_resident_memory_bytes", bytes);
    }
    let load_metrics = state.load_snapshot(None);
    if let Some(sample) = load_metrics
        .as_ref()
        .and_then(|metrics| metrics.samples.last())
    {
        for (phase, value) in [
            ("input", sample.phase_us.input_us),
            ("chunks", sample.phase_us.chunks_us),
            ("entity", sample.phase_us.entity_us),
            ("merge", sample.phase_us.merge_us),
            ("block", sample.phase_us.block_us),
            ("commit", sample.phase_us.commit_us),
            ("snapshot", sample.phase_us.snapshot_us),
            ("views", sample.phase_us.views_us),
            ("publish", sample.phase_us.publish_us),
            ("plugins", sample.phase_us.plugins_us),
            ("coordination", sample.phase_us.coordination_us),
            ("journal_build", sample.phase_us.journal_build_us),
            ("journal_queue", sample.phase_us.journal_queue_us),
        ] {
            out.push_str("krasnokamen_tick_phase_microseconds{phase=\"");
            out.push_str(phase);
            out.push_str("\"} ");
            out.push_str(&value.to_string());
            out.push('\n');
        }
    }
    out
}
