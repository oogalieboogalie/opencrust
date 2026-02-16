use chrono::{DateTime, Utc};
use opencrust_common::{Error, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::info;

use crate::migrations::MEMORY_SCHEMA_V1;

/// Persisted memory entry used for retrieval and context assembly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub session_id: String,
    pub channel_id: Option<String>,
    pub user_id: Option<String>,
    pub role: MemoryRole,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
    pub embedding_dimensions: Option<usize>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Insert shape for new memory records before persistence assigns ID/timestamps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMemoryEntry {
    pub session_id: String,
    pub channel_id: Option<String>,
    pub user_id: Option<String>,
    pub role: MemoryRole,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRetrievalQuery {
    pub session_id: String,
    pub limit: usize,
    pub include_roles: Option<Vec<MemoryRole>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticMemoryQuery {
    pub session_id: Option<String>,
    pub embedding: Vec<f32>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchResult {
    pub entry_id: String,
    pub score: f32,
    pub role: MemoryRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

/// Backing store for long-term and session-scoped memory data.
pub struct MemoryStore {
    conn: Connection,
}

impl MemoryStore {
    pub fn open(db_path: &Path) -> Result<Self> {
        info!("opening memory store at {}", db_path.display());
        let conn = Connection::open(db_path)
            .map_err(|e| Error::Database(format!("failed to open memory database: {e}")))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| Error::Database(format!("failed to set pragmas: {e}")))?;

        let store = Self { conn };
        store.run_migrations()?;
        Ok(store)
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| Error::Database(format!("failed to open in-memory database: {e}")))?;

        let store = Self { conn };
        store.run_migrations()?;
        Ok(store)
    }

    fn run_migrations(&self) -> Result<()> {
        self.conn
            .execute_batch(MEMORY_SCHEMA_V1.sql)
            .map_err(|e| Error::Database(format!("memory migration failed: {e}")))?;

        Ok(())
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryStore;

    #[test]
    fn in_memory_creates_memory_entries_table() {
        let store = MemoryStore::in_memory().expect("failed to create in-memory memory store");
        let exists: i64 = store
            .connection()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='memory_entries'",
                [],
                |row| row.get(0),
            )
            .expect("failed to query sqlite_master");

        assert_eq!(exists, 1);
    }

    #[test]
    fn schema_has_embedding_metadata_columns() {
        let store = MemoryStore::in_memory().expect("failed to create in-memory memory store");
        let mut stmt = store
            .connection()
            .prepare("PRAGMA table_info(memory_entries)")
            .expect("failed to prepare pragma statement");

        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("failed to read table info")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("failed to collect columns");

        assert!(columns.iter().any(|c| c == "embedding"));
        assert!(columns.iter().any(|c| c == "embedding_model"));
        assert!(columns.iter().any(|c| c == "embedding_dimensions"));
    }

    #[test]
    fn schema_creates_session_index() {
        let store = MemoryStore::in_memory().expect("failed to create in-memory memory store");
        let exists: i64 = store
            .connection()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_memory_session_created_at'",
                [],
                |row| row.get(0),
            )
            .expect("failed to query sqlite_master for index");

        assert_eq!(exists, 1);
    }
}
