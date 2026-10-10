use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

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
    let _guard = state.lock.write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let Some(user) = users
        .users
        .iter_mut()
        .find(|user| user.username == req.username)
    else {
        tracing::warn!(target: "lunadb::ops", op = %"login", user = %req.username, "credenciais invalidas");
        return Err(AppError::Unauthorized);
    };
    if !auth::verify_password(&req.password, &user.password_hash) {
        tracing::warn!(target: "lunadb::ops", op = %"login", user = %req.username, "credenciais invalidas");
        return Err(AppError::Unauthorized);
    }
    let api_key = auth::generate_api_key();
    user.api_key_hash = Some(auth::hash_api_key(&api_key));
    let public = UserPublic::from(&*user);
    storage::write_json(&state.paths.users_file(), &users).await?;
    tracing::info!(target: "lunadb::ops", op = %"login", user = %req.username, "login ok");
    Ok(Json(json!({ "user": public, "api_key": api_key })))
}

pub async fn rotate_key(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let _guard = state.lock.write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let target = users
        .users
        .iter_mut()
        .find(|candidate| candidate.id == user.0.id)
        .ok_or(AppError::Unauthorized)?;
    let api_key = auth::generate_api_key();
    target.api_key_hash = Some(auth::hash_api_key(&api_key));
    storage::write_json(&state.paths.users_file(), &users).await?;
    tracing::info!(target: "lunadb::ops", op = %"rotate_key", user = %user.0.username);
    Ok(Json(json!({ "api_key": api_key })))
}

pub async fn me(user: AuthUser) -> Json<UserPublic> {
    Json(UserPublic::from(&user.0))
}
