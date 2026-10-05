//! HTTP layer: JSON API under `/api`, static frontend everywhere else.

mod error;

pub use error::{ApiError, ApiResult};

use crate::app_state::AppState;
use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::routing::get;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

pub type SharedState = Arc<AppState>;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

pub fn router(state: SharedState) -> Router {
    let api = Router::new().route("/healthz", get(|| async { "ok" }));
    let static_files = ServeDir::new(&state.config.server.html_dir);
    let middleware = ServiceBuilder::new()
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(CompressionLayer::new())
        .layer(header_layer(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(header_layer(header::X_FRAME_OPTIONS, "DENY"))
        .layer(header_layer(
            header::REFERRER_POLICY,
            "strict-origin-when-cross-origin",
        ));
    Router::new()
        .nest("/api", api)
        .fallback_service(static_files)
        .layer(middleware)
        .with_state(state)
}

fn header_layer(
    name: header::HeaderName,
    value: &'static str,
) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
}

pub async fn serve(state: SharedState, port: u16) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, router(state)).await?;
    Ok(())
}
