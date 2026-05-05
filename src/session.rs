// ============================================================================
// ULTRACLAW — session.rs
// ============================================================================
// Session lifecycle management — tracks active multi-turn conversations
// and evicts idle sessions to reclaim RAM.
//
// ARCHITECTURE:
// Each chat thread gets one Session when the first message arrives.
// The session tracks metadata (platform, turn count, timestamps) that
// helps the Soul tailor its behavior per-conversation.
//
// MEMORY OPTIMIZATION:
// - Sessions are stored in a HashMap<String, Session>. Each Session is
//   ~120 bytes. With the default max of 256 sessions, total RAM usage
//   is ~30KB — negligible.
// - When a session expires (idle > TTL), it's fully evicted from the
//   HashMap. The key String and Session struct are both deallocated.
// - The `max_sessions` cap prevents memory growth even under high load.
//   When the cap is hit, the oldest idle session is evicted (LRU).
//
// ENERGY OPTIMIZATION:
// - `expire_idle()` is O(N) where N = number of sessions. With max 256
//   sessions, this completes in microseconds — called once per minute.
// - No background timer thread. Expiration is checked lazily on incoming
//   messages or via a periodic `tokio::time::interval` in the event loop.
//   The interval uses tokio's timer wheel, which is a single kernel timer
//   shared across all intervals — zero additional OS resources.
// ============================================================================

use crate::formatter::Platform;
use std::collections::HashMap;

/// The lifecycle state of a session.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    /// Actively processing messages.
    Active,
    /// No messages received recently, but still in memory.
    #[allow(dead_code)]
    Idle,
    /// Marked for eviction. Will be removed on next sweep.
    #[allow(dead_code)]
    Expired,
}

/// A single conversation session.
///
/// # Memory Layout (approximate)
/// - session_id: 24 bytes (String) + 36 bytes content (UUID)
/// - room_id: 24 bytes (String) + ~30-60 bytes content
/// - platform: 1 byte (enum)
/// - timestamps: 16 bytes (2x i64)
/// - turn_count: 4 bytes (u32)
/// - state: 1 byte (enum)
/// Total: ~120-150 bytes per session
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique session ID (UUID v4).
    pub session_id: String,
    /// The generic chat_id this session is bound to.
    #[allow(dead_code)]
    pub chat_id: String,
    /// The platform detected from the room_id (WhatsApp, Discord, etc.)
    pub platform: Platform,
    /// Unix timestamp when the session was created.
    pub created_at: i64,
    /// Unix timestamp of the last message in this session.
    pub last_active: i64,
    /// Number of user turns (messages) in this session.
    pub turn_count: u32,
    /// Current lifecycle state.
    pub state: SessionState,
}

/// Manages all active sessions.
///
/// # Memory Bound
/// HashMap overhead: ~48 bytes per entry (key + value pointers + hash).
/// With Session at ~140 bytes and key at ~50 bytes: ~238 bytes per entry.
/// Max 256 sessions = ~60KB. This is bounded and predictable.
pub struct SessionManager {
    /// Active sessions keyed by chat_id.
    sessions: HashMap<String, Session>,
    /// Session idle timeout in seconds.
    ttl_secs: u64,
    /// Maximum number of concurrent sessions.
    max_sessions: usize,
}

impl SessionManager {
    /// Create a new session manager.
    pub fn new(ttl_secs: u64, max_sessions: usize) -> Self {
        Self {
            sessions: HashMap::with_capacity(max_sessions.min(64)),
            ttl_secs,
            max_sessions,
        }
    }

    /// Get an existing session or create a new one for the given chat.
    ///
    /// If the session cap is reached, the oldest idle session is evicted
    /// to make room (LRU eviction).
    pub fn get_or_create(&mut self, chat_id: &str, platform: Platform) -> &Session {
        let now = chrono::Utc::now().timestamp();

        // If session exists, touch it and return
        if self.sessions.contains_key(chat_id) {
            let session = self.sessions.get_mut(chat_id).unwrap();
            session.last_active = now;
            session.turn_count += 1;
            session.state = SessionState::Active;
            return self.sessions.get(chat_id).unwrap();
        }

        // Evict if at capacity
        if self.sessions.len() >= self.max_sessions {
            self.evict_oldest();
        }

        // Create new session
        let session = Session {
            session_id: uuid::Uuid::new_v4().to_string(),
            chat_id: chat_id.to_string(),
            platform,
            created_at: now,
            last_active: now,
            turn_count: 1,
            state: SessionState::Active,
        };

        self.sessions.insert(chat_id.to_string(), session);
        self.sessions.get(chat_id).unwrap()
    }

    /// Update the last-active timestamp for a chat's session.
    #[allow(dead_code)]
    pub fn touch(&mut self, chat_id: &str) {
        if let Some(session) = self.sessions.get_mut(chat_id) {
            session.last_active = chrono::Utc::now().timestamp();
            session.state = SessionState::Active;
        }
    }

    /// Sweep expired sessions and evict them.
    ///
    /// Sessions idle longer than `ttl_secs` are removed from the HashMap,
    /// fully deallocating their memory. This should be called periodically
    /// (e.g., every 60 seconds from the event loop).
    ///
    /// Returns the number of sessions evicted.
    pub fn expire_idle(&mut self) -> usize {
        let now = chrono::Utc::now().timestamp();
        let ttl = self.ttl_secs as i64;

        // Collect expired chat_ids first to avoid borrowing issues.
        // This temporary Vec is at most `max_sessions` * ~50 bytes.
        let expired: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, session)| now - session.last_active > ttl)
            .map(|(chat_id, _)| chat_id.clone())
            .collect();

        let count = expired.len();
        for chat_id in expired {
            self.sessions.remove(&chat_id);
            // Memory freed: the Session struct + the chat_id String key
        }

        count
    }

    /// Evict the single oldest idle session to make room for a new one.
    fn evict_oldest(&mut self) {
        if let Some(oldest_chat) = self
            .sessions
            .iter()
            .min_by_key(|(_, s)| s.last_active)
            .map(|(k, _)| k.clone())
        {
            self.sessions.remove(&oldest_chat);
        }
    }

    /// Get session context as a human-readable string for the system prompt.
    ///
    /// Returns metadata like turn count and session duration, which helps
    /// the LLM understand the conversation's stage.
    pub fn get_session_context(&self, chat_id: &str) -> Option<String> {
        self.sessions.get(chat_id).map(|session| {
            let now = chrono::Utc::now().timestamp();
            let duration_mins = (now - session.created_at) / 60;
            format!(
                "Session: {} | Platform: {:?} | Turn: {} | Duration: {} min",
                &session.session_id[..8], // Short ID prefix for readability
                session.platform,
                session.turn_count,
                duration_mins
            )
        })
    }

    /// Get the session for a chat, if it exists.
    #[allow(dead_code)]
    pub fn get(&self, chat_id: &str) -> Option<&Session> {
        self.sessions.get(chat_id)
    }

    /// Get the total number of active sessions.
    #[allow(dead_code)]
    pub fn active_count(&self) -> usize {
        self.sessions.len()
    }
}
