pub mod auth;
pub mod dbs;
pub mod keys;
pub mod records;
pub mod status;
pub mod users;

use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit};
use axum::http::{Request, Response, StatusCode};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Duration;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::Span;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let metrics = state.metrics.clone();
    let max_body_bytes = state.config.server.max_body_bytes;
    let timeout = Duration::from_secs(state.config.server.request_timeout_secs.max(1));
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(status::metrics))
        .route("/status", get(status::status))
        .route("/auth/login", post(auth::login))
        .route("/auth/rotate-key", post(auth::rotate_key))
        .route(
            "/auth/api-keys",
            get(keys::list_self).post(keys::create_self),
        )
        .route("/auth/api-keys/{key}", delete(keys::revoke_self))
        .route("/me", get(auth::me))
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", delete(users::delete))
        .route("/users/{id}/grants", put(users::set_grants))
        .route("/users/{id}/password", put(users::set_password))
        .route(
            "/users/{id}/api-keys",
            get(keys::list_for_user).post(keys::create_for_user),
        )
        .route("/users/{id}/api-keys/{key}", delete(keys::revoke_for_user))
        .route("/roles", get(users::list_roles))
        .route("/dbs", get(dbs::list).post(dbs::create))
        .route("/dbs/{db}", get(dbs::get).delete(dbs::delete))
        .route(
            "/dbs/{db}/tables",
            get(dbs::list_tables).post(dbs::create_table),
        )
        .route("/dbs/{db}/tables/{table}", delete(dbs::delete_table))
        .route(
            "/dbs/{db}/tables/{table}/schema",
            get(dbs::get_schema)
                .put(dbs::set_schema)
                .patch(dbs::patch_schema)
                .delete(dbs::delete_schema),
        )
        .route(
            "/dbs/{db}/tables/{table}/records",
            get(records::list).post(records::create),
        )
        .route(
            "/dbs/{db}/tables/{table}/records/validate",
            post(records::validate),
        )
        .route(
            "/dbs/{db}/tables/{table}/records/query",
            post(records::query),
        )
        .route(
            "/dbs/{db}/tables/{table}/records/aggregate",
            post(records::aggregate),
        )
        .route(
            "/dbs/{db}/tables/{table}/records/{id}",
            get(records::get)
                .put(records::replace)
                .patch(records::patch)
                .delete(records::delete),
        )
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            timeout,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<Body>| {
                    let ip = request
                        .extensions()
                        .get::<ConnectInfo<SocketAddr>>()
                        .map(|info| info.0.to_string())
                        .unwrap_or_default();
                    tracing::info_span!(
                        "http",
                        method = %request.method(),
                        uri = %request.uri(),
                        ip = %ip
                    )
                })
                .on_response(
                    move |response: &Response<Body>, latency: Duration, span: &Span| {
                        let _guard = span.enter();
                        let status = response.status().as_u16();
                        metrics.record(status);
                        let latency_ms = latency.as_millis() as u64;
                        if response.status().is_server_error() {
                            tracing::error!(status, latency_ms, "http_request");
                        } else if response.status().is_client_error() {
                            tracing::warn!(status, latency_ms, "http_request");
                        } else {
                            tracing::info!(status, latency_ms, "http_request");
                        }
                    },
                ),
        )
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "name": "lunadb",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
