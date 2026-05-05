use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::config::Config;
use crate::db::ConversationDb;
use crate::inference::InferenceEngine;
use crate::memory::MemoryStore;
use crate::session::SessionManager;
use crate::skill::SkillRegistry;
use crate::soul::Soul;
use crate::mcp::McpClient;

/// A Connector is a bridge between a specific platform (Matrix, Discord, CLI)
/// and the core Ultraclaw agent logic.
#[async_trait]
pub trait Connector: Send + Sync {
    /// Run the connector's main loop. This should block until the connector exits.
    async fn run(
        &self,
        engine: Arc<dyn InferenceEngine>,
        db: Arc<Mutex<ConversationDb>>,
        memory: Arc<Mutex<MemoryStore>>,
        sessions: Arc<Mutex<SessionManager>>,
        soul: Arc<Soul>,
        skills: Arc<SkillRegistry>,
        mcp: Option<Arc<McpClient>>,
        config: Arc<Config>,
    ) -> anyhow::Result<()>;

    /// Returns the name of the connector (e.g., "matrix", "cli", "discord").
    fn name(&self) -> &str;
}
