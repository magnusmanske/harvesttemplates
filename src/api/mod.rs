//! HTTP layer: JSON API under `/api`, static frontend everywhere else.

mod auth;
mod error;
mod meta;
mod runs;
mod shares;

pub use error::{ApiError, ApiResult};

use crate::app_state::AppState;
use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
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
use tower_sessions::cookie::SameSite;
use tower_sessions::{Expiry, SessionManagerLayer};

pub type SharedState = Arc<AppState>;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

pub fn router(state: SharedState) -> Router {
    let sessions = SessionManagerLayer::new(state.sessions.clone())
        .with_name("harvesttemplates")
        .with_secure(state.config.server.cookie_secure)
        .with_same_site(SameSite::Lax)
        .with_http_only(true)
        .with_expiry(Expiry::OnInactivity(time::Duration::days(state.config.server.session_lifetime_days)));
    let api = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/auth", auth::routes())
        .nest("/runs", runs::routes())
        .nest("/shares", shares::routes())
        .merge(meta::routes())
        .layer(middleware::from_fn(same_origin_writes));
    let callback = auth::callback_path(&state.config.oauth.callback_url);
    let static_files = ServeDir::new(&state.config.server.html_dir);
    let middleware = ServiceBuilder::new()
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new())
        .layer(TimeoutLayer::with_status_code(StatusCode::GATEWAY_TIMEOUT, REQUEST_TIMEOUT))
        .layer(CompressionLayer::new())
        .layer(header_layer(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(header_layer(header::X_FRAME_OPTIONS, "DENY"))
        .layer(header_layer(header::REFERRER_POLICY, "strict-origin-when-cross-origin"));
    Router::new()
        .nest("/api", api)
        .route(&callback, get(auth::callback))
        .route("/share.php", get(|| async { Redirect::permanent("/#/shares") }))
        .fallback_service(static_files)
        .layer(sessions)
        .layer(middleware)
        .with_state(state)
}

/// Reject cross-site writes. Together with `SameSite=Lax` cookies this is the CSRF defence.
async fn same_origin_writes(request: Request, next: Next) -> Response {
    let safe = matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    let headers = request.headers();
    let origin_host = headers
        .get(header::ORIGIN)
        .and_then(|o| o.to_str().ok())
        .map(|o| o.split_once("://").map_or(o, |(_, host)| host));
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    match origin_host {
        Some(origin) if !safe && Some(origin) != host => {
            (StatusCode::FORBIDDEN, "cross-site request refused").into_response()
        }
        _ => next.run(request).await,
    }
}

fn header_layer(name: header::HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
}

pub async fn serve(state: SharedState, port: u16) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, router(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::post;
    use tower::ServiceExt;

    async fn status(method: Method, origin: Option<&str>) -> StatusCode {
        let app = Router::new()
            .route("/x", post(|| async { "ok" }).get(|| async { "ok" }))
            .layer(middleware::from_fn(same_origin_writes));
        let mut req = Request::builder().method(method).uri("/x").header(header::HOST, "ht.toolforge.org");
        if let Some(o) = origin {
            req = req.header(header::ORIGIN, o);
        }
        app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap().status()
    }

    async fn call(method: Method, uri: &str, body: &str) -> (StatusCode, serde_json::Value) {
        let sessions = tempfile::tempdir().unwrap();
        let store = crate::storage::Store::from_url("mysql://nobody@127.0.0.1:1/none", 1).unwrap();
        let app = router(crate::test_support::test_app(store, "http://127.0.0.1:1", sessions.path()));
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn endpoints_without_login() {
        let (status, json) =
            call(Method::GET, "/api/spec/from-query?p=345&template=IMDb%20title&parameters=1", "").await;
        assert_eq!(
            (status, json["property"].as_str(), json["parameters"][0].as_str()),
            (StatusCode::OK, Some("P345"), Some("1"))
        );
        let (status, json) = call(Method::POST, "/api/spec/to-query", r#"{"property":"P345","template":"X"}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["query"].as_str().unwrap().contains("p=P345&template=X"));
        assert_eq!(call(Method::GET, "/api/auth/me", "").await.1, serde_json::json!({"user": null}));
        let (status, json) = call(Method::POST, "/api/runs", r#"{"spec":{}}"#).await;
        assert_eq!((status, json["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("not logged in")));
        assert_eq!(call(Method::POST, "/api/runs/1/start", "").await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(call(Method::GET, "/api/property/X1", "").await.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn cross_site_writes_are_refused() {
        assert_eq!(status(Method::POST, Some("https://evil.example")).await, StatusCode::FORBIDDEN);
        assert_eq!(status(Method::POST, Some("https://ht.toolforge.org")).await, StatusCode::OK);
        assert_eq!(status(Method::POST, None).await, StatusCode::OK);
        assert_eq!(status(Method::GET, Some("https://evil.example")).await, StatusCode::OK);
    }
}
