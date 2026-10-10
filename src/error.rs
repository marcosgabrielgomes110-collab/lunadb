use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
    #[serde(skip)]
    pub unique: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordError {
    pub id: String,
    pub fields: Vec<FieldError>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("api key ausente ou invalida")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("{0} nao encontrado")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("validacao falhou")]
    Validation(Vec<FieldError>),
    #[error("valor duplicado (unique)")]
    UniqueViolation(Vec<FieldError>),
    #[error("schema rejeitado pelos registros existentes")]
    SchemaRejected(Vec<RecordError>),
    #[error("{0}")]
    Crypto(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Internal(err.into())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Internal(err.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Validation(fields) => (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "validacao falhou", "fields": fields })),
            )
                .into_response(),
            AppError::UniqueViolation(fields) => (
                StatusCode::CONFLICT,
                Json(json!({ "error": "valor duplicado (unique)", "fields": fields })),
            )
                .into_response(),
            AppError::SchemaRejected(records) => (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "schema rejeitado pelos registros existentes",
                    "records": records,
                })),
            )
                .into_response(),
            other => {
                let status = match &other {
                    AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
                    AppError::Unauthorized => StatusCode::UNAUTHORIZED,
                    AppError::Forbidden(_) => StatusCode::FORBIDDEN,
                    AppError::NotFound(_) => StatusCode::NOT_FOUND,
                    AppError::Conflict(_) => StatusCode::CONFLICT,
                    AppError::Crypto(_) => StatusCode::INTERNAL_SERVER_ERROR,
                    AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
                    AppError::Validation(_)
                    | AppError::UniqueViolation(_)
                    | AppError::SchemaRejected(_) => unreachable!(),
                };
                let message = other.to_string();
                if status == StatusCode::INTERNAL_SERVER_ERROR {
                    tracing::error!("erro interno: {message}");
                }
                (status, Json(json!({ "error": message }))).into_response()
            }
        }
    }
}
