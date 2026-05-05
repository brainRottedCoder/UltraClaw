// ============================================================================
// ULTRACLAW — db.rs
// ============================================================================
// Short-term conversational context storage — per-room message history.
//
// ARCHITECTURE:
// This is the agent's "short-term memory" (working memory). Each Chat
// thread gets an isolated conversation timeline. When the LLM needs
// context for a response, we pull the last N messages from this database.
//
// WHY SQLITE INSTEAD OF IN-MEMORY VECS?
// - A Vec<String> per chat would grow unboundedly as conversations continue.
//   With 100 chats × 1000 messages × 500 bytes each = 50MB of RAM just for
//   conversation history. That's unacceptable on a Raspberry Pi or phone.
// - SQLite stores everything on disk. The page cache is capped at 2MB.
//   We only load the last `context_window_size` messages into RAM when needed.
// - SQLite handles concurrent access (WAL mode), crash recovery, and data
//   integrity for free. A Vec would lose all history on crash.
//
// ENERGY OPTIMIZATION:
// - Index on `chat_id` makes context retrieval O(log N) via B-tree.
// - WAL mode uses sequential I/O (append-only log), which is 10-100x
//   faster than random writes on flash storage (SSDs, SD cards, eMMC).
// - Prepared statements are cached by SQLite, avoiding repeated SQL parsing.
// ============================================================================

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// A single chat message in the conversation history.
///
/// # Memory Layout
/// ~80 bytes for the struct header + actual string content.
/// Typically ~200-500 bytes total per message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Role: "system", "user", or "assistant"
    pub role: String,
    /// The message content (plain text, stripped of formatting)
    pub content: String,
}

/// SQLite-backed conversation database.
///
/// # Memory Layout
/// - `conn`: SQLite connection handle (~8 bytes) + page cache (~2MB)
/// - Total: ~2MB fixed overhead regardless of conversation count.
pub struct ConversationDb {
    conn: Connection,
}

impl ConversationDb {
    /// Open or create the conversation database.
    ///
    /// Initializes the schema and sets performance pragmas optimized for
    /// an embedded agent on resource-constrained hardware.
    pub fn open(db_path: &str) -> Result<Self, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open conversation DB: {}", e))?;

        // --- PERFORMANCE PRAGMAS ---

        // WAL mode: allows concurrent reads during writes.
        // Critical because the event loop reads context while other handlers
        // may be writing new messages simultaneously.
        // Energy benefit: reduces lock contention → fewer CPU cycles wasted spinning.
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("WAL failed: {}", e))?;

        // Limit page cache to 512 pages × 4KB = 2MB.
        // This caps SQLite's RAM usage regardless of database size.
        // On a Raspberry Pi with 1GB RAM, this is a responsible budget.
        conn.execute_batch("PRAGMA cache_size=512;")
            .map_err(|e| format!("Cache size failed: {}", e))?;

        // NORMAL synchronous mode: fsync on commit but not on every write.
        // Trades a tiny durability risk (losing last transaction on power loss)
        // for 2-5x faster writes. Acceptable for chat history.
        // Energy benefit: fewer fsync syscalls → fewer disk flushes → lower power.
        conn.execute_batch("PRAGMA synchronous=NORMAL;")
            .map_err(|e| format!("Synchronous mode failed: {}", e))?;

        // Create the conversations table with an index on chat_id.
        // The rowid is the implicit primary key (auto-incrementing integer),
        // which is used for ordering messages chronologically.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations (
                chat_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_conv_chat ON conversations(chat_id, timestamp);",
        )
        .map_err(|e| format!("Schema creation failed: {}", e))?;

        Ok(Self { conn })
    }

    /// Append a message to a chat's conversation history.
    ///
    /// This is O(1) amortized — SQLite appends to the WAL file sequentially.
    /// No existing pages are modified until the next checkpoint.
    pub fn append_message(
        &self,
        chat_id: &str,
        role: &str,
        content: &str,
    ) -> Result<(), String> {
        let timestamp = chrono::Utc::now().timestamp();
        self.conn
            .execute(
                "INSERT INTO conversations (chat_id, role, content, timestamp)
                 VALUES (?1, ?2, ?3, ?4)",
                params![chat_id, role, content, timestamp],
            )
            .map_err(|e| format!("Insert failed: {}", e))?;
        Ok(())
    }

    /// Retrieve the last N messages for a chat.
    ///
    /// # Memory Usage
    /// Returns at most `limit` messages. With limit=20 and ~300 bytes/message,
    /// this allocates ~6KB of heap memory — trivial.
    ///
    /// The query uses the (chat_id, timestamp) index, so even with millions
    /// of total messages, retrieval is O(log N + limit).
    pub fn get_context(
        &self,
        chat_id: &str,
        limit: usize,
    ) -> Result<Vec<ChatMessage>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT role, content FROM conversations
                 WHERE chat_id = ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| format!("Prepare failed: {}", e))?;

        let messages: Vec<ChatMessage> = stmt
            .query_map(params![chat_id, limit as i64], |row| {
                Ok(ChatMessage {
                    role: row.get(0)?,
                    content: row.get(1)?,
                })
            })
            .map_err(|e| format!("Query failed: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        // Reverse because we queried DESC (newest first) but need
        // chronological order (oldest first) for the LLM context.
        let mut messages = messages;
        messages.reverse();
        Ok(messages)
    }

    /// Clear all conversation history for a chat.
    ///
    /// Used when a user explicitly asks to reset context or when
    /// the chat is detected as a new conversation.
    #[allow(dead_code)]
    pub fn clear_context(&self, chat_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM conversations WHERE chat_id = ?1",
                params![chat_id],
            )
            .map_err(|e| format!("Clear failed: {}", e))?;
        Ok(())
    }

    /// Prune old messages across all chats to keep the database compact.
    ///
    /// Removes messages older than `max_age_days`. This is a maintenance
    /// operation — run daily or on startup.
    ///
    /// After pruning, we run PRAGMA incremental_vacuum to return freed
    /// pages to the OS, actually reducing the file size on disk.
    #[allow(dead_code)]
    pub fn prune_old(&self, max_age_days: i64) -> Result<usize, String> {
        let cutoff = chrono::Utc::now().timestamp() - (max_age_days * 86400);

        let deleted = self
            .conn
            .execute(
                "DELETE FROM conversations WHERE timestamp < ?1",
                params![cutoff],
            )
            .map_err(|e| format!("Prune failed: {}", e))?;

        // Reclaim disk space from deleted rows.
        // incremental_vacuum returns freed pages to the filesystem.
        let _ = self.conn.execute_batch("PRAGMA incremental_vacuum;");

        Ok(deleted)
    }
}
