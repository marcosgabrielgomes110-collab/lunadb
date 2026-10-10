use axum::extract::State;
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::models::{DbsFile, UsersFile};
use crate::state::AppState;
use crate::storage;

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
