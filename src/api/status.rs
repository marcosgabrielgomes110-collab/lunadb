use axum::Json;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use chrono::Utc;
use serde_json::{Value, json};

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::models::{DbsFile, UsersFile};
use crate::state::AppState;
use crate::storage;

/// Metrics in Prometheus text format (aggregated counters, no sensitive data).
pub async fn metrics(state: State<AppState>) -> impl IntoResponse {
    let uptime = (Utc::now() - state.started_at).num_seconds().max(0);
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        state.metrics.render(env!("CARGO_PKG_VERSION"), uptime),
    )
}

pub async fn status(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    user.require_admin()?;
    let users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let dbs: DbsFile = storage::read_json(&state.paths.dbs_registry()).await?;
    let uptime = (Utc::now() - state.started_at).num_seconds().max(0);
    Ok(Json(json!({
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION"),
        "started_at": state.started_at,
        "uptime_seconds": uptime,
        "host": state.config.server.host,
        "port": state.config.server.port,
        "storage": state.paths.home,
        "config": state.config_path,
        "log_file": state.paths.log_file(),
        "users": users.users.len(),
        "dbs": dbs.dbs.len(),
    })))
}
