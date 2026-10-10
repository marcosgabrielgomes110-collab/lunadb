use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{self, AuthUser};
use crate::engine;
use crate::error::AppError;
use crate::models::{Grant, Role, User, UserPublic, UsersFile, ROLE_ADMIN, ROLE_USER};
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
            "senha deve ter ao menos 6 caracteres".to_string(),
        ));
    }
    let roles = engine::roles(&state).await?;
    if !roles.roles.iter().any(|role| role.name == req.role) {
        return Err(AppError::BadRequest(format!(
            "role '{}' nao existe",
            req.role
        )));
    }
    for grant in &req.grants {
        engine::validate_grant(grant)?;
    }
    let _guard = state.lock.write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    if users
        .users
        .iter()
        .any(|candidate| candidate.username == req.username)
    {
        return Err(AppError::Conflict(format!(
            "usuario '{}' ja existe",
            req.username
        )));
    }
    let new_user = User {
        id: Uuid::new_v4().to_string(),
        username: req.username,
        password_hash: auth::hash_password(&req.password)?,
        api_key_hash: None,
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
        return Err(AppError::BadRequest(
            "nao e possivel remover a si mesmo".to_string(),
        ));
    }
    let _guard = state.lock.write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let index = users
        .users
        .iter()
        .position(|candidate| candidate.id == id)
        .ok_or_else(|| AppError::NotFound(format!("usuario '{id}'")))?;
    let is_last_admin = users.users[index].role == ROLE_ADMIN
        && users
            .users
            .iter()
            .filter(|candidate| candidate.role == ROLE_ADMIN)
            .count()
            <= 1;
    if is_last_admin {
        return Err(AppError::Conflict(
            "nao e possivel remover o ultimo administrador".to_string(),
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
    let _guard = state.lock.write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let target = users
        .users
        .iter_mut()
        .find(|candidate| candidate.id == id)
        .ok_or_else(|| AppError::NotFound(format!("usuario '{id}'")))?;
    target.grants = req.grants;
    let public = UserPublic::from(&*target);
    storage::write_json(&state.paths.users_file(), &users).await?;
    tracing::info!(target: "lunadb::ops", op = %"set_grants", user = %user.0.username, target = %public.username, grants = public.grants.len());
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
