use crate::error::AppError;
use crate::models::{ApiKey, ROLE_ADMIN, UsersFile};
use crate::state::AppState;
use crate::storage;
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use rand::RngExt;
use sha2::{Digest, Sha256};

pub const LOGIN_KEY_NAME: &str = "login";
pub const LAST_USED_THROTTLE_SECONDS: i64 = 60;

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

pub fn key_prefix(key: &str) -> String {
    let visible: String = key.chars().take(12).collect();
    format!("{visible}...")
}

pub fn new_api_key(name: &str, plaintext: &str, created_at: DateTime<Utc>) -> ApiKey {
    ApiKey {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        key_hash: Some(hash_api_key(plaintext)),
        prefix: key_prefix(plaintext),
        created_at,
        last_used_at: None,
        revoked_at: None,
    }
}

pub fn revoke_key(key: &mut ApiKey, now: DateTime<Utc>) {
    if key.is_active() {
        key.revoked_at = Some(now);
        key.key_hash = None;
    }
}

pub fn rotate_named_key(user: &mut crate::models::User, name: &str, now: DateTime<Utc>) -> String {
    for key in user.api_keys.iter_mut() {
        if key.name == name {
            revoke_key(key, now);
        }
    }
    let plaintext = generate_api_key();
    user.api_keys.push(new_api_key(name, &plaintext, now));
    plaintext
}

pub struct AuthUser(pub crate::models::User);

impl AuthUser {
    pub fn require_admin(&self) -> Result<(), AppError> {
        if self.0.role == ROLE_ADMIN {
            Ok(())
        } else {
            Err(AppError::Forbidden("administrator-only action".to_string()))
        }
    }

    pub fn require_self_or_admin(&self, target_id: &str) -> Result<(), AppError> {
        if self.0.id == target_id {
            Ok(())
        } else {
            self.require_admin()
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
        let matched = users
            .users
            .iter()
            .enumerate()
            .find_map(|(user_index, user)| {
                user.api_keys
                    .iter()
                    .position(|api_key| {
                        api_key.is_active() && api_key.key_hash.as_deref() == Some(wanted.as_str())
                    })
                    .map(|key_index| (user_index, key_index))
            });
        let Some((user_index, key_index)) = matched else {
            return Err(AppError::Unauthorized);
        };
        let user = users.users[user_index].clone();
        let api_key = user.api_keys[key_index].clone();
        let stale = api_key
            .last_used_at
            .is_none_or(|last| (Utc::now() - last).num_seconds() >= LAST_USED_THROTTLE_SECONDS);
        if stale {
            touch_last_used(state, &user.id, &api_key.id).await;
        }
        Ok(AuthUser(user))
    }
}

async fn touch_last_used(state: &AppState, user_id: &str, key_id: &str) {
    let _guard = state.lock_meta_write().await;
    let Ok(mut users) = storage::read_json::<UsersFile>(&state.paths.users_file()).await else {
        return;
    };
    let Some(user) = users.users.iter_mut().find(|user| user.id == user_id) else {
        return;
    };
    let Some(api_key) = user
        .api_keys
        .iter_mut()
        .find(|api_key| api_key.id == key_id)
    else {
        return;
    };
    api_key.last_used_at = Some(Utc::now());
    let _ = storage::write_json(&state.paths.users_file(), &users).await;
}
