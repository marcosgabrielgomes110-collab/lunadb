use crate::auth;
use crate::config::{Config, Paths};
use crate::crypto;
use crate::models::{
    ApiKey, Grant, Level, ROLE_ADMIN, ROLE_USER, Role, RolesFile, User, UsersFile,
};
use crate::storage;
use chrono::Utc;
use std::path::Path;
use uuid::Uuid;

pub const PASSWORD_HASH_PREFIX: &str = "$argon2";

pub async fn init(paths: &Paths, config: &mut Config, config_path: &Path) -> anyhow::Result<()> {
    paths.ensure().await?;
    ensure_master_key(paths, config).await?;
    ensure_roles(paths).await?;
    if ensure_hashed_root_password(config, config_path)? {
        tracing::info!(
            path = %config_path.display(),
            "root password in config was in plaintext and has been hashed (argon2id)"
        );
    }
    ensure_root(paths, config).await?;
    migrate_legacy_api_keys(paths).await?;
    Ok(())
}

pub fn ensure_hashed_root_password(
    config: &mut Config,
    config_path: &Path,
) -> anyhow::Result<bool> {
    let raw = config.root.password.clone();
    let trimmed = raw.trim();
    if trimmed.starts_with(PASSWORD_HASH_PREFIX) {
        argon2::password_hash::phc::PasswordHash::new(trimmed).map_err(|err| {
            anyhow::anyhow!("root.password in config is not a valid argon2 hash: {err}")
        })?;
        if trimmed != raw {
            config.root.password = trimmed.to_string();
            config.save(config_path)?;
        }
        return Ok(false);
    }
    if raw == "changeme" {
        tracing::warn!(
            "the root password is still the default 'changeme', change it in the config"
        );
    }
    config.root.password = auth::hash_password(&raw)?;
    config.save(config_path)?;
    if let Err(err) = crypto::set_private_permissions(config_path) {
        tracing::warn!("failed to adjust config permissions: {err}");
    }
    Ok(true)
}

async fn migrate_legacy_api_keys(paths: &Paths) -> anyhow::Result<()> {
    let mut users: UsersFile = storage::read_json(&paths.users_file()).await?;
    let mut migrated = 0;
    for user in users.users.iter_mut() {
        let Some(hash) = user.api_key_hash.take() else {
            continue;
        };
        user.api_keys.push(ApiKey {
            id: Uuid::new_v4().to_string(),
            name: "legacy".to_string(),
            key_hash: Some(hash),
            prefix: "legacy".to_string(),
            created_at: user.created_at,
            last_used_at: None,
            revoked_at: None,
        });
        migrated += 1;
    }
    if migrated > 0 {
        storage::write_json(&paths.users_file(), &users).await?;
        tracing::info!("{migrated} legacy api key(s) migrated to the new format");
    }
    Ok(())
}

async fn ensure_master_key(paths: &Paths, config: &Config) -> anyhow::Result<()> {
    if std::env::var("LUNADB_MASTER_KEY").is_ok() {
        tracing::info!("master key loaded from LUNADB_MASTER_KEY variable");
        return Ok(());
    }
    let path = crypto::master_key_path(paths, config);
    if !path.exists() {
        crypto::write_master_key(&path)?;
        tracing::info!("master key created at {}", path.display());
    } else if !crypto::has_private_permissions(&path) {
        tracing::warn!(
            "master key permissions at {} are broad, chmod 600 recommended",
            path.display()
        );
    }
    Ok(())
}

async fn ensure_roles(paths: &Paths) -> anyhow::Result<()> {
    let mut roles: RolesFile = storage::read_json(&paths.roles_file()).await?;
    if roles.roles.is_empty() {
        roles.roles = vec![
            Role {
                name: ROLE_ADMIN.to_string(),
                description: "full access to all dbs and user management".to_string(),
                builtin: true,
                permissions: vec![Grant {
                    db: "*".to_string(),
                    level: Level::Admin,
                }],
            },
            Role {
                name: ROLE_USER.to_string(),
                description: "default user, access only via grants".to_string(),
                builtin: true,
                permissions: Vec::new(),
            },
        ];
        storage::write_json(&paths.roles_file(), &roles).await?;
        tracing::info!("default roles created (admin, user)");
    }
    Ok(())
}

async fn ensure_root(paths: &Paths, config: &Config) -> anyhow::Result<()> {
    let mut users: UsersFile = storage::read_json(&paths.users_file()).await?;
    let configured_hash = config.root.password.clone();
    match users
        .users
        .iter_mut()
        .find(|user| user.username == config.root.username)
    {
        Some(user) => {
            if user.password_hash != configured_hash {
                user.password_hash = configured_hash;
                storage::write_json(&paths.users_file(), &users).await?;
                tracing::info!(
                    "root password '{}' synced with config",
                    config.root.username
                );
            }
        }
        None => {
            let root = User {
                id: Uuid::new_v4().to_string(),
                username: config.root.username.clone(),
                password_hash: configured_hash,
                api_key_hash: None,
                api_keys: Vec::new(),
                role: ROLE_ADMIN.to_string(),
                grants: Vec::new(),
                created_at: Utc::now(),
            };
            users.users.push(root);
            storage::write_json(&paths.users_file(), &users).await?;
            tracing::info!("root user '{}' created", config.root.username);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_plaintext_password_and_rewrites_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.root.password = "secret".to_string();

        assert!(ensure_hashed_root_password(&mut config, &path).unwrap());
        assert!(config.root.password.starts_with(PASSWORD_HASH_PREFIX));
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains(PASSWORD_HASH_PREFIX));
        assert!(!saved.contains("secret"));

        let before = config.root.password.clone();
        assert!(!ensure_hashed_root_password(&mut config, &path).unwrap());
        assert_eq!(config.root.password, before, "idempotent");
    }

    #[test]
    fn accepts_existing_hash_and_trims_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let hash = auth::hash_password("x").unwrap();
        let mut config = Config::default();
        config.root.password = format!("  {hash}  ");
        assert!(!ensure_hashed_root_password(&mut config, &path).unwrap());
        assert_eq!(config.root.password, hash);
    }

    #[test]
    fn rejects_invalid_hash_instead_of_hashing_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config.root.password = "$argon2id$this-is-not-valid".to_string();
        assert!(ensure_hashed_root_password(&mut config, &path).is_err());
    }
}
