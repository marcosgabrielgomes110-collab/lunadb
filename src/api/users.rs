use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{self, AuthUser};
use crate::engine;
use crate::error::AppError;
use crate::models::{Grant, ROLE_ADMIN, ROLE_USER, Role, User, UserPublic, UsersFile};
use crate::state::AppState;
use crate::storage;

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    pub grants: Vec<Grant>,
}

#[derive(Debug, Deserialize)]
pub struct SetGrantsRequest {
    pub grants: Vec<Grant>,
}

#[derive(Debug, Deserialize)]
pub struct SetPasswordRequest {
    pub password: String,
    #[serde(default)]
    pub current_password: Option<String>,
}

fn default_role() -> String {
    ROLE_USER.to_string()
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<UserPublic>>, AppError> {
    user.require_admin()?;
    let users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    tracing::info!(target: "lunadb::ops", op = %"list_users", user = %user.0.username, count = users.users.len());
    Ok(Json(users.users.iter().map(UserPublic::from).collect()))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<UserPublic>, AppError> {
    user.require_admin()?;
    engine::validate_name(&req.username)?;
    if req.password.len() < 6 {
        return Err(AppError::BadRequest(
            "password must be at least 6 characters".to_string(),
        ));
    }
    let roles = engine::roles(&state).await?;
    if !roles.roles.iter().any(|role| role.name == req.role) {
        return Err(AppError::BadRequest(format!(
            "role '{}' does not exist",
            req.role
        )));
    }
    for grant in &req.grants {
        engine::validate_grant(grant)?;
    }
    let _guard = state.lock_meta_write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    if users
        .users
        .iter()
        .any(|candidate| candidate.username == req.username)
    {
        return Err(AppError::Conflict(format!(
            "user '{}' already exists",
            req.username
        )));
    }
    let new_user = User {
        id: Uuid::new_v4().to_string(),
        username: req.username,
        password_hash: auth::hash_password(&req.password)?,
        api_key_hash: None,
        api_keys: Vec::new(),
        role: req.role,
        grants: req.grants,
        created_at: Utc::now(),
    };
    let public = UserPublic::from(&new_user);
    let target_username = new_user.username.clone();
    users.users.push(new_user);
    storage::write_json(&state.paths.users_file(), &users).await?;
    tracing::info!(target: "lunadb::ops", op = %"create_user", user = %user.0.username, target = %target_username, role = %public.role);
    Ok(Json(public))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    user.require_admin()?;
    if user.0.id == id {
        return Err(AppError::BadRequest("cannot remove yourself".to_string()));
    }
    let _guard = state.lock_meta_write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let index = users
        .users
        .iter()
        .position(|candidate| candidate.id == id)
        .ok_or_else(|| AppError::NotFound(format!("user '{id}'")))?;
    let is_last_admin = users.users[index].role == ROLE_ADMIN
        && users
            .users
            .iter()
            .filter(|candidate| candidate.role == ROLE_ADMIN)
            .count()
            <= 1;
    if is_last_admin {
        return Err(AppError::Conflict(
            "cannot remove the last administrator".to_string(),
        ));
    }
    let target_username = users.users[index].username.clone();
    users.users.remove(index);
    storage::write_json(&state.paths.users_file(), &users).await?;
    tracing::info!(target: "lunadb::ops", op = %"delete_user", user = %user.0.username, target = %target_username);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn set_grants(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<SetGrantsRequest>,
) -> Result<Json<UserPublic>, AppError> {
    user.require_admin()?;
    for grant in &req.grants {
        engine::validate_grant(grant)?;
    }
    let _guard = state.lock_meta_write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let target = users
        .users
        .iter_mut()
        .find(|candidate| candidate.id == id)
        .ok_or_else(|| AppError::NotFound(format!("user '{id}'")))?;
    target.grants = req.grants;
    let public = UserPublic::from(&*target);
    storage::write_json(&state.paths.users_file(), &users).await?;
    tracing::info!(target: "lunadb::ops", op = %"set_grants", user = %user.0.username, target = %public.username, grants = public.grants.len());
    Ok(Json(public))
}

pub async fn set_password(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<SetPasswordRequest>,
) -> Result<Json<UserPublic>, AppError> {
    if req.password.chars().count() < 6 {
        return Err(AppError::BadRequest(
            "password must be at least 6 characters".to_string(),
        ));
    }
    let is_self = user.0.id == id;
    if !is_self {
        user.require_admin()?;
    }
    let _guard = state.lock_meta_write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    if is_self && user.0.role != ROLE_ADMIN {
        let current = req.current_password.as_deref().ok_or_else(|| {
            AppError::BadRequest("provide current_password to change your own password".to_string())
        })?;
        let target = users
            .users
            .iter()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| AppError::NotFound(format!("user '{id}'")))?;
        if !auth::verify_password(current, &target.password_hash) {
            return Err(AppError::Unauthorized);
        }
    }
    let (public, username, new_hash) = {
        let target = users
            .users
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| AppError::NotFound(format!("user '{id}'")))?;
        let hash = auth::hash_password(&req.password)?;
        target.password_hash = hash.clone();
        let now = Utc::now();
        for key in target.api_keys.iter_mut() {
            auth::revoke_key(key, now);
        }
        (UserPublic::from(&*target), target.username.clone(), hash)
    };
    storage::write_json(&state.paths.users_file(), &users).await?;
    // keeps the config in sync when the root password changes (bootstrap re-syncs)
    if username == state.config.root.username {
        match crate::config::Config::load(&state.config_path) {
            Ok(mut config) => {
                config.root.password = new_hash;
                if let Err(err) = config.save(&state.config_path) {
                    tracing::warn!("failed to sync root password in config: {err}");
                }
            }
            Err(err) => {
                tracing::warn!("failed to load config to sync root: {err}");
            }
        }
    }
    tracing::info!(target: "lunadb::ops", op = %"set_password", user = %user.0.username, target = %public.username);
    Ok(Json(public))
}

pub async fn list_roles(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<Role>>, AppError> {
    user.require_admin()?;
    let roles = engine::roles(&state).await?;
    tracing::info!(target: "lunadb::ops", op = %"list_roles", user = %user.0.username, count = roles.roles.len());
    Ok(Json(roles.roles))
}
