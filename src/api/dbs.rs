use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::AuthUser;
use crate::engine;
use crate::error::AppError;
use crate::models::DbMeta;
use crate::schema::{SchemaPatch, TableSchema};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateDbRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTableRequest {
    pub name: String,
    #[serde(default)]
    pub schema: Option<TableSchema>,
}

#[derive(Debug, Deserialize)]
pub struct ForceParams {
    #[serde(default)]
    pub force: bool,
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<DbMeta>>, AppError> {
    Ok(Json(engine::list_dbs(&state, &user.0).await?))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateDbRequest>,
) -> Result<(StatusCode, Json<DbMeta>), AppError> {
    let meta = engine::create_db(&state, &user.0, &req.name).await?;
    Ok((StatusCode::CREATED, Json(meta)))
}

pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(db): Path<String>,
) -> Result<Json<DbMeta>, AppError> {
    Ok(Json(engine::get_db(&state, &user.0, &db).await?))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(db): Path<String>,
) -> Result<StatusCode, AppError> {
    engine::delete_db(&state, &user.0, &db).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_tables(
    State(state): State<AppState>,
    user: AuthUser,
    Path(db): Path<String>,
) -> Result<Json<Value>, AppError> {
    let tables = engine::list_tables(&state, &user.0, &db).await?;
    Ok(Json(json!({ "db": db, "tables": tables })))
}

pub async fn create_table(
    State(state): State<AppState>,
    user: AuthUser,
    Path(db): Path<String>,
    Json(req): Json<CreateTableRequest>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let table = engine::create_table(&state, &user.0, &db, &req.name, req.schema).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "db": db, "table": table })),
    ))
}

pub async fn get_schema(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
) -> Result<Json<TableSchema>, AppError> {
    Ok(Json(
        engine::get_schema(&state, &user.0, &db, &table).await?,
    ))
}

pub async fn set_schema(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
    Query(params): Query<ForceParams>,
    Json(schema): Json<TableSchema>,
) -> Result<Json<TableSchema>, AppError> {
    Ok(Json(
        engine::set_schema(&state, &user.0, &db, &table, schema, params.force).await?,
    ))
}

pub async fn patch_schema(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
    Query(params): Query<ForceParams>,
    Json(patch): Json<SchemaPatch>,
) -> Result<Json<TableSchema>, AppError> {
    Ok(Json(
        engine::patch_schema(&state, &user.0, &db, &table, patch, params.force).await?,
    ))
}

pub async fn delete_schema(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    engine::delete_schema(&state, &user.0, &db, &table).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_table(
    State(state): State<AppState>,
    user: AuthUser,
    Path((db, table)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    engine::delete_table(&state, &user.0, &db, &table).await?;
    Ok(StatusCode::NO_CONTENT)
}
