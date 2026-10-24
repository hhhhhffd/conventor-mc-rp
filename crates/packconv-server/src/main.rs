use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use packconv_core::{convert, repair, supported_versions, ConvertOptions, Error, Limits, RepairOptions};
use serde::Deserialize;
use tokio::sync::Semaphore;

struct AppState {
    jobs: Arc<Semaphore>,
    /// Requests currently waiting for or holding a job slot.
    queued: AtomicUsize,
    max_queue: usize,
    max_upload: usize,
    timeout: Duration,
    limits: Limits,
}

#[derive(Deserialize)]
struct ConvertQuery {
    to: String,
    from: Option<String>,
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    match std::env::var(name) {
        Ok(v) => v.parse().unwrap_or_else(|_| panic!("invalid value for {name}: {v}")),
        Err(_) => default,
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let addr: SocketAddr = env_or("PACKCONV_ADDR", "0.0.0.0:8080".parse().unwrap());
    let default_jobs = std::thread::available_parallelism().map_or(2, |n| n.get());
    let jobs = env_or("PACKCONV_MAX_JOBS", default_jobs).max(1);
    let state = Arc::new(AppState {
        jobs: Arc::new(Semaphore::new(jobs)),
        queued: AtomicUsize::new(0),
        max_queue: env_or("PACKCONV_MAX_QUEUE", jobs * 8),
        max_upload: env_or("PACKCONV_MAX_UPLOAD", 128 * 1024 * 1024),
        timeout: Duration::from_secs(env_or("PACKCONV_TIMEOUT_SECS", 120)),
        limits: Limits {
            max_entries: env_or("PACKCONV_MAX_ENTRIES", Limits::default().max_entries),
            max_total_size: env_or("PACKCONV_MAX_UNPACKED", Limits::default().max_total_size),
        },
    });

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/versions", get(|| async { Json(supported_versions()) }))
        .route("/convert", post(convert_handler))
        .route("/repair", post(repair_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await.expect("failed to bind");
    tracing::info!("listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        sig.recv().await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

/// Decrements the queue counter however the request ends (including client disconnects).
struct QueueSlot<'a>(&'a AtomicUsize);

impl Drop for QueueSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Result of a job: the ZIP to send back and the report for the `x-packconv-report` header.
type JobOutput = packconv_core::Result<(Vec<u8>, serde_json::Value)>;

/// `POST /convert?to=<version>[&from=<version>]` with the raw ZIP/RAR archive as the body.
/// Responds with the converted ZIP; the conversion report is in the `x-packconv-report` header.
async fn convert_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ConvertQuery>,
    body: Body,
) -> Response {
    let limits = state.limits;
    run_job(&state, body, "converted.zip", move |input, cancel| {
        let options = ConvertOptions { from: query.from, to: query.to, limits, cancel: Some(cancel) };
        convert(input, &options).map(|out| (out.zip, out.report.header_summary()))
    })
    .await
}

/// `POST /repair` with the raw ZIP/RAR archive as the body. Re-encodes badly saved PNG and
/// JSON files without changing the pack version; the report is in `x-packconv-report`.
async fn repair_handler(State(state): State<Arc<AppState>>, body: Body) -> Response {
    let limits = state.limits;
    run_job(&state, body, "repaired.zip", move |input, cancel| {
        let options = RepairOptions { limits, cancel: Some(cancel) };
        repair(input, &options).map(|out| {
            let summary = serde_json::json!({
                "repaired": out.report.repaired.len(),
                "warnings": out.report.warnings.len(),
                "files": out.report.files,
            });
            (out.zip, summary)
        })
    })
    .await
}

/// Runs one CPU-bound job under the server's admission limits: queue cap, job slots,
/// upload size and timeout with cooperative cancellation.
async fn run_job<F>(state: &AppState, body: Body, filename: &'static str, job: F) -> Response
where
    F: FnOnce(&[u8], Arc<AtomicBool>) -> JobOutput + Send + 'static,
{
    if state.queued.fetch_add(1, Ordering::Relaxed) >= state.max_queue {
        state.queued.fetch_sub(1, Ordering::Relaxed);
        return error(StatusCode::TOO_MANY_REQUESTS, "server busy, retry later");
    }
    let _slot = QueueSlot(&state.queued);

    // The permit is taken before the body is read so concurrent uploads cannot pile up in
    // memory, and it moves into the blocking task so it is held until the work really ends.
    let Ok(permit) = state.jobs.clone().acquire_owned().await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "shutting down");
    };
    let input = match axum::body::to_bytes(body, state.max_upload).await {
        Ok(bytes) => bytes,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "upload too large or interrupted"),
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let task_cancel = cancel.clone();
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job(&input, task_cancel)
    });
    // Dropping the handler (client gone) must also stop the work, not just the waiting.
    let _cancel_on_drop = CancelOnDrop(cancel.clone());

    let result = match tokio::time::timeout(state.timeout, task).await {
        Ok(joined) => joined,
        Err(_) => {
            cancel.store(true, Ordering::Relaxed);
            return error(StatusCode::GATEWAY_TIMEOUT, "job took too long");
        }
    };

    match result {
        Ok(Ok((zip, report))) => {
            let mut response = zip.into_response();
            let headers = response.headers_mut();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
            let disposition = format!("attachment; filename=\"{filename}\"");
            headers.insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&disposition).expect("static filename is a valid header"),
            );
            match HeaderValue::from_str(&report.to_string()) {
                Ok(value) => {
                    headers.insert("x-packconv-report", value);
                }
                Err(err) => tracing::warn!("report not representable as header: {err}"),
            }
            response
        }
        Ok(Err(Error::Cancelled)) => error(StatusCode::GATEWAY_TIMEOUT, "job took too long"),
        Ok(Err(err)) => error(StatusCode::UNPROCESSABLE_ENTITY, err.to_string()),
        Err(err) => {
            tracing::error!("job task failed: {err}");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    }
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
