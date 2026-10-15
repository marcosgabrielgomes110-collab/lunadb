use crate::config::{Config, Paths};
use crate::crypto::Keyring;
use crate::locks::{LockTable, ReadGuard, WriteGuard};
use crate::metrics::Metrics;
use crate::ratelimit::LoginThrottle;
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub paths: Paths,
    /// Per-resource read/write locks (one gate per database, one per table).
    pub locks: Arc<LockTable>,
    pub keyring: Arc<Keyring>,
    pub config: Config,
    pub config_path: PathBuf,
    pub started_at: DateTime<Utc>,
    pub metrics: Arc<Metrics>,
    pub login: Arc<LoginThrottle>,
}

impl AppState {
    pub fn new(paths: Paths, keyring: Arc<Keyring>, config: Config, config_path: PathBuf) -> Self {
        Self {
            paths,
            locks: Arc::new(LockTable::new()),
            keyring,
            config,
            config_path,
            started_at: Utc::now(),
            metrics: Arc::new(Metrics::default()),
            login: Arc::new(LoginThrottle::new()),
        }
    }

    pub(crate) fn db_key(db: &str) -> String {
        format!("db:{db}")
    }

    pub(crate) fn table_key(db: &str, table: &str) -> String {
        format!("db:{db}/table:{table}")
    }

    /// Shared access to a single table's file.
    pub async fn lock_table_read(&self, db: &str, table: &str) -> ReadGuard {
        self.locks.read(&Self::table_key(db, table)).await
    }

    /// Exclusive access to a single table's file.
    pub async fn lock_table_write(&self, db: &str, table: &str) -> WriteGuard {
        self.locks.write(&Self::table_key(db, table)).await
    }

    /// Exclusive access to a database and everything in it, for structural
    /// changes (create/delete of the database itself).
    pub async fn lock_db_write(&self, db: &str) -> WriteGuard {
        self.locks.write(&Self::db_key(db)).await
    }

    /// Metadata files (users, roles, databases) keep a single global gate.
    pub async fn lock_meta_read(&self) -> ReadGuard {
        self.locks.read("meta").await
    }

    pub async fn lock_meta_write(&self) -> WriteGuard {
        self.locks.write("meta").await
    }
}
