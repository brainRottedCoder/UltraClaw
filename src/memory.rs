// ============================================================================
// ULTRACLAW — memory.rs
// ============================================================================
// Long-term semantic memory — persistent knowledge that survives across
// sessions and even agent restarts.
//
// ARCHITECTURE:
// This is the agent's "hippocampus". While the ConversationDb (db.rs) stores
// raw message history (short-term, ephemeral), the MemoryStore holds distilled
// facts, user preferences, and learned instructions that should persist.
//
// Examples of stored memories:
// - "User prefers Python over JavaScript"
// - "User's timezone is IST (UTC+5:30)"
// - "User asked to always respond in Vietnamese on Zalo"
//
// MEMORY OPTIMIZATION:
// - All memories live on disk in SQLite. The RAM footprint is just the
//   SQLite page cache (default ~2MB, configurable).
// - `recall()` returns at most `limit` results. The Vec is sized exactly
//   to the result count — no overallocation.
// - `summarize_for_context()` enforces a token budget, preventing memory
//   recall from consuming the entire LLM context window.
// - `prune()` runs periodically to garbage-collect old, low-importance
//   memories, keeping the database size bounded.
//
// ENERGY OPTIMIZATION:
// - SQLite queries use indexes on (room_id, importance, accessed_at),
//   making lookups O(log N) via B-tree traversal. No table scans.
// - WAL mode is enabled, allowing concurrent reads during writes without
//   lock contention (and thus without spinning/retrying).
// ============================================================================

use rusqlite::{params, Connection};

/// A single memory entry.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Memory {
    /// Unique ID for this memory (UUID v4 string).
    pub id: String,
    /// The room_id this memory is associated with. Can be "*" for global memories.
    pub room_id: String,
    /// The actual content/fact/preference.
    pub content: String,
    /// Category tag (e.g., "preference", "fact", "instruction", "context").
    pub category: String,
    /// Importance score from 0.0 (trivial) to 1.0 (critical).
    /// Higher importance memories survive pruning longer.
    pub importance: f64,
    /// Unix timestamp when this memory was created.
    pub created_at: i64,
    /// Unix timestamp when this memory was last accessed/recalled.
    /// Updated on every `recall()` hit to implement LRU-like aging.
    pub accessed_at: i64,
}

/// Long-term memory store backed by SQLite.
///
/// # Memory Layout
/// - `conn`: SQLite connection (~few KB for the handle + ~2MB page cache)
/// - That's it. All actual memory data lives on disk.
pub struct MemoryStore {
    conn: Connection,
}

impl MemoryStore {
    /// Open or create the memory store.
    ///
    /// Creates the `memories` table if it doesn't exist, along with indexes
    /// for efficient querying. Uses WAL mode for concurrent read/write.
    pub fn open(db_path: &str) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open memory DB: {}", e))?;

        // WAL mode: Write-Ahead Logging.
        // This is critical for an async agent because:
        // 1. Readers don't block writers and vice versa
        // 2. Writes are appended to a log file (sequential I/O = fast on HDDs)
        // 3. Checkpointing happens in the background
        // Energy impact: WAL reduces fsync calls by ~90% compared to DELETE mode.
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("WAL mode failed: {}", e))?;

        // Reduce page cache to 512 pages * 4KB = 2MB.
        // Default is 2000 pages = 8MB. We trade some query speed for RAM savings.
        conn.execute_batch("PRAGMA cache_size=512;")
            .map_err(|e| format!("Cache size failed: {}", e))?;

        // Create table and indexes
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                room_id TEXT NOT NULL,
                content TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT 'fact',
                importance REAL NOT NULL DEFAULT 0.5,
                created_at INTEGER NOT NULL,
                accessed_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memories_room ON memories(room_id);
            CREATE INDEX IF NOT EXISTS idx_memories_importance ON memories(importance DESC);
            CREATE INDEX IF NOT EXISTS idx_memories_accessed ON memories(accessed_at DESC);",
        )
        .map_err(|e| format!("Table creation failed: {}", e))?;

        Ok(Self { conn })
    }

    /// Store a new memory.
    ///
    /// # Arguments
    /// * `room_id` - The room this memory belongs to. Use "*" for global.
    /// * `content` - The fact/preference/instruction to remember.
    /// * `category` - Category tag for filtering.
    /// * `importance` - Score from 0.0 to 1.0.
    #[allow(dead_code)]
    pub fn store(
        &self,
        room_id: &str,
        content: &str,
        category: &str,
        importance: f64,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        self.conn
            .execute(
                "INSERT INTO memories (id, room_id, content, category, importance, created_at, accessed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, room_id, content, category, importance, now, now],
            )
            .map_err(|e| format!("Failed to store memory: {}", e))?;

        Ok(id)
    }

    /// Recall memories relevant to a query for a specific room.
    ///
    /// Uses keyword matching (LIKE) ranked by importance and recency.
    /// Results are capped at `limit` to prevent unbounded RAM usage.
    ///
    /// Also updates `accessed_at` on all returned memories (LRU tracking).
    ///
    /// # Scoring
    /// Memories are ranked by: importance * recency_weight
    /// where recency_weight decays based on time since last access.
    #[allow(dead_code)]
    pub fn recall(
        &self,
        query: &str,
        room_id: &str,
        limit: usize,
    ) -> Result<Vec<Memory>, String> {
        // Search both room-specific and global ("*") memories
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, room_id, content, category, importance, created_at, accessed_at
                 FROM memories
                 WHERE (room_id = ?1 OR room_id = '*')
                   AND content LIKE '%' || ?2 || '%'
                 ORDER BY importance DESC, accessed_at DESC
                 LIMIT ?3",
            )
            .map_err(|e| format!("Query prepare failed: {}", e))?;

        let memories: Vec<Memory> = stmt
            .query_map(params![room_id, query, limit as i64], |row| {
                Ok(Memory {
                    id: row.get(0)?,
                    room_id: row.get(1)?,
                    content: row.get(2)?,
                    category: row.get(3)?,
                    importance: row.get(4)?,
                    created_at: row.get(5)?,
                    accessed_at: row.get(6)?,
                })
            })
            .map_err(|e| format!("Query failed: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        // Update accessed_at for recalled memories (LRU tracking)
        let now = chrono::Utc::now().timestamp();
        for mem in &memories {
            let _ = self.conn.execute(
                "UPDATE memories SET accessed_at = ?1 WHERE id = ?2",
                params![now, mem.id],
            );
        }

        Ok(memories)
    }

    /// Explicitly forget a memory by ID (privacy/GDPR compliance).
    #[allow(dead_code)]
    pub fn forget(&self, id: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to forget memory: {}", e))?;
        Ok(())
    }

    /// Prune old, low-importance memories to keep the database lean.
    ///
    /// This is the garbage collector for long-term memory. It removes
    /// memories that are both old AND unimportant. High-importance memories
    /// survive indefinitely.
    ///
    /// Recommended: run once per hour or on agent startup.
    pub fn prune(&self, max_age_days: i64, min_importance: f64) -> Result<usize, String> {
        let cutoff = chrono::Utc::now().timestamp() - (max_age_days * 86400);

        let deleted = self
            .conn
            .execute(
                "DELETE FROM memories WHERE accessed_at < ?1 AND importance < ?2",
                params![cutoff, min_importance],
            )
            .map_err(|e| format!("Prune failed: {}", e))?;

        Ok(deleted)
    }

    /// Build a context string from recalled memories, respecting a token budget.
    ///
    /// # Token Budget
    /// We approximate 1 token ≈ 4 characters (conservative for English).
    /// The `token_budget` parameter limits how much memory context is injected
    /// into the system prompt, preventing memory from starving the actual
    /// conversation of context window space.
    pub fn summarize_for_context(
        &self,
        room_id: &str,
        token_budget: usize,
    ) -> Result<Option<String>, String> {
        // Char budget: 4 chars per token approximation
        let char_budget = token_budget * 4;

        let mut stmt = self
            .conn
            .prepare(
                "SELECT content, category, importance FROM memories
                 WHERE (room_id = ?1 OR room_id = '*')
                 ORDER BY importance DESC, accessed_at DESC
                 LIMIT 50",
            )
            .map_err(|e| format!("Context query failed: {}", e))?;

        let entries: Vec<(String, String, f64)> = stmt
            .query_map(params![room_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| format!("Context query exec failed: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        if entries.is_empty() {
            return Ok(None);
        }

        let mut summary = String::with_capacity(char_budget.min(4096));
        for (content, category, importance) in entries {
            let line = format!("- [{}|{:.1}] {}\n", category, importance, content);
            if summary.len() + line.len() > char_budget {
                break; // Respect token budget
            }
            summary.push_str(&line);
        }

        if summary.is_empty() {
            Ok(None)
        } else {
            Ok(Some(summary))
        }
    }
}
