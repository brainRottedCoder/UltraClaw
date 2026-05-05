// ============================================================================
// ULTRACLAW — Discord Connector
// ============================================================================
// Native Discord integrations via the `serenity` library.
//
// MEMORY OPTIMIZATION:
// - Uses `rustls` backend to avoid OpenSSL.
// - Events are processed in strict async tasks to avoid blocking the gateway shard.

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
use futures::StreamExt;
use serenity::all::{GatewayIntents, Message, Ready};
use serenity::client::{Client, Context as SerenityContext, EventHandler};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

pub struct DiscordConnector;

impl DiscordConnector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Connector for DiscordConnector {
    fn name(&self) -> &str {
        "Discord"
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
        let token = config
            .discord_token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Discord token not configured"))?;

        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;

        let handler = Handler {
            engine,
            db,
            memory,
            sessions,
            soul,
            skills,
            mcp,
            config: config.clone(),
            bot_id: Mutex::new(None),
        };

        let mut client = Client::builder(token, intents)
            .event_handler(handler)
            .await
            .context("Error creating Discord client")?;

        info!("Discord Connector starting...");

        client.start().await.context("Discord client error")?;

        Ok(())
    }
}

struct Handler {
    engine: Arc<dyn InferenceEngine>,
    db: Arc<Mutex<ConversationDb>>,
    memory: Arc<Mutex<MemoryStore>>,
    sessions: Arc<Mutex<SessionManager>>,
    soul: Arc<Soul>,
    skills: Arc<SkillRegistry>,
    mcp: Option<Arc<McpClient>>,
    config: Arc<Config>,
    bot_id: Mutex<Option<serenity::model::id::UserId>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _: SerenityContext, ready: Ready) {
        info!("{} is connected to Discord!", ready.user.name);
        *self.bot_id.lock().await = Some(ready.user.id);
    }

    async fn message(&self, ctx: SerenityContext, msg: Message) {
        // Ignore messages from self
        let bot_id = *self.bot_id.lock().await;
        if Some(msg.author.id) == bot_id {
            return;
        }

        // Ignore messages from other bots
        if msg.author.bot {
            return;
        }

        let content = msg.content.clone();
        let author = msg.author.name.clone();
        let channel_id = msg.channel_id;
        let room_id = channel_id.to_string(); // Use Channel ID as Room ID

        info!(user = %author, channel = %room_id, "Received Discord message");

        // Simple ping check for connectivity testing
        if content == "!ping" {
            if let Err(e) = channel_id.say(&ctx.http, "Pong from Ultraclaw!").await {
                error!("Error sending ping response: {:?}", e);
            }
            return;
        }

        // --- BRAIN LOOP START ---
        
        let platform = Platform::Discord;

        // 1. Ensure Session Exists
        {
            let mut sessions = self.sessions.lock().await;
            sessions.get_or_create(&room_id, platform);
        }

        // 2. Store User Message
        {
            let db = self.db.lock().await;
            if let Err(e) = db.append_message(&room_id, "user", &content) {
                error!("Failed to store user message: {}", e);
            }
        }

        // 3. Load Context (Short-term Conversation History)
        let context = {
            let db = self.db.lock().await;
            db.get_context(&room_id, self.config.context_window_size).unwrap_or_default()
        };

        // 4. Recall Long-term Memory
        let memory_context = {
            let mem = self.memory.lock().await;
            mem.summarize_for_context(&room_id, 200).unwrap_or(None)
        };

        // 5. Build System Prompt (Persona + Session State)
        let session_context = {
            let sessions = self.sessions.lock().await;
            sessions.get_session_context(&room_id)
        };

        let system_msg = self.soul.build_system_message(
            Some("User is on Discord. You can use markdown, code blocks, and emojis."),
            session_context.as_deref(),
            memory_context.as_deref(),
        );

        let mut messages = Vec::new();
        messages.push(ChatMessage { role: "system".to_string(), content: system_msg });
        messages.extend(context);

        // 6. Inference (Think) with streaming
        let tool_schema = self.skills.to_tool_schema();
        let _ = channel_id.broadcast_typing(&ctx.http).await;
        
        let mut stream = self.engine.infer_stream(messages.clone(), Some(tool_schema), self.soul.temperature, self.soul.max_tokens);
        
        let mut response = String::new();
        let mut streamed_response = String::new();
        let mut edit_message_id: Option<serenity::model::id::MessageId> = None;
        
        loop {
            match stream.next().await {
                Some(Ok(token)) => {
                    response.push_str(&token);
                    streamed_response.push_str(&token);
                    
                    if streamed_response.len() > 100 || streamed_response.contains('\n') {
                        if let Some(msg_id) = edit_message_id {
                            let edit = serenity::builder::EditMessage::new().content(&streamed_response);
                            let _ = channel_id.edit_message(&ctx.http, msg_id, edit).await;
                        } else {
                            match channel_id.say(&ctx.http, &streamed_response).await {
                                Ok(sent) => { edit_message_id = Some(sent.id); }
                                Err(_) => {}
                            }
                        }
                        streamed_response.clear();
                    }
                }
                Some(Err(e)) => {
                    error!("Inference error: {}", e);
                    let _ = channel_id.say(&ctx.http, "I'm having trouble thinking right now.").await;
                    return;
                }
                None => {
                    break;
                }
            }
        }
        
        if !streamed_response.is_empty() {
            if let Some(msg_id) = edit_message_id {
                let edit = serenity::builder::EditMessage::new().content(&streamed_response);
                let _ = channel_id.edit_message(&ctx.http, msg_id, edit).await;
            } else {
                let _ = channel_id.say(&ctx.http, &streamed_response).await;
            }
        } else if let Some(msg_id) = edit_message_id {
            let edit = serenity::builder::EditMessage::new().content(&response);
            let _ = channel_id.edit_message(&ctx.http, msg_id, edit).await;
        }

        // 7. Tool Execution (Act)
        let tool_calls = tools::parse_tool_calls(&response);
        let final_response = if !tool_calls.is_empty() {
             info!("Executing {} tool(s)...", tool_calls.len());
             let _ = channel_id.broadcast_typing(&ctx.http).await;

             let tool_output = tools::execute_tool_calls(&tool_calls, &self.skills, self.mcp.as_deref()).await;
             
             messages.push(ChatMessage { role: "assistant".to_string(), content: response.clone() });
             messages.push(ChatMessage { role: "system".to_string(), content: format!("Tool execution results:\n{}", tool_output) });
             
             let mut re_stream = self.engine.infer_stream(messages.clone(), None, self.soul.temperature, self.soul.max_tokens);
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
                     None => {
                         break;
                     }
                 }
             }
             
             if !re_response.is_empty() {
                 re_response
             } else {
                 response
             }
        } else {
             response
        };

        // 8. Store and Send Response
        {
             let db = self.db.lock().await;
             if let Err(e) = db.append_message(&room_id, "assistant", &final_response) {
                 error!("Failed to store assistant response: {}", e);
             }
        }

        if let Err(e) = channel_id.say(&ctx.http, &final_response).await {
            error!("Error sending response to Discord: {:?}", e);
        }
    }
}
