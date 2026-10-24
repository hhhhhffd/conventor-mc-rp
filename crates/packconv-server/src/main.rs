use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use packconv_core::{convert, supported_versions, ConvertOptions, Limits};
use serde::Deserialize;
use tokio::sync::Semaphore;

struct AppState {
    jobs: Semaphore,
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
    let max_upload: usize = env_or("PACKCONV_MAX_UPLOAD", 256 * 1024 * 1024);
    let default_jobs = std::thread::available_parallelism().map_or(2, |n| n.get());
    let state = Arc::new(AppState {
        jobs: Semaphore::new(env_or("PACKCONV_MAX_JOBS", default_jobs)),
        limits: Limits {
            max_entries: env_or("PACKCONV_MAX_ENTRIES", Limits::default().max_entries),
            max_total_size: env_or("PACKCONV_MAX_UNPACKED", Limits::default().max_total_size),
        },
    });

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/versions", get(|| async { Json(supported_versions()) }))
        .route("/convert", post(convert_handler))
        .layer(DefaultBodyLimit::max(max_upload))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await.expect("failed to bind");
    tracing::info!("listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");
}

/// `POST /convert?to=<version>[&from=<version>]` with the raw ZIP/RAR archive as the body.
/// Responds with the converted ZIP; the conversion report is in the `x-packconv-report` header.
async fn convert_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ConvertQuery>,
    body: Bytes,
) -> Response {
    let Ok(_permit) = state.jobs.acquire().await else {
        return (StatusCode::SERVICE_UNAVAILABLE, "shutting down").into_response();
    };
    let options = ConvertOptions { from: query.from, to: query.to, limits: state.limits };
    let result = tokio::task::spawn_blocking(move || convert(&body, &options)).await;

    match result {
        Ok(Ok(output)) => {
            let report = serde_json::to_string(&output.report.header_summary())
                .expect("report serializes");
            let mut response = output.zip.into_response();
            let headers = response.headers_mut();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
            headers.insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"converted.zip\""),
            );
            match HeaderValue::from_str(&report) {
                Ok(value) => {
                    headers.insert("x-packconv-report", value);
                }
                Err(err) => tracing::warn!("report not representable as header: {err}"),
            }
            response
        }
        Ok(Err(err)) => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({ "error": err.to_string() })))
                .into_response()
        }
        Err(err) => {
            tracing::error!("conversion task failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "internal error" })))
                .into_response()
        }
    }
}
