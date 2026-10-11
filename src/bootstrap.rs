use crate::auth;
use crate::config::{Config, Paths};
use crate::crypto;
use crate::models::{
    ApiKey, Grant, Level, Role, User, UsersFile, RolesFile, ROLE_ADMIN, ROLE_USER,
};
use crate::storage;
use chrono::Utc;
use uuid::Uuid;

pub async fn init(paths: &Paths, config: &Config) -> anyhow::Result<()> {
    paths.ensure().await?;
    ensure_master_key(paths, config).await?;
    ensure_roles(paths).await?;
    ensure_root(paths, config).await?;
    migrate_legacy_api_keys(paths).await?;
    if config.root.password == "changeme" {
        tracing::warn!("a senha do root ainda e o padrao 'changeme', altere no config");
    }
    Ok(())
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
        tracing::info!("{migrated} api key(s) legada(s) migrada(s) para o novo formato");
    }
    Ok(())
}

async fn ensure_master_key(paths: &Paths, config: &Config) -> anyhow::Result<()> {
    if std::env::var("LUNADB_MASTER_KEY").is_ok() {
        tracing::info!("master key carregada da variavel LUNADB_MASTER_KEY");
        return Ok(());
    }
    let path = crypto::master_key_path(paths, config);
    if !path.exists() {
        crypto::write_master_key(&path)?;
        tracing::info!("master key criada em {}", path.display());
    } else if !crypto::has_private_permissions(&path) {
        tracing::warn!(
            "permissoes da master key em {} sao amplas, recomendado chmod 600",
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
                description: "acesso total a todos os dbs e gestao de usuarios".to_string(),
                builtin: true,
                permissions: vec![Grant {
                    db: "*".to_string(),
                    level: Level::Admin,
                }],
            },
            Role {
                name: ROLE_USER.to_string(),
                description: "usuario padrao, acesso somente via grants".to_string(),
                builtin: true,
                permissions: Vec::new(),
            },
        ];
        storage::write_json(&paths.roles_file(), &roles).await?;
        tracing::info!("roles padrao criadas (admin, user)");
    }
    Ok(())
}

async fn ensure_root(paths: &Paths, config: &Config) -> anyhow::Result<()> {
    let mut users: UsersFile = storage::read_json(&paths.users_file()).await?;
    match users
        .users
        .iter_mut()
        .find(|user| user.username == config.root.username)
    {
        Some(user) => {
            if !auth::verify_password(&config.root.password, &user.password_hash) {
                user.password_hash = auth::hash_password(&config.root.password)?;
                storage::write_json(&paths.users_file(), &users).await?;
                tracing::info!(
                    "senha do root '{}' sincronizada com o config",
                    config.root.username
                );
            }
        }
        None => {
            let root = User {
                id: Uuid::new_v4().to_string(),
                username: config.root.username.clone(),
                password_hash: auth::hash_password(&config.root.password)?,
                api_key_hash: None,
                api_keys: Vec::new(),
                role: ROLE_ADMIN.to_string(),
                grants: Vec::new(),
                created_at: Utc::now(),
            };
            users.users.push(root);
            storage::write_json(&paths.users_file(), &users).await?;
            tracing::info!("usuario root '{}' criado", config.root.username);
        }
    }
    Ok(())
}
