// ============================================================================
// ULTRACLAW — Matrix Connector
// ============================================================================
// Native Matrix integration via the `matrix-sdk` library.
//
// MEMORY OPTIMIZATION:
// - Uses `rustls` backend to avoid OpenSSL.
// - Long-polling for events with minimal memory footprint.
// - Event processing in async tasks to avoid blocking.
//
// ENVIRONMENT VARIABLES:
// - ULTRACLAW_MATRIX_HOMESERVER: e.g., "https://matrix.org"
// - ULTRACLAW_MATRIX_USER: e.g., "@bot:matrix.org"
// - ULTRACLAW_MATRIX_TOKEN: Access token from Element or Element Synapse

use crate::config::Config;
use crate::connector::Connector;
use crate::db::{ChatMessage, ConversationDb};
use crate::formatter::Platform;
use crate::inference::InferenceEngine;
use crate::mcp::McpClient;
use crate::memory::MemoryStore;
use crate::session::SessionManager;
use crate::skill::SkillRegistry;
use crate::soul::Soul;
use crate::tools;

use anyhow::Context as _;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MatrixRoom {
    room_id: String,
    name: Option<String>,
}

pub struct MatrixConnector {
    homeserver: String,
    user_id: String,
    access_token: String,
}

impl MatrixConnector {
    pub fn new(homeserver: &str, user_id: &str, access_token: &str) -> Self {
        Self {
            homeserver: homeserver.to_string(),
            user_id: user_id.to_string(),
            access_token: access_token.to_string(),
        }
    }

    pub fn from_config(config: &Config) -> Option<Self> {
        let homeserver = config.matrix_homeserver.clone();
        let user_id = config.matrix_user.clone();
        let access_token = config.matrix_token.clone()?;

        if homeserver.is_empty() || user_id.is_empty() || access_token.is_empty() {
            return None;
        }

        Some(Self::new(&homeserver, &user_id, &access_token))
    }

    fn http_client(&self) -> reqwest::Client {
        reqwest::Client::new()
    }

    async fn sync(&self) -> Result<SyncResponse, String> {
        let url = format!(
            "{}/_matrix/client/r0/sync?timeout=30000",
            self.homeserver
        );

        let response = self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .await
            .map_err(|e| format!("Sync request failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Sync failed ({}): {}", status, body));
        }

        response
            .json::<SyncResponse>()
            .await
            .map_err(|e| format!("Failed to parse sync response: {}", e))
    }

    async fn get_room_messages(&self, room_id: &str, from: &str, limit: u64) -> Result<MessagesResponse, String> {
        let url = format!(
            "{}/_matrix/client/r0/rooms/{}/messages?from={}&limit={}",
            self.homeserver, room_id, from, limit
        );

        let response = self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .await
            .map_err(|e| format!("Get messages request failed: {}", e))?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Get messages failed: {}", body));
        }

        response
            .json::<MessagesResponse>()
            .await
            .map_err(|e| format!("Failed to parse messages response: {}", e))
    }

    async fn send_message(&self, room_id: &str, body: &str) -> Result<(), String> {
        let txn_id = format!("{}", chrono::Utc::now().timestamp_millis());
        let url = format!(
            "{}/_matrix/client/r0/rooms/{}/send/m.room.message/{}",
            self.homeserver, room_id, txn_id
        );

        let payload = serde_json::json!({
            "msgtype": "m.text",
            "body": body
        });

        let response = self
            .http_client()
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("Send message request failed: {}", e))?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Send message failed: {}", body));
        }

        Ok(())
    }

    async fn get_login_user_info(&self) -> Result<serde_json::Value, String> {
        let url = format!("{}/_matrix/client/r0/account/whoami", self.homeserver);

        let response = self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .await
            .map_err(|e| format!("Whoami request failed: {}", e))?;

        if !response.status().is_success() {
            return Err("Not authenticated with Matrix".to_string());
        }

        response
            .json()
            .await
            .map_err(|e| format!("Failed to parse whoami response: {}", e))
    }

    async fn get_room_name(&self, room_id: &str) -> Option<String> {
        let url = format!(
            "{}/_matrix/client/r0/rooms/{}/state/m.room.name",
            self.homeserver, room_id
        );

        let response = self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .await
            .ok()?;

        if !response.status().is_success() {
            return None;
        }

        let data: serde_json::Value = response.json().await.ok()?;
        data.get("name")?.as_str().map(String::from)
    }
}

#[derive(Debug, Deserialize)]
struct SyncResponse {
    next_batch: String,
    #[serde(default)]
    rooms: RoomsData,
}

#[derive(Debug, Deserialize, Default)]
struct RoomsData {
    #[serde(default)]
    join: std::collections::HashMap<String, JoinRoomData>,
}

#[derive(Debug, Deserialize, Default)]
struct JoinRoomData {
    #[serde(default)]
    unread_notifications: UnreadNotifications,
}

#[derive(Debug, Deserialize, Default)]
struct UnreadNotifications {
    #[serde(default)]
    highlight_count: Option<u64>,
    #[serde(default)]
    notification_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    chunk: Vec<MessageEvent>,
    end: String,
}

#[derive(Debug, Deserialize)]
struct MessageEvent {
    sender: String,
    content: EventContent,
    #[serde(rename = "event_id")]
    event_id: String,
    origin_server_ts: u64,
    #[serde(rename = "type")]
    event_type: String,
}

#[derive(Debug, Deserialize)]
struct EventContent {
    body: String,
    #[serde(rename = "msgtype")]
    msgtype: Option<String>,
}

#[async_trait]
impl Connector for MatrixConnector {
    fn name(&self) -> &str {
        "Matrix"
    }

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
    ) -> anyhow::Result<()> {
        info!("Matrix Connector starting for user {}", self.user_id);

        // Verify authentication
        match self.get_login_user_info().await {
            Ok(info) => {
                info!(
                    "Authenticated as user_id: {}",
                    info.get("user_id").and_then(|v| v.as_str()).unwrap_or("unknown")
                );
            }
            Err(e) => {
                error!("Matrix authentication failed: {}", e);
                return Err(anyhow::anyhow!("Matrix authentication failed: {}", e));
            }
        }

        let mut next_batch = String::new();
        let mut last_sync = std::time::Instant::now();

        loop {
            // Sync and get new events
            match self.sync().await {
                Ok(sync_resp) => {
                    // Process joined rooms
                    for (room_id, _room_data) in sync_resp.rooms.join.iter() {
                        self.process_room(
                            room_id,
                            &engine,
                            &db,
                            &memory,
                            &sessions,
                            &soul,
                            &skills,
                            &mcp,
                            &config,
                        )
                        .await;
                    }

                    next_batch = sync_resp.next_batch;
                    last_sync = std::time::Instant::now();
                }
                Err(e) => {
                    error!("Sync error: {}", e);
                    // Back off on error
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }

            // Periodic keepalive
            if last_sync.elapsed() > tokio::time::Duration::from_secs(300) {
                info!("Matrix connector heartbeat");
                last_sync = std::time::Instant::now();
            }
        }
    }
}

impl MatrixConnector {
    async fn process_room(
        &self,
        room_id: &str,
        engine: &Arc<dyn InferenceEngine>,
        db: &Arc<Mutex<ConversationDb>>,
        memory: &Arc<Mutex<MemoryStore>>,
        sessions: &Arc<Mutex<SessionManager>>,
        soul: &Arc<Soul>,
        skills: &Arc<SkillRegistry>,
        mcp: &Option<Arc<McpClient>>,
        config: &Arc<Config>,
    ) {
        // Get recent messages from the room
        match self.get_room_messages(room_id, "", 20).await {
            Ok(messages_resp) => {
                for event in messages_resp.chunk.iter().rev() {
                    if event.event_type != "m.room.message" {
                        continue;
                    }
                    if event.content.msgtype.as_deref() != Some("m.text") {
                        continue;
                    }
                    if event.sender == self.user_id {
                        continue;
                    }

                    let content = &event.content.body;
                    if content.trim().is_empty() {
                        continue;
                    }

                    let room_name = self.get_room_name(room_id).await;
                    info!(
                        room_id = %room_id,
                        room_name = ?room_name,
                        sender = %event.sender,
                        "Received Matrix message"
                    );

                    // Process the message through the brain loop
                    if let Err(e) = self
                        .process_message(
                            room_id,
                            content,
                            engine,
                            db,
                            memory,
                            sessions,
                            soul,
                            skills,
                            mcp,
                            config,
                        )
                        .await
                    {
                        error!("Failed to process message: {}", e);
                    }
                }
            }
            Err(e) => {
                warn!("Failed to get messages for room {}: {}", room_id, e);
            }
        }
    }

    async fn process_message(
        &self,
        room_id: &str,
        content: &str,
        engine: &Arc<dyn InferenceEngine>,
        db: &Arc<Mutex<ConversationDb>>,
        memory: &Arc<Mutex<MemoryStore>>,
        sessions: &Arc<Mutex<SessionManager>>,
        soul: &Arc<Soul>,
        skills: &Arc<SkillRegistry>,
        mcp: &Option<Arc<McpClient>>,
        config: &Arc<Config>,
    ) -> Result<(), String> {
        let platform = Platform::Matrix;

        // 1. Ensure Session Exists
        {
            let mut sessions = sessions.lock().await;
            sessions.get_or_create(room_id, platform);
        }

        // 2. Store User Message
        {
            let db = db.lock().await;
            db.append_message(room_id, "user", content)
                .map_err(|e| format!("Failed to store user message: {}", e))?;
        }

        // 3. Load Context
        let context = {
            let db = db.lock().await;
            db.get_context(room_id, config.context_window_size)
                .unwrap_or_default()
        };

        // 4. Recall Long-term Memory
        let memory_context = {
            let mem = memory.lock().await;
            mem.summarize_for_context(room_id, 200).unwrap_or(None)
        };

        // 5. Build System Prompt
        let session_context = {
            let sessions = sessions.lock().await;
            sessions.get_session_context(room_id)
        };

        let room_name = self.get_room_name(room_id).await;
        let system_msg = soul.build_system_message(
            Some(&format!(
                "User is on Matrix{}.",
                room_name.as_ref().map(|n| format!(" in room '{}'", n)).unwrap_or_default()
            )),
            session_context.as_deref(),
            memory_context.as_deref(),
        );

        let mut messages = Vec::new();
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: system_msg,
        });
        messages.extend(context);

        // 6. Inference with streaming
        let tool_schema = skills.to_tool_schema();
        let mut stream = engine.infer_stream(
            messages.clone(),
            Some(tool_schema),
            soul.temperature,
            soul.max_tokens,
        );

        let mut response = String::new();
        let mut streamed_response = String::new();
        let mut has_response = false;

        loop {
            match stream.next().await {
                Some(Ok(token)) => {
                    response.push_str(&token);
                    streamed_response.push_str(&token);
                    has_response = true;

                    // Send partial response if buffer is large enough
                    if streamed_response.len() > 100 || streamed_response.contains('\n') {
                        if let Err(e) = self.send_message(room_id, &streamed_response).await {
                            warn!("Failed to send partial response: {}", e);
                        }
                        streamed_response.clear();
                    }
                }
                Some(Err(e)) => {
                    error!("Inference error: {}", e);
                    let _ = self.send_message(room_id, "I'm having trouble thinking right now.").await;
                    return Err(format!("Inference error: {}", e));
                }
                None => break,
            }
        }

        // Send remaining streamed response
        if !streamed_response.is_empty() {
            if let Err(e) = self.send_message(room_id, &streamed_response).await {
                warn!("Failed to send final partial response: {}", e);
            }
        }

        // If no response was sent at all, send the full response
        if has_response && response.len() > 0 && streamed_response.is_empty() {
            // Response already sent in chunks
        } else if !response.is_empty() {
            if let Err(e) = self.send_message(room_id, &response).await {
                warn!("Failed to send response: {}", e);
            }
        }

        // 7. Tool Execution
        let tool_calls = tools::parse_tool_calls(&response);
        let final_response = if !tool_calls.is_empty() {
            info!("Executing {} tool(s)...", tool_calls.len());

            let tool_output = tools::execute_tool_calls(&tool_calls, skills, mcp.as_deref()).await;

            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.clone(),
            });
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: format!("Tool execution results:\n{}", tool_output),
            });

            let mut re_stream = engine.infer_stream(
                messages.clone(),
                None,
                soul.temperature,
                soul.max_tokens,
            );
            let mut re_response = String::new();

            loop {
                match re_stream.next().await {
                    Some(Ok(token)) => {
                        re_response.push_str(&token);
                    }
                    Some(Err(e)) => {
                        error!("Re-inference error: {}", e);
                        break;
                    }
                    None => break,
                }
            }

            if !re_response.is_empty() {
                if let Err(e) = self.send_message(room_id, &re_response).await {
                    warn!("Failed to send tool response: {}", e);
                }
                re_response
            } else {
                response
            }
        } else {
            response
        };

        // 8. Store Response
        {
            let db = db.lock().await;
            db.append_message(room_id, "assistant", &final_response)
                .map_err(|e| format!("Failed to store assistant response: {}", e))?;
        }

        Ok(())
    }
}

use futures::StreamExt;