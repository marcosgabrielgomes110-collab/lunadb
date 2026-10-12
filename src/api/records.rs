use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::auth::AuthUser;
use crate::engine::{self, ListParams};
use crate::error::AppError;
use crate::query::{AggregateRequest, QueryRequest};
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
    Query(params): Query<ListParams>,
) -> Result<Json<Value>, AppError> {
    let outcome = engine::list_records(&state, &user.0, &db, &table, params).await?;
    Ok(Json(json!({
        "db": db,
        "table": table,
        "total": outcome.total,
        "count": outcome.records.len(),
        "records": outcome.records,
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

pub async fn aggregate(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
    Json(request): Json<AggregateRequest>,
) -> Result<Json<Value>, AppError> {
    let outcome = engine::aggregate_records(&state, &user.0, &db, &table, request).await?;
    Ok(Json(json!({
        "db": db,
        "table": table,
        "total": outcome.total,
        "count": outcome.groups.len(),
        "groups": outcome.groups,
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

pub async fn patch(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table, id)): Path<(String, String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        engine::patch_record(&state, &user.0, &db, &table, &id, body).await?,
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
