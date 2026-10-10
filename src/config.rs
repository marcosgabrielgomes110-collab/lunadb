use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub root: RootConfig,
    pub encryption: EncryptionConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RootConfig {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EncryptionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_key_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
    pub console: bool,
    pub file: bool,
    pub rotation: String,
    pub max_files: usize,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            console: true,
            file: true,
            rotation: "daily".to_string(),
            max_files: 7,
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 7700,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            path: "~/.lunadb".to_string(),
        }
    }
}

impl Default for RootConfig {
    fn default() -> Self {
        Self {
            username: "root".to_string(),
            password: "changeme".to_string(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self)?;
        std::fs::write(path, format!("# LunaDB - arquivo de configuracao\n{body}"))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    pub meta: PathBuf,
    pub dbs: PathBuf,
    pub infra: PathBuf,
}

impl Paths {
    pub fn from_config(config: &Config) -> Self {
        let home = expand_home(&config.storage.path);
        Self {
            meta: home.join("meta"),
            dbs: home.join("dbs"),
            infra: home.join("infra"),
            home,
        }
    }

    pub async fn ensure(&self) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.meta).await?;
        tokio::fs::create_dir_all(&self.dbs).await?;
        tokio::fs::create_dir_all(self.log_dir()).await?;
        Ok(())
    }

    pub fn log_dir(&self) -> PathBuf {
        self.infra.join("logs")
    }

    pub fn log_file(&self) -> PathBuf {
        self.log_dir().join("lunadb.log")
    }

    pub fn status_file(&self) -> PathBuf {
        self.infra.join("status.json")
    }

    pub fn users_file(&self) -> PathBuf {
        self.meta.join("users.json")
    }

    pub fn roles_file(&self) -> PathBuf {
        self.meta.join("roles.json")
    }

    pub fn dbs_registry(&self) -> PathBuf {
        self.meta.join("dbs.json")
    }

    pub fn db_dir(&self, db: &str) -> PathBuf {
        self.dbs.join(db)
    }

    pub fn table_file(&self, db: &str, table: &str) -> PathBuf {
        self.db_dir(db).join(format!("{table}.json"))
    }
}

pub fn expand_home(path: &str) -> PathBuf {
    if path == "~" && let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home);
    } else if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}
