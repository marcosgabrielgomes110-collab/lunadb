use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::AuthUser;
use crate::engine::{self, ListParams};
use crate::error::AppError;
use crate::query::QueryRequest;
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
    Query(params): Query<ListParams>,
) -> Result<Json<Value>, AppError> {
    let records = engine::list_records(&state, &user.0, &db, &table, params).await?;
    Ok(Json(json!({
        "db": db,
        "table": table,
        "count": records.len(),
        "records": records,
    })))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let record = engine::create_record(&state, &user.0, &db, &table, body).await?;
    Ok((StatusCode::CREATED, Json(record)))
}

pub async fn validate(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        engine::validate_record(&state, &user.0, &db, &table, body).await?,
    ))
}

pub async fn query(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
    Json(request): Json<QueryRequest>,
) -> Result<Json<Value>, AppError> {
    let outcome = engine::query_records(&state, &user.0, &db, &table, request).await?;
    Ok(Json(json!({
        "db": db,
        "table": table,
        "total": outcome.total,
        "count": outcome.records.len(),
        "records": outcome.records,
    })))
}

pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table, id)): Path<(String, String, String)>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        engine::get_record(&state, &user.0, &db, &table, &id).await?,
    ))
}

pub async fn replace(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table, id)): Path<(String, String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        engine::replace_record(&state, &user.0, &db, &table, &id, body).await?,
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table, id)): Path<(String, String, String)>,
) -> Result<StatusCode, AppError> {
    engine::delete_record(&state, &user.0, &db, &table, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
