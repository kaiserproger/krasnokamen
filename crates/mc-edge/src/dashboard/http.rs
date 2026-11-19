//! The one-request-per-connection TCP server and its routes.

use super::*;

use super::prometheus::prometheus;
use super::routing::parse_head;
use super::routing::waited_load_query;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;

/// Serve the dashboard on `addr:port` until the process exits. Binding
/// refuses a non-loopback address unless `allow_remote` is set; bind
/// failure is logged and the task returns.
pub async fn serve(addr: String, port: u16, state: Arc<DashboardState>) {
    serve_inner(addr, port, state, false).await;
}

/// Test/embedding entry: like [`serve`] but returns whether the listener
/// bound, so tests can assert the `allow_remote` gate.
pub async fn serve_bound(addr: String, port: u16, state: Arc<DashboardState>) -> bool {
    serve_inner(addr, port, state, true).await
}

async fn serve_inner(addr: String, port: u16, state: Arc<DashboardState>, report: bool) -> bool {
    let target = format!("{addr}:{port}");
    let Ok(sock) = target.parse::<SocketAddr>() else {
        tracing::warn!(target = %target, "dashboard: invalid bind address");
        return false;
    };
    if !sock.ip().is_loopback() {
        tracing::warn!(target = %target, "dashboard: non-loopback bind refused without allow_remote");
        return false;
    }
    let Ok(listener) = TcpListener::bind(sock).await else {
        tracing::warn!(target = %target, "dashboard: bind failed");
        return false;
    };
    tracing::info!(target = %target, "dashboard bound and listening");
    if report {
        tokio::spawn(run(listener, state));
        return true;
    }
    run(listener, state).await;
    true
}

async fn run(listener: TcpListener, state: Arc<DashboardState>) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            serve_connection(&mut stream, &state).await;
        });
    }
}

/// Serve exactly one request on one connection, then close it.
pub(super) async fn serve_connection<S>(stream: &mut S, state: &Arc<DashboardState>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let Ok(head) = read_head(stream).await else {
        return;
    };
    let Some(request) = parse_head(&head) else {
        let body = b"bad request";
        let _ = write_response(stream, "400 Bad Request", "text/plain", body).await;
        return;
    };
    if request.method != "GET" {
        let body = b"method not allowed";
        let _ = write_response_allow(stream, "405 Method Not Allowed", body).await;
        return;
    }
    match request.path.as_str() {
        "/coreperf-visibility-proof"
            if state.coreperf_visibility_requested.load(Ordering::Acquire) != u8::MAX =>
        {
            if state.metrics_bearer_token.get().is_none()
                || !state.metrics_authorized(request.bearer_token.as_deref())
            {
                let _ =
                    write_response(stream, "401 Unauthorized", "text/plain", b"unauthorized").await;
                return;
            }
            if state
                .coreperf_visibility_requested
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                let _ = write_response(
                    stream,
                    "409 Conflict",
                    "text/plain",
                    b"capture already requested",
                )
                .await;
                return;
            }
            let Ok(proof) = tokio::time::timeout(
                LOAD_WAIT_TIMEOUT,
                state.wait_for_coreperf_visibility_proof(),
            )
            .await
            else {
                let _ = write_response(
                    stream,
                    "504 Gateway Timeout",
                    "text/plain",
                    b"capture deadline expired",
                )
                .await;
                return;
            };
            let Ok(body) = serde_json::to_vec(&proof) else {
                let _ = write_response(
                    stream,
                    "500 Internal Server Error",
                    "text/plain",
                    b"encode error",
                )
                .await;
                return;
            };
            let _ = write_response(stream, "200 OK", "application/json", &body).await;
        }
        "/stats" => {
            let snapshot = state.snapshot();
            let Ok(body) = serde_json::to_vec(&snapshot) else {
                let _ = write_response(
                    stream,
                    "500 Internal Server Error",
                    "text/plain",
                    b"encode error",
                )
                .await;
                return;
            };
            let _ = write_response(stream, "200 OK", "application/json", &body).await;
        }
        "/tps" => {
            let snapshot = state.snapshot();
            let body = serde_json::json!({
                "tps": snapshot.tps,
                "mspt": snapshot.tick.p50_us as f64 / 1_000.0,
                "mspt_p95": snapshot.tick.p95_us as f64 / 1_000.0,
                "mspt_p99": snapshot.tick.p99_us as f64 / 1_000.0,
                "tick": snapshot.simulation_tick,
            });
            let _ = write_response(
                stream,
                "200 OK",
                "application/json",
                body.to_string().as_bytes(),
            )
            .await;
        }
        "/metrics" => {
            if !state.metrics_authorized(request.bearer_token.as_deref()) {
                let _ =
                    write_response(stream, "401 Unauthorized", "text/plain", b"unauthorized").await;
                return;
            }
            let body = prometheus(state);
            let _ = write_response(
                stream,
                "200 OK",
                "text/plain; version=0.0.4; charset=utf-8",
                body.as_bytes(),
            )
            .await;
        }
        "/load-metrics" => {
            let Some((query, wait_timed_out)) =
                waited_load_query(stream, state, request.query.as_deref(), false).await
            else {
                return;
            };
            let Some(mut snapshot) = state.load_snapshot(query.after_tick) else {
                let _ = write_response(
                    stream,
                    "503 Service Unavailable",
                    "text/plain",
                    b"metrics unavailable",
                )
                .await;
                return;
            };
            snapshot.wait_timed_out = wait_timed_out;
            let Ok(body) = serde_json::to_vec(&snapshot) else {
                let _ = write_response(
                    stream,
                    "500 Internal Server Error",
                    "text/plain",
                    b"encode error",
                )
                .await;
                return;
            };
            let _ = write_response(stream, "200 OK", "application/json", &body).await;
        }
        "/telemetry.jsonl" => {
            let Some((query, _)) =
                waited_load_query(stream, state, request.query.as_deref(), true).await
            else {
                return;
            };
            let Some(body) = state.telemetry_jsonl(query.after_tick) else {
                let _ = write_response(
                    stream,
                    "503 Service Unavailable",
                    "text/plain",
                    b"metrics unavailable",
                )
                .await;
                return;
            };
            let _ = write_response(stream, "200 OK", "application/x-ndjson", &body).await;
        }
        "/profile" => {
            let snapshot = state.snapshot();
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let mut value = serde_json::to_value(&snapshot).unwrap_or_default();
            if let Some(obj) = value.as_object_mut() {
                obj.insert("profile_schema".to_owned(), serde_json::json!(1));
                obj.insert("captured_unix_ms".to_owned(), serde_json::json!(now_ms));
            }
            let body = serde_json::to_vec(&value).unwrap_or_default();
            let _ = write_response(stream, "200 OK", "application/json", &body).await;
        }
        _ => {
            let body = b"not found";
            let _ = write_response(stream, "404 Not Found", "text/plain", body).await;
        }
    }
}

/// Read bytes until the CRLFCRLF head terminator, bounded by
/// [`MAX_HEAD_BYTES`]. Peer EOF or an oversized head is an error.
async fn read_head<S>(stream: &mut S) -> Result<String, ()>
where
    S: AsyncRead + Unpin,
{
    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0_u8; 512];
    let deadline = tokio::time::sleep(READ_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > MAX_HEAD_BYTES {
            return Err(());
        }
        tokio::select! {
            biased;
            _ = &mut deadline => return Err(()),
            read = stream.read(&mut chunk) => {
                let n = read.map_err(|_| ())?;
                if n == 0 {
                    return Err(());
                }
                buf.extend_from_slice(&chunk[..n]);
            }
        }
    }
    String::from_utf8(buf).map_err(|_| ())
}

pub(super) async fn write_response<S>(
    stream: &mut S,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

async fn write_response_allow<S>(stream: &mut S, status: &str, body: &[u8]) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let head = format!(
        "HTTP/1.1 {status}\r\nallow: GET\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}
