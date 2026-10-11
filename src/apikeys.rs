use chrono::Utc;

use crate::auth::{generate_api_key, new_api_key, revoke_key};
use crate::error::AppError;
use crate::models::{ApiKeyPublic, User, UsersFile, ROLE_ADMIN};
use crate::state::AppState;
use crate::storage;

const MAX_ACTIVE_KEYS: usize = 50;

fn ensure_access(actor: &User, target_id: &str) -> Result<(), AppError> {
    if actor.id == target_id || actor.role == ROLE_ADMIN {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "so o proprio usuario ou um admin pode gerenciar api keys".to_string(),
        ))
    }
}

fn normalize_name(name: Option<String>) -> Result<String, AppError> {
    let name = name.unwrap_or_else(|| "default".to_string());
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("nome da api key vazio".to_string()));
    }
    if name.len() > 64 {
        return Err(AppError::BadRequest(
            "nome da api key excede 64 caracteres".to_string(),
        ));
    }
    Ok(name)
}

fn find_active<'a>(keys: &'a mut [crate::models::ApiKey], key_ref: &str) -> Result<&'a mut crate::models::ApiKey, AppError> {
    if keys.iter().any(|key| key.id == key_ref) {
        return keys
            .iter_mut()
            .find(|key| key.id == key_ref)
            .ok_or_else(|| AppError::NotFound(format!("api key '{key_ref}'")));
    }
    let mut matches = keys
        .iter_mut()
        .filter(|key| key.is_active() && !key_ref.is_empty() && key.prefix.starts_with(key_ref));
    let first = matches
        .next()
        .ok_or_else(|| AppError::NotFound(format!("api key '{key_ref}'")))?;
    if matches.next().is_some() {
        return Err(AppError::Conflict(format!(
            "prefixo '{key_ref}' e ambiguo, use o id completo"
        )));
    }
    Ok(first)
}

pub async fn create(
    state: &AppState,
    actor: &User,
    target_id: &str,
    name: Option<String>,
) -> Result<(ApiKeyPublic, String), AppError> {
    ensure_access(actor, target_id)?;
    let name = normalize_name(name)?;
    let _guard = state.lock.write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let target = users
        .users
        .iter_mut()
        .find(|user| user.id == target_id)
        .ok_or_else(|| AppError::NotFound(format!("usuario '{target_id}'")))?;
    if target.api_keys.iter().filter(|key| key.is_active()).count() >= MAX_ACTIVE_KEYS {
        return Err(AppError::Conflict(format!(
            "limite de {MAX_ACTIVE_KEYS} api keys ativas atingido"
        )));
    }
    let plaintext = generate_api_key();
    let key = new_api_key(&name, &plaintext, Utc::now());
    let public = ApiKeyPublic::from(&key);
    target.api_keys.push(key);
    let target_username = target.username.clone();
    storage::write_json(&state.paths.users_file(), &users).await?;
    tracing::info!(
        target: "lunadb::ops",
        op = "create_api_key",
        user = %actor.username,
        target = %target_username,
        name = %public.name,
        prefix = %public.prefix,
    );
    Ok((public, plaintext))
}

pub async fn list(
    state: &AppState,
    actor: &User,
    target_id: &str,
) -> Result<Vec<ApiKeyPublic>, AppError> {
    ensure_access(actor, target_id)?;
    let users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let target = users
        .users
        .iter()
        .find(|user| user.id == target_id)
        .ok_or_else(|| AppError::NotFound(format!("usuario '{target_id}'")))?;
    let keys = target.api_keys.iter().map(ApiKeyPublic::from).collect::<Vec<_>>();
    tracing::info!(
        target: "lunadb::ops",
        op = "list_api_keys",
        user = %actor.username,
        target = %target.username,
        count = keys.len(),
    );
    Ok(keys)
}

pub async fn revoke(
    state: &AppState,
    actor: &User,
    target_id: &str,
    key_ref: &str,
) -> Result<ApiKeyPublic, AppError> {
    ensure_access(actor, target_id)?;
    let _guard = state.lock.write().await;
    let mut users: UsersFile = storage::read_json(&state.paths.users_file()).await?;
    let target = users
        .users
        .iter_mut()
        .find(|user| user.id == target_id)
        .ok_or_else(|| AppError::NotFound(format!("usuario '{target_id}'")))?;
    let key = find_active(&mut target.api_keys, key_ref)?;
    if !key.is_active() {
        return Err(AppError::Conflict(format!(
            "api key '{}' ja esta revogada",
            key.id
        )));
    }
    revoke_key(key, Utc::now());
    let public = ApiKeyPublic::from(&*key);
    let target_username = target.username.clone();
    storage::write_json(&state.paths.users_file(), &users).await?;
    tracing::info!(
        target: "lunadb::ops",
        op = "revoke_api_key",
        user = %actor.username,
        target = %target_username,
        name = %public.name,
        id = %public.id,
    );
    Ok(public)
}
