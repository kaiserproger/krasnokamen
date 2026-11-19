//! Request-head parsing, the load-metrics query and its push-driven wait.

use super::*;

use super::http::write_response;
use tokio::io::AsyncWrite;

/// Parse a request head. The metrics endpoint reads only a standard Bearer
/// authorization header; every other route remains unauthenticated.
pub(super) fn parse_head(head: &str) -> Option<Request> {
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?;
    let (path, query) = target
        .split_once('?')
        .map_or((target, None), |(path, query)| {
            (path, Some(query.to_owned()))
        });
    let bearer_token = head.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("authorization")
            .then(|| value.trim().strip_prefix("Bearer ").map(str::to_owned))
            .flatten()
    });
    Some(Request {
        method,
        path: path.to_owned(),
        query,
        bearer_token,
    })
}

pub(super) fn parse_load_query(query: Option<&str>) -> Result<LoadMetricsQuery, ()> {
    let mut after_tick = None;
    let mut wait = None;
    let mut min_samples = None;
    for parameter in query.into_iter().flat_map(|query| query.split('&')) {
        if parameter.is_empty() {
            continue;
        }
        let (key, value) = parameter.split_once('=').ok_or(())?;
        match key {
            "wait" if wait.is_none() => {
                wait = Some(match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err(()),
                });
            }
            "after_tick" if after_tick.is_none() => {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(());
                }
                after_tick = Some(value.parse().map_err(|_| ())?);
            }
            "min_samples" if min_samples.is_none() => {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(());
                }
                let samples = value.parse::<usize>().map_err(|_| ())?;
                if !(1..=1200).contains(&samples) {
                    return Err(());
                }
                min_samples = Some(samples);
            }
            _ => return Err(()),
        }
    }
    Ok(LoadMetricsQuery {
        after_tick,
        wait: wait.unwrap_or(false),
        min_samples: min_samples.unwrap_or(1),
    })
}

/// Parse a `/load-metrics` query (answering 400 when invalid) and run its
/// push-driven wait (on fanned-out ticks only when `fanned_out`); returns
/// the query and whether the wait timed out.
pub(super) async fn waited_load_query<S>(
    stream: &mut S,
    state: &DashboardState,
    query: Option<&str>,
    fanned_out: bool,
) -> Option<(LoadMetricsQuery, bool)>
where
    S: AsyncWrite + Unpin,
{
    let Ok(query) = parse_load_query(query) else {
        let _ = write_response(
            stream,
            "400 Bad Request",
            "text/plain",
            b"invalid metrics query",
        )
        .await;
        return None;
    };
    let timed_out = query.wait
        && tokio::time::timeout(
            LOAD_WAIT_TIMEOUT,
            state.wait_for_load_samples(query.after_tick, query.min_samples, fanned_out),
        )
        .await
        .is_err();
    Some((query, timed_out))
}

/// The value an ascending `(tick, value)` series holds for `tick`, after
/// skipping its older entries.
pub(super) fn joined<'a, T: Copy + 'a>(
    series: &mut std::iter::Peekable<impl Iterator<Item = &'a (u64, T)>>,
    tick: u64,
) -> Option<T> {
    while series.next_if(|(at, _)| *at < tick).is_some() {}
    series
        .next_if(|(at, _)| *at == tick)
        .map(|&(_, value)| value)
}
