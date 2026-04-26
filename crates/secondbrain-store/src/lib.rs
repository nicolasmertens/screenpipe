//! secondbrain — local SQLite store.
//!
//! Owns the dedicated `store.db` at
//! `~/Library/Application Support/secondbrain/store.db` (macOS) or
//! `$XDG_DATA_HOME/secondbrain/store.db` (Linux).
//!
//! Independent from screenpipe's internal databases. See PLAN.md.

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub mod models;
pub mod repo;

/// Connection wrapper around the secondbrain SQLite store.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Open (or create) the store at the default user-data path.
    pub async fn open_default() -> Result<Self> {
        let path = default_db_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating store dir at {parent:?}"))?;
        }
        Self::open(&path).await
    }

    /// Open (or create) the store at an arbitrary path. Useful for tests.
    pub async fn open(path: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .with_context(|| format!("opening store at {path:?}"))?;

        sqlx::migrate!("./src/migrations")
            .run(&pool)
            .await
            .context("running secondbrain migrations")?;

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// Returns `~/Library/Application Support/secondbrain/store.db` on macOS,
/// `$XDG_DATA_HOME/secondbrain/store.db` (or equivalent) elsewhere.
pub fn default_db_path() -> Result<PathBuf> {
    let base = dirs::data_dir().context("could not resolve data dir")?;
    Ok(base.join("secondbrain").join("store.db"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn store_opens_and_runs_migrations() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let store = Store::open(&path).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM segments")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
