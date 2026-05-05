// ============================================================================
// ULTRACLAW — Webhook Connector
// ============================================================================
// Unified HTTP server for handling incoming webhooks from:
// - Slack (Events API / Slash Commands)
// - Google Chat (HTTP Endpoint)
// - Mattermost (Outgoing Webhooks)

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
use axum::{
    extract::{State, Json},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

pub struct WebhookConnector;

impl WebhookConnector {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Clone)]
struct AppState {
    engine: Arc<dyn InferenceEngine>,
    db: Arc<Mutex<ConversationDb>>,
    memory: Arc<Mutex<MemoryStore>>,
    sessions: Arc<Mutex<SessionManager>>,
    soul: Arc<Soul>,
    skills: Arc<SkillRegistry>,
    mcp: Option<Arc<McpClient>>,
    config: Arc<Config>,
}

#[async_trait]
impl Connector for WebhookConnector {
    fn name(&self) -> &str {
        "Webhook"
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
        let port = config.webhook_port;
        
        let state = AppState {
            engine,
            db,
            memory,
            sessions,
            soul,
            skills,
            mcp,
            config: config.clone(),
        };

        let app = Router::new()
            .route("/webhooks/slack", post(handle_slack))
            .route("/webhooks/google", post(handle_google))
            .route("/webhooks/mattermost", post(handle_mattermost))
            .with_state(state);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        info!("Webhook Connector listening on http://{}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await
            .context("Failed to bind webhook port")?;

        axum::serve(listener, app).await
            .context("Webhook server error")?;

        Ok(())
    }
}

async fn handle_slack(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    if let Some(msg_type) = payload.get("type").and_then(|v| v.as_str()) {
        if msg_type == "url_verification" {
            if let Some(challenge) = payload.get("challenge").and_then(|v| v.as_str()) {
                return (StatusCode::OK, challenge.to_string());
            }
        }
    }

    if let Some(event) = payload.get("event") {
        if let Some(text) = event.get("text").and_then(|v| v.as_str()) {
            let user = event.get("user").and_then(|v| v.as_str()).unwrap_or("unknown");
            let channel = event.get("channel").and_then(|v| v.as_str()).unwrap_or("");
            let bot_token = state.config.slack_bot_token.clone();
            let state_clone = state.clone();
            let text_owned = text.to_string();
            let user_owned = user.to_string();
            let channel_owned = channel.to_string();

            tokio::spawn(async move {
                info!("Slack message from {} in {}: {}", user_owned, channel_owned, text_owned);

                let response = process_brain(
                    state_clone.clone(),
                    user_owned,
                    text_owned,
                    Platform::Slack,
                ).await;

                if let Some(token) = bot_token {
                    send_slack_reply(&token, &channel_owned, &response).await;
                } else {
                    warn!("No SLACK_BOT_TOKEN configured. Cannot reply to Slack message.");
                }
            });
        }
    }

    (StatusCode::OK, "OK".to_string())
}

async fn send_slack_reply(token: &str, channel: &str, text: &str) {
    let client = reqwest::Client::new();
    let payload = serde_json::json!({
        "channel": channel,
        "text": text,
    });

    match client
        .post("https://slack.com/api/chat.postMessage")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
    {
        Ok(resp) => {
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                error!("Slack API error: {}", body);
            } else {
                info!("Slack reply sent to {}", channel);
            }
        }
        Err(e) => {
            error!("Failed to send Slack reply: {}", e);
        }
    }
}

async fn handle_google(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let msg_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    
    if msg_type == "MESSAGE" {
        if let Some(message) = payload.get("message") {
            if let Some(text) = message.get("text").and_then(|v| v.as_str()) {
                let user_name = payload.get("user")
                    .and_then(|u| u.get("displayName"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("User");
                    
                let user_id = payload.get("user")
                    .and_then(|u| u.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown_user");

                info!("Google Chat message from {}: {}", user_name, text);

                let response_text = process_brain(
                    state,
                    user_id.to_string(),
                    text.to_string(),
                    Platform::Unknown
                ).await;

                return (StatusCode::OK, serde_json::json!({ "text": response_text }).to_string());
            }
        }
    }

    (StatusCode::OK, "OK".to_string())
}

async fn handle_mattermost(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    if let Some(token) = &state.config.mattermost_token {
        if let Some(payload_token) = payload.get("token").and_then(|v| v.as_str()) {
            if token != payload_token {
                return (StatusCode::UNAUTHORIZED, "Invalid Token".to_string());
            }
        }
    }

    if let Some(user_id) = payload.get("user_id").and_then(|v| v.as_str()) {
        if let Some(text) = payload.get("text").and_then(|v| v.as_str()) {
            info!("Mattermost message from {}: {}", user_id, text);

            let response_text = process_brain(
                state,
                user_id.to_string(),
                text.to_string(),
                Platform::Unknown
            ).await;

            return (StatusCode::OK, serde_json::json!({ "text": response_text }).to_string());
        }
    }

    (StatusCode::OK, "OK".to_string())
}

async fn process_brain(state: AppState, user_id: String, content: String, platform: Platform) -> String {
    info!(user = %user_id, "Processing webhook message");
    
    {
        let mut sessions = state.sessions.lock().await;
        sessions.get_or_create(&user_id, platform);
    }

    {
        let db = state.db.lock().await;
        if let Err(e) = db.append_message(&user_id, "user", &content) {
            error!("Failed to store user message: {}", e);
        }
    }

    let context = {
        let db = state.db.lock().await;
        db.get_context(&user_id, state.config.context_window_size).unwrap_or_default()
    };

    let memory_context = {
        let mem = state.memory.lock().await;
        mem.summarize_for_context(&user_id, 200).unwrap_or(None)
    };

    let session_context = {
        let sessions = state.sessions.lock().await;
        sessions.get_session_context(&user_id)
    };

    let system_msg = state.soul.build_system_message(
        Some("User is using a Webhook Interface. Format answers as plain text."),
        session_context.as_deref(),
        memory_context.as_deref(),
    );

    let mut messages = Vec::new();
    messages.push(ChatMessage { role: "system".to_string(), content: system_msg });
    messages.extend(context);

    let tool_schema = state.skills.to_tool_schema();
    let response = match state.engine.infer(messages.clone(), Some(tool_schema), state.soul.temperature, state.soul.max_tokens).await {
        Ok(resp) => resp,
        Err(e) => {
            error!("Inference error: {}", e);
            return "I'm having trouble thinking right now.".to_string();
        }
    };

    let tool_calls = tools::parse_tool_calls(&response);
    let final_response = if !tool_calls.is_empty() {
        info!("Executing {} tool(s)...", tool_calls.len());
        let tool_output = tools::execute_tool_calls(&tool_calls, &state.skills, state.mcp.as_deref()).await;

        messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone() });
        messages.push(ChatMessage { role: "system".to_string(), content: format!("Tool execution results:\n{}", tool_output) });

        match state.engine.infer(messages.clone(), None, state.soul.temperature, state.soul.max_tokens).await {
            Ok(resp) => resp,
            Err(e) => {
                error!("Re-inference error: {}", e);
                format!("Tool error: {}", e)
            }
        }
    } else {
        response
    };

    {
        let db = state.db.lock().await;
        if let Err(e) = db.append_message(&user_id, "assistant", &final_response) {
            error!("Failed to store assistant response: {}", e);
        }
    }

    final_response
}