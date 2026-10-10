use crate::config::{Config, Paths};
use crate::crypto::Keyring;
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub paths: Paths,
    pub lock: Arc<RwLock<()>>,
    pub keyring: Arc<Keyring>,
    pub config: Config,
    pub config_path: PathBuf,
    pub started_at: DateTime<Utc>,
}

impl AppState {
    pub fn new(
        paths: Paths,
        keyring: Arc<Keyring>,
        config: Config,
        config_path: PathBuf,
    ) -> Self {
        Self {
            paths,
            lock: Arc::new(RwLock::new(())),
            keyring,
            config,
            config_path,
            started_at: Utc::now(),
        }
    }
}
