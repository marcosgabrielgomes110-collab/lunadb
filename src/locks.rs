use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock as AsyncRwLock};

/// Per-resource asynchronous read/write locks.
///
/// Reads of a resource run concurrently; a write excludes both readers and
/// other writers, so the storage layer never interleaves a read with a partial
/// write of the same file.
///
/// A single `LockTable` backs every resource, so a per-database lock and a
/// per-table lock can be held together without deadlocking.
pub struct LockTable {
    inner: RwLock<HashMap<String, Arc<AsyncRwLock<()>>>>,
}

impl LockTable {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// The lock for `key`, created on first use. The entry is never evicted,
    /// which keeps the map small (one entry per database and per table) and
    /// avoids dropping a lock another task may still be waiting on.
    fn entry(&self, key: &str) -> Arc<AsyncRwLock<()>> {
        if let Some(entry) = self.inner.read().unwrap().get(key) {
            return entry.clone();
        }
        let mut map = self.inner.write().unwrap();
        map.entry(key.to_string())
            .or_insert_with(|| Arc::new(AsyncRwLock::new(())))
            .clone()
    }

    pub async fn read(&self, key: &str) -> ReadGuard {
        let guard = self.entry(key).read_owned().await;
        ReadGuard { _guard: guard }
    }

    pub async fn write(&self, key: &str) -> WriteGuard {
        let guard = self.entry(key).write_owned().await;
        WriteGuard { _guard: guard }
    }
}

impl Default for LockTable {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ReadGuard {
    _guard: OwnedRwLockReadGuard<()>,
}

pub struct WriteGuard {
    _guard: OwnedRwLockWriteGuard<()>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn readers_run_concurrently_and_writers_exclude() {
        let table = Arc::new(LockTable::new());
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let mut tasks = Vec::new();
        for _ in 0..4 {
            let table = table.clone();
            let active = active.clone();
            let peak = peak.clone();
            tasks.push(tokio::spawn(async move {
                let _guard = table.read("db").await;
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(40)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) > 1, "readers must overlap");
    }

    #[tokio::test]
    async fn writer_excludes_readers() {
        let table = Arc::new(LockTable::new());
        let holder = {
            let table = table.clone();
            tokio::spawn(async move {
                let _guard = table.write("db").await;
                tokio::time::sleep(Duration::from_millis(60)).await;
            })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let started = std::time::Instant::now();
        let _guard = table.read("db").await;
        assert!(
            started.elapsed() >= Duration::from_millis(30),
            "reader must wait for the in-flight writer"
        );
        holder.await.unwrap();
    }

    #[tokio::test]
    async fn distinct_keys_do_not_block() {
        let table = LockTable::new();
        let _db = table.write("db:a").await;
        let _table = table.write("db:a/table:t").await;
    }
}
