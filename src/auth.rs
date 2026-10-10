use crate::error::AppError;
use crate::models::{UsersFile, ROLE_ADMIN};
use crate::state::AppState;
use crate::storage;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use rand::RngExt;
use sha2::{Digest, Sha256};

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    Ok(Argon2::default()
        .hash_password(password.as_bytes())?
        .to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    Argon2::default()
        .verify_password(password.as_bytes(), hash)
        .is_ok()
}

pub fn generate_api_key() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    format!("luna_{}", hex::encode(bytes))
}

pub fn hash_api_key(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

pub struct AuthUser(pub crate::models::User);

impl AuthUser {
    pub fn require_admin(&self) -> Result<(), AppError> {
        if self.0.role == ROLE_ADMIN {
            Ok(())
        } else {
            Err(AppError::Forbidden(
                "acao exclusiva de administradores".to_string(),
            ))
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(AppError::Unauthorized)?;
        let key = header
            .strip_prefix("Bearer ")
            .ok_or(AppError::Unauthorized)?
            .trim();
        let wanted = hash_api_key(key);
        let users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
        users
            .users
            .into_iter()
            .find(|user| user.api_key_hash.as_deref() == Some(wanted.as_str()))
            .map(AuthUser)
            .ok_or(AppError::Unauthorized)
    }
}
