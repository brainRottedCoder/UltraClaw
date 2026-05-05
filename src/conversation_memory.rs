use crate::db::ConversationDb;
use crate::embedding_service::EmbeddingService;
use crate::memory_vector::{MemoryEntry, MemoryVectorStore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

const MAX_RECALL_RESULTS: usize = 10;
const DEFAULT_MEMORY_DB_NAME: &str = "ultraclaw_memories.db";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentMemoryEntry {
    pub id: String,
    pub content: String,
    pub category: String,
    pub importance: f32,
    pub created_at: i64,
    pub access_count: u64,
}

impl From<MemoryEntry> for PersistentMemoryEntry {
    fn from(e: MemoryEntry) -> Self {
        Self {
            id: e.id,
            content: e.content,
            category: e.category,
            importance: e.importance,
            created_at: e.created_at,
            access_count: e.access_count,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryRecallResult {
    pub entries: Vec<PersistentMemoryEntry>,
    pub query_used: String,
    pub total_available: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryStats {
    pub total_entries: usize,
    pub categories: HashMap<String, usize>,
    pub oldest_entry_secs: Option<i64>,
}

pub struct PersistentMemory {
    vector_store: Arc<MemoryVectorStore>,
    embedding_service: Arc<EmbeddingService>,
    stats: RwLock<MemoryStats>,
}

impl PersistentMemory {
    pub fn open(db_path: Option<&str>) -> Result<Arc<Self>, String> {
        let path = db_path.unwrap_or(DEFAULT_MEMORY_DB_NAME);
        ConversationDb::open(path)?;

        let conn = rusqlite::Connection::open(path)
            .map_err(|e| format!("Memory DB connection: {}", e))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS long_term_memory (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT 'general',
                importance REAL NOT NULL DEFAULT 1.0,
                created_at INTEGER NOT NULL,
                last_accessed INTEGER NOT NULL DEFAULT 0,
                access_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_mem_category ON long_term_memory(category);
            CREATE INDEX IF NOT EXISTS idx_mem_importance ON long_term_memory(importance DESC);
            CREATE INDEX IF NOT EXISTS idx_mem_accessed ON long_term_memory(last_accessed DESC);",
        )
        .map_err(|e| format!("Memory schema creation: {}", e))?;

        let store = Arc::new(Self {
            vector_store: MemoryVectorStore::new(),
            embedding_service: EmbeddingService::new(None, None),
            stats: RwLock::new(MemoryStats::default()),
        });

        let loaded_count: usize = {
            let conn2 = rusqlite::Connection::open(path)
                .map_err(|e| format!("Memory DB connection: {}", e))?;
            conn2.query_row("SELECT COUNT(*) FROM long_term_memory", [], |row| row.get::<_, i64>(0))
                .unwrap_or(0) as usize
        };

        {
            let mut s = store.stats.blocking_write();
            s.total_entries = loaded_count;
        }

        info!(loaded = loaded_count, "Memories loaded from DB");

        Ok(store)
    }

    pub async fn remember(
        &self,
        content: &str,
        category: &str,
        importance: f32,
    ) -> Result<String, String> {
        let memory_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let conn = rusqlite::Connection::open(DEFAULT_MEMORY_DB_NAME)
            .map_err(|e| format!("DB connect: {}", e))?;

        conn.execute(
            "INSERT INTO long_term_memory (id, content, category, importance, created_at, last_accessed, access_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![memory_id, content, category, importance, now, now, 0u64],
        )
        .map_err(|e| format!("Memory insert: {}", e))?;

        let _ = self
            .vector_store
            .add_with_importance(content, category, importance)
            .await;

        if let Ok(mut stats) = self.stats.try_write() {
            stats.total_entries += 1;
            *stats
                .categories
                .entry(category.to_string())
                .or_insert(0) += 1;
        }

        info!(
            id = %memory_id,
            category = %category,
            importance = importance,
            "Memory stored"
        );

        Ok(memory_id)
    }

    pub async fn recall(
        &self,
        query: &str,
        top_k: Option<usize>,
    ) -> Result<MemoryRecallResult, String> {
        let k = top_k.unwrap_or(MAX_RECALL_RESULTS);

        let entries: Vec<MemoryEntry> = self.vector_store.search(query, k).await;

        let now = chrono::Utc::now().timestamp();
        let conn = rusqlite::Connection::open(DEFAULT_MEMORY_DB_NAME)
            .map_err(|e| format!("DB connect: {}", e))?;

        for entry in &entries {
            let _ = conn.execute(
                "UPDATE long_term_memory SET last_accessed = ?1, access_count = access_count + 1 WHERE id = ?2",
                rusqlite::params![now, entry.id],
            );
        }

        let total = {
            conn.query_row(
                "SELECT COUNT(*) FROM long_term_memory",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
        };

        let persistent_entries: Vec<PersistentMemoryEntry> =
            entries.into_iter().map(PersistentMemoryEntry::from).collect();

        info!(
            query = %query,
            found = persistent_entries.len(),
            total = total,
            "Memory recalled"
        );

        Ok(MemoryRecallResult {
            entries: persistent_entries,
            query_used: query.to_string(),
            total_available: total,
        })
    }

    pub async fn recall_for_context(&self, limit: Option<usize>) -> String {
        let limit = limit.unwrap_or(MAX_RECALL_RESULTS);

        let conn = match rusqlite::Connection::open(DEFAULT_MEMORY_DB_NAME) {
            Ok(c) => c,
            Err(_) => return String::new(),
        };

        let mut stmt = match conn.prepare(
            "SELECT content, category, importance FROM long_term_memory
             ORDER BY importance DESC, last_accessed DESC
             LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return String::new(),
        };

        let memories: Vec<String> = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                let content: String = row.get(0)?;
                let category: String = row.get(1)?;
                let importance: f32 = row.get(2)?;
                Ok(format!("[{} (importance: {:.1})] {}", category, importance, content))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        if memories.is_empty() {
            "[No memories stored yet]".to_string()
        } else {
            memories.join("\n  ")
        }
    }

    pub async fn forget(&self, memory_id: &str) -> Result<bool, String> {
        let conn = rusqlite::Connection::open(DEFAULT_MEMORY_DB_NAME)
            .map_err(|e| format!("DB connect: {}", e))?;

        let deleted = conn
            .execute(
                "DELETE FROM long_term_memory WHERE id = ?1",
                rusqlite::params![memory_id],
            )
            .map_err(|e| format!("Delete: {}", e))?;

        self.vector_store.delete(memory_id).await;

        if let Ok(mut stats) = self.stats.try_write() {
            stats.total_entries = stats.total_entries.saturating_sub(deleted);
        }

        info!(id = %memory_id, deleted = deleted, "Memory forgotten");
        Ok(deleted > 0)
    }

    pub async fn prune_expired(&self, ttl_days: u32, min_importance: f32) -> Result<usize, String> {
        let conn = rusqlite::Connection::open(DEFAULT_MEMORY_DB_NAME)
            .map_err(|e| format!("DB connect: {}", e))?;

        let cutoff = chrono::Utc::now().timestamp() - (ttl_days as i64 * 86400);

        let expired: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM long_term_memory
                     WHERE last_accessed < ?1 AND importance < ?2"
                )
                .map_err(|e| format!("Prepare: {}", e))?;

            let mapped = stmt.query_map(rusqlite::params![cutoff, min_importance], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("Query: {}", e))?;
            mapped.filter_map(|r| r.ok()).collect()
        };

        let count = expired.len();
        for id in &expired {
            conn.execute("DELETE FROM long_term_memory WHERE id = ?1", rusqlite::params![id]).ok();
            self.vector_store.delete(id).await;
        }

        if count > 0 {
            if let Ok(mut stats) = self.stats.try_write() {
                stats.total_entries = stats.total_entries.saturating_sub(count);
            }
            info!(pruned = count, ttl_days = ttl_days, min_importance = min_importance, "Expired memories pruned");
        }

        Ok(count)
    }

    pub async fn forget_by_category(&self, category: &str) -> Result<usize, String> {
        let conn = rusqlite::Connection::open(DEFAULT_MEMORY_DB_NAME)
            .map_err(|e| format!("DB connect: {}", e))?;

        let ids: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT id FROM long_term_memory WHERE category = ?1")
                .map_err(|e| format!("Prepare: {}", e))?;

            let mapped = stmt.query_map(rusqlite::params![category], |row| row.get::<_, String>(0))
                .map_err(|e| format!("Query: {}", e))?;
            mapped.filter_map(|r| r.ok()).collect()
        };

        let count = ids.len();
        for id in &ids {
            conn.execute("DELETE FROM long_term_memory WHERE id = ?1", rusqlite::params![id]).ok();
            self.vector_store.delete(id).await;
        }

        if count > 0 {
            if let Ok(mut stats) = self.stats.try_write() {
                stats.total_entries = stats.total_entries.saturating_sub(count);
                stats.categories.remove(category);
            }
            info!(category = %category, removed = count, "Category memories forgotten");
        }

        Ok(count)
    }

    pub async fn stats(&self) -> MemoryStats {
        self.stats.read().await.clone()
    }

    async fn load_from_db(&self) -> Result<(), String> {
        let conn = rusqlite::Connection::open(DEFAULT_MEMORY_DB_NAME)
            .map_err(|e| format!("DB connect: {}", e))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, content, category, importance, created_at, access_count
                 FROM long_term_memory
                 ORDER BY created_at DESC
                 LIMIT 100",
            )
            .map_err(|e| format!("Prepare: {}", e))?;

        let entries: Vec<(String, String, String, f32, i64, u64)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .map_err(|e| format!("Query: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        let mut stats = MemoryStats::default();
        for (id, content, category, importance, created_at, _access_count) in entries {
            let _ = self
                .vector_store
                .add_with_importance(&content, &category, importance)
                .await;

            stats.total_entries += 1;
            *stats.categories.entry(category).or_insert(0) += 1;
            stats.oldest_entry_secs = Some(
                stats
                    .oldest_entry_secs
                    .unwrap_or(created_at)
                    .min(created_at),
            );
        }

        let loaded_count = stats.total_entries;
        *self.stats.write().await = stats;
        info!(loaded = loaded_count, "Memories loaded from DB");
        Ok(())
    }
}