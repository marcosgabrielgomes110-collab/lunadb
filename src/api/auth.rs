use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{self, AuthUser};
use crate::error::AppError;
use crate::models::{UserPublic, UsersFile};
use crate::state::AppState;
use crate::storage;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<Value>, AppError> {
    let throttle_key = req.username.to_lowercase();
    if let Some(seconds) = state.login.retry_after(&throttle_key) {
        tracing::warn!(target: "lunadb::ops", op = %"login", user = %req.username, retry_after = seconds, "login blocked due to attempts");
        return Err(AppError::TooManyRequests(seconds));
    }
    let _guard = state.lock_meta_write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let Some(user) = users
        .users
        .iter_mut()
        .find(|user| user.username == req.username)
    else {
        state.login.record_failure(&throttle_key);
        tracing::warn!(target: "lunadb::ops", op = %"login", user = %req.username, "invalid credentials");
        return Err(AppError::Unauthorized);
    };
    if !auth::verify_password(&req.password, &user.password_hash) {
        state.login.record_failure(&throttle_key);
        tracing::warn!(target: "lunadb::ops", op = %"login", user = %req.username, "invalid credentials");
        return Err(AppError::Unauthorized);
    }
    state.login.record_success(&throttle_key);
    user.api_key_hash = None;
    let api_key = auth::rotate_named_key(user, auth::LOGIN_KEY_NAME, chrono::Utc::now());
    let public = UserPublic::from(&*user);
    storage::write_json(&state.paths.users_file(), &users).await?;
    tracing::info!(target: "lunadb::ops", op = %"login", user = %req.username, "login ok");
    Ok(Json(json!({ "user": public, "api_key": api_key })))
}

pub async fn rotate_key(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let _guard = state.lock_meta_write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let target = users
        .users
        .iter_mut()
        .find(|candidate| candidate.id == user.0.id)
        .ok_or(AppError::Unauthorized)?;
    target.api_key_hash = None;
    let api_key = auth::rotate_named_key(target, auth::LOGIN_KEY_NAME, chrono::Utc::now());
    storage::write_json(&state.paths.users_file(), &users).await?;
    tracing::info!(target: "lunadb::ops", op = %"rotate_key", user = %user.0.username);
    Ok(Json(json!({ "api_key": api_key })))
}

pub async fn me(user: AuthUser) -> Json<UserPublic> {
    Json(UserPublic::from(&user.0))
}
