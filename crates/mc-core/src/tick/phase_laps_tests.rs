use super::*;

#[test]
fn disabled_collector_never_records_or_allocates() {
    let mut laps = Laps::new(false, Instant::now());
    laps.record("known", Duration::from_secs(1));
    laps.lap("elapsed");
    assert!(laps.last.is_none());
    assert!(laps.rows.is_empty());
    assert_eq!(laps.rows.capacity(), 0);
    assert!(laps.line(format_args!("phase ms: tick=7")).is_none());
}

#[test]
fn known_spans_preserve_the_continuous_lap_boundary() {
    let start = Instant::now();
    let mut laps = Laps::new(true, start);
    laps.record("nested", Duration::from_micros(1250));
    assert_eq!(laps.last, Some(start));
    assert_eq!(laps.rows, [("nested", Duration::from_micros(1250))]);
    laps.lap("whole");
    assert_eq!(laps.rows.len(), 2);
    assert_eq!(laps.rows[1].1, laps.last.unwrap().duration_since(start));
}

#[test]
fn summary_prefix_and_numeric_units_remain_parser_compatible() {
    let mut laps = Laps::new(true, Instant::now());
    laps.record("input", Duration::from_micros(1250));
    laps.record("allocator", Duration::from_micros(20));
    assert_eq!(
        laps.line(format_args!(
            "phase ms: tick=7 intents=3 dimension=Overworld"
        )),
        Some("phase ms: tick=7 intents=3 dimension=Overworld input=1.25 allocator=0.02".into()),
    );
    assert_eq!(
        laps.line(format_args!("tick outer ms: tick=7")),
        Some("tick outer ms: tick=7 input=1.25 allocator=0.02".into()),
    );
    assert_eq!(
        laps.line(format_args!("journal build ms: tick=7")),
        Some("journal build ms: tick=7 input=1.25 allocator=0.02".into()),
    );
}
