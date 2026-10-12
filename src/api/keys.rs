use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::apikeys;
use crate::auth::AuthUser;
use crate::error::AppError;
use crate::models::ApiKeyPublic;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateKeyRequest {
    #[serde(default)]
    pub name: Option<String>,
}

pub async fn list_for_user(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<ApiKeyPublic>>, AppError> {
    Ok(Json(apikeys::list(&state, &user.0, &id).await?))
}

pub async fn create_for_user(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<CreateKeyRequest>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let (api_key, plaintext) = apikeys::create(&state, &user.0, &id, req.name).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "api_key": api_key, "key": plaintext })),
    ))
}

pub async fn revoke_for_user(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, key)): Path<(String, String)>,
) -> Result<Json<ApiKeyPublic>, AppError> {
    Ok(Json(apikeys::revoke(&state, &user.0, &id, &key).await?))
}

pub async fn list_self(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<ApiKeyPublic>>, AppError> {
    let id = user.0.id.clone();
    Ok(Json(apikeys::list(&state, &user.0, &id).await?))
}

pub async fn create_self(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateKeyRequest>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let id = user.0.id.clone();
    let (api_key, plaintext) = apikeys::create(&state, &user.0, &id, req.name).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "api_key": api_key, "key": plaintext })),
    ))
}

pub async fn revoke_self(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
) -> Result<Json<ApiKeyPublic>, AppError> {
    let id = user.0.id.clone();
    Ok(Json(apikeys::revoke(&state, &user.0, &id, &key).await?))
}
