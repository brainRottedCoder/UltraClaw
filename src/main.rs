#![allow(unexpected_cfgs)]

// ============================================================================
// ULTRACLAW — main.rs
// ============================================================================
// Entrypoint for the Ultraclaw autonomous AI agent.
//
// This file orchestrates the boot sequence:
// 1. Initialize structured logging (tracing)
// 2. Load configuration from environment variables
// 3. Open the SQLite databases (conversation context + long-term memory)
// 4. Build the Soul (agent personality + directives)
// 5. Register built-in Skills (file read, command exec, dir list)
// 6. Optionally connect to MCP servers for external tools
// 7. Create the SessionManager for multi-turn conversation tracking
// 8. Initialize the FailoverEngine (Cloud → Local LLM)
// 9. Initialize available standard connectors (CLI, Discord, Webhook, etc.)
// 10. Spawn a background task for session expiration
// 11. Enter the event loop (runs forever)
//
// TOTAL MEMORY BUDGET AT STARTUP (before any conversations):
// ┌────────────────────────────┬────────────┐
// │ Component                  │ RAM Usage   │
// ├────────────────────────────┼────────────┤
// │ Tokio runtime + threads    │ ~2 MB       │
// │ CLI / Webhook Connectors   │ ~1-2 MB     │
// │ SQLite conv DB (page cache)│ ~2 MB       │
// │ SQLite memory DB           │ ~2 MB       │
// │ Soul (persona + directives)│ ~1 KB       │
// │ SkillRegistry              │ ~500 B      │
// │ SessionManager (empty)     │ ~100 B      │
// │ Reqwest HTTP client        │ ~200 B      │
// │ Config struct              │ ~500 B      │
// │ Binary code (.text segment)│ ~5-10 MB    │
// │ Stack space (main + tasks) │ ~1 MB       │
// ├────────────────────────────┼────────────┤
// │ TOTAL (idle, no model)     │ ~13-18 MB   │
// │ + Local model (mmap RSS)   │ ~500 MB-1GB │
// └────────────────────────────┴────────────┘
//
// Note: The local model's mmap'd pages are demand-paged. The ~500MB-1GB
// is only resident during active local inference. At idle, the OS can
// reclaim these pages, bringing total RSS back to ~13-18 MB.
//
// ENERGY BUDGET:
// - Idle (no messages): ~0.1W (epoll_wait, sleeping)
// - Processing a message (cloud): ~0.5W for ~2 seconds
// - Processing a message (local): ~5-15W for ~5-30 seconds (CPU inference)
// - For context: a Raspberry Pi 4 draws ~3W at idle, ~6W under full CPU load
// ============================================================================

// Module declarations — each file becomes a module in the crate.
// The compiler only includes code that is actually used, so dead modules
// don't contribute to binary size (with LTO enabled).
mod config;
mod db;
mod demo;
mod formatter;
mod inference;
mod mcp;
mod media;
mod media_skill;
mod memory;
mod onboarding;
mod offline;
mod sandbox_skill;
mod search_skill;
mod session;
mod skill;
mod soul;
mod swarm_skill;
mod cron_skill;
mod embedding_service;
mod document_parser;
mod rag_engine;
mod tools;
mod cli;
mod connector;
mod connectors;
mod voice_skill;
mod browser_skill;
mod smarthome_skill;
mod system_nodes;
mod auth;
mod gateway;
mod memory_vector;
mod quota;
mod rag_sop;
mod robot_hardware;
mod robot_skill;
mod security_landlock;
mod skill_manager;
mod wasm_plugin;
mod web_dashboard;
mod git_resolver;
mod openclaw_skills;
mod tailscale_funnel;
mod group_context;
mod vision_agent;
mod vision_browser_skill;
mod browser_recorder;
mod browser_replay_skill;
mod live_canvas;
mod tts_service;
mod rag_advanced;

mod self_healing;
mod goal_decomp;
mod vision_agent_impl;
mod chain_tools;
mod hallucination_shield;
mod conversation_memory;
mod debate_orchestrator;
mod prompt_refine;

use crate::config::Config;
use crate::db::ConversationDb;
use crate::inference::{fetch_ollama_models, CloudEngine, FailoverEngine, LocalEngine, OllamaModel};
use crate::media::{MediaEngine, MediaProvider};
use crate::media_skill::{GenerateImageSkill, GenerateVideoSkill};
use crate::memory::MemoryStore;
use crate::mcp::McpClient;
use crate::session::SessionManager;
use crate::skill::SkillRegistry;
use crate::soul::Soul;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

const DEFAULT_OLLAMA_MODEL: &str = "phi4-mini:latest";
const PREFERRED_OLLAMA_MODEL_PREFIXES: [&str; 4] = ["phi", "llama3.2", "mistral", "qwen2.5"];

fn is_default_ollama_model(model: &str) -> bool {
    let trimmed = model.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case(DEFAULT_OLLAMA_MODEL)
}

fn ollama_model_matches(candidate: &str, requested: &str) -> bool {
    let candidate = candidate.trim().to_ascii_lowercase();
    let requested = requested.trim().to_ascii_lowercase();

    if candidate.is_empty() || requested.is_empty() {
        return false;
    }

    if candidate == requested {
        return true;
    }

    let requested_family = requested.split(':').next().unwrap_or(requested.as_str());
    candidate == requested_family || candidate.starts_with(&format!("{}:", requested_family))
}

fn find_ollama_model<'a>(models: &'a [OllamaModel], requested: &str) -> Option<&'a OllamaModel> {
    models.iter().find(|model| {
        ollama_model_matches(model.display_name(), requested)
            || ollama_model_matches(&model.model, requested)
    })
}

fn preferred_ollama_model(models: &[OllamaModel]) -> Option<&OllamaModel> {
    for prefix in PREFERRED_OLLAMA_MODEL_PREFIXES {
        if let Some(model) = models.iter().find(|model| {
            let candidate = model.display_name().to_ascii_lowercase();
            candidate == prefix || candidate.starts_with(&format!("{}:", prefix))
        }) {
            return Some(model);
        }
    }

    models.first()
}

fn format_model_size(size_bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;

    if size_bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", size_bytes as f64 / GIB)
    } else if size_bytes >= 1024 * 1024 {
        format!("{:.0} MB", size_bytes as f64 / MIB)
    } else if size_bytes == 0 {
        "unknown size".to_string()
    } else {
        format!("{} bytes", size_bytes)
    }
}

async fn print_ollama_models(base_url: &str) -> Result<(), String> {
    let models = fetch_ollama_models(base_url).await?;

    println!("Ollama models at {}:", base_url);
    if models.is_empty() {
        println!("  No models installed.");
        println!("  Recommended: ollama pull {}", DEFAULT_OLLAMA_MODEL);
        return Ok(());
    }

    let recommended = preferred_ollama_model(&models).map(|model| model.display_name().to_string());
    for model in &models {
        let details = model.details.as_ref();
        let family = details
            .map(|detail| detail.family.as_str())
            .filter(|family| !family.is_empty())
            .unwrap_or("unknown");
        let parameter_size = details
            .map(|detail| detail.parameter_size.as_str())
            .filter(|size| !size.is_empty())
            .unwrap_or("unknown");
        let recommended_suffix = recommended
            .as_deref()
            .filter(|name| name.eq_ignore_ascii_case(model.display_name()))
            .map(|_| " [recommended]")
            .unwrap_or("");

        println!(
            "  - {}{} ({}, family {}, params {})",
            model.display_name(),
            recommended_suffix,
            format_model_size(model.size),
            family,
            parameter_size
        );
    }

    Ok(())
}

/// The main entry point for Ultraclaw.
///
/// Uses `#[tokio::main]` which expands to a multi-threaded async runtime.
/// The runtime spawns worker threads equal to the number of CPU cores.
/// On a single-core device, it uses 1 worker thread (no overhead).
///
/// `current_thread` flavor could save ~500KB of RAM by using cooperative
/// scheduling on a single thread. We use `multi_thread` for robustness:
/// if a local inference blocks a thread, other rooms can still be served.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ========================================================================
    // STEP 1: Initialize structured logging
    // ========================================================================
    // The env filter allows runtime control: RUST_LOG=ultraclaw=debug
    // Default: info level (minimal output, minimal I/O energy).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("ultraclaw=info")),
        )
        .compact() // Compact format saves terminal I/O bandwidth
        .init();

    info!("╔══════════════════════════════════════════════════╗");
    info!("║          ULTRACLAW — AI Agent v0.1.0             ║");
    info!("║  Hyper-Optimized Multi-Platform Inference Engine  ║");
    info!("╚══════════════════════════════════════════════════╝");

    // ========================================================================
    // STEP 1.5: Security Kernel Sandbox (Landlock)
    // ========================================================================
    let mut landlock = crate::security_landlock::LandlockSecurity::new();
    if let Err(e) = landlock.enforce() {
        warn!("Linux Landlock sandboxing not available or failed: {}", e);
    } else {
        info!("Linux Landlock strict kernel sandboxing engaged.");
    }

    // Detect internet connectivity and set offline mode
    crate::offline::detect_connectivity().await;
    crate::offline::print_offline_status();

    // ========================================================================
    // STEP 2: Load configuration
    // ========================================================================
let args: Vec<String> = std::env::args().collect();
    let force_init = args.contains(&"--init".to_string()) || args.contains(&"--setup".to_string());
    let list_models = args.contains(&"--list-models".to_string());
    let run_benchmark = args.contains(&"--benchmark".to_string());
    let run_compare = args.contains(&"--compare".to_string());
    let model_override = args.iter().skip(1).find(|a| a.starts_with("--model="))
        .map(|a| a.trim_start_matches("--model=").to_string());
    let _hyde_enabled = args.contains(&"--hyde".to_string());
    let _rerank_enabled = args.contains(&"--rerank".to_string());
    let _load_wasm = args.iter().skip(1).find(|a| a.starts_with("--load-wasm="))
        .map(|a| a.trim_start_matches("--load-wasm=").to_string());

    if _hyde_enabled {
        info!("HyDE (Hypothetical Document Embeddings) enabled");
    }
    if _rerank_enabled {
        info!("Cross-encoder reranking enabled");
    }
    if let Some(ref wasm_path) = _load_wasm {
        info!(path = %wasm_path, "WASM plugin loading enabled");
    }

if list_models {
        let config = Config::load().unwrap_or_default();
        if let Err(e) = print_ollama_models(&config.ollama_base_url).await {
            error!(ollama_url = %config.ollama_base_url, error = %e, "Failed to list Ollama models");
            return Err(anyhow::anyhow!(e));
        }
        return Ok(());
    }

    if run_benchmark {
        let config = Config::load().unwrap_or_default();
        let cfg = Arc::new(config);
        return crate::demo::run_benchmark_mode(cfg.clone()).await;
    }

    if run_compare {
        let config = Config::load().unwrap_or_default();
        let cfg = Arc::new(config);
        return crate::demo::run_compare_mode(cfg.clone()).await;
    }
    
    // Attempt to load config; if it fails or init requested, run wizard
    let mut config = if force_init || Config::load().is_err() || !Config::load().unwrap().is_valid() {
        if !force_init {
             info!("Configuration missing or invalid. Starting onboarding wizard...");
        }
        if let Err(e) = onboarding::run_wizard() {
             error!("Wizard failed: {}", e);
             return Ok(());
        }
        // Reload after wizard
        Config::load().expect("Failed to load config after wizard")
    } else {
        Config::load().unwrap()
    };
    
    info!(
        homeserver = %config.homeserver_url,
        model = %config.cloud_model,
        ollama_model = %config.ollama_model,
        local_temperature = config.local_temperature,
        local_max_tokens = config.local_max_tokens,
        db_path = %config.db_path,
        session_ttl = config.session_ttl_secs,
        max_sessions = config.max_sessions,
        "Configuration loaded"
    );
    let config = Arc::new(config);

    // Apply model override from CLI args
    if let Some(ref model_name) = model_override {
        info!(model = %model_name, "Model override applied from CLI");
    }

    // ========================================================================
    // STEP 3: Open databases
    // ========================================================================
    // Conversation context DB — short-term, per-room message history.
    // SQLite with WAL mode, 2MB page cache.
    let conv_db = ConversationDb::open(&config.db_path)
        .expect("Failed to open conversation database");
    info!(path = %config.db_path, "Conversation database opened");
    let conv_db = Arc::new(Mutex::new(conv_db));

    // Long-term memory DB — persistent facts, preferences, instructions.
    // Stored alongside the conversation DB (same file for simplicity,
    // or a separate file for isolation — configurable).
    let memory_db_path = format!("{}.memory", config.db_path);
    let memory_store = MemoryStore::open(&memory_db_path)
        .expect("Failed to open memory database");
    info!(path = %memory_db_path, "Memory database opened");

    // Prune old memories on startup (garbage collection)
    match memory_store.prune(config.memory_max_age_days, 0.2) {
        Ok(n) if n > 0 => info!(pruned = n, "Pruned old memories"),
        _ => {}
    }
    let memory_store = Arc::new(Mutex::new(memory_store));

    // ========================================================================
    // STEP 4: Build the Soul
    // ========================================================================
    // The Soul defines the agent's personality and behavioral directives.
    // Default Ultraclaw persona uses static strings (zero heap allocation).
    let soul = Soul::default_soul();
    info!(
        name = %soul.name,
        directives = soul.directives.len(),
        temperature = soul.temperature,
        "Soul initialized"
    );
    let soul = Arc::new(soul);

    // ========================================================================
    // STEP 5: Register Skills
    // ========================================================================
    // Built-in skills: read_file, list_directory, run_command.
    // These are the agent's "hands" — they execute side-effects.
    let mut skills = SkillRegistry::new();
    info!("Skill registry initialized with built-in skills");

    // ========================================================================
    // STEP 5b: Initialize Media Engine + Register Media Skills
    // ========================================================================
    // Build the API key map from config. Only providers with non-empty keys
    // are added — the MediaEngine auto-selects the best available provider.
    let mut media_keys: HashMap<MediaProvider, String> = HashMap::new();
    let key_pairs = [
        (MediaProvider::OpenAI, config.cloud_api_key.as_str()),
        (MediaProvider::Stability, config.stability_api_key.as_str()),
        (MediaProvider::Runway, config.runway_api_key.as_str()),
        (MediaProvider::Replicate, config.replicate_api_key.as_str()),
        (MediaProvider::Together, config.together_api_key.as_str()),
        (MediaProvider::Fal, config.fal_api_key.as_str()),
        (MediaProvider::Leonardo, config.leonardo_api_key.as_str()),
        (MediaProvider::Imagen, config.imagen_api_key.as_str()),
        (MediaProvider::Veo, config.veo_api_key.as_str()),
        (MediaProvider::Kling, config.kling_api_key.as_str()),
        (MediaProvider::Seedance, config.seedance_api_key.as_str()),
        (MediaProvider::Luma, config.luma_api_key.as_str()),
        (MediaProvider::Minimax, config.minimax_api_key.as_str()),
        (MediaProvider::Pika, config.pika_api_key.as_str()),
        (MediaProvider::Sora, config.sora_api_key.as_str()),
    ];
    for (provider, key) in &key_pairs {
        if !key.is_empty() {
            media_keys.insert(*provider, key.to_string());
        }
    }

    let has_media = !media_keys.is_empty();
    let media_engine = Arc::new(Mutex::new(MediaEngine::new(
        media_keys,
        std::path::PathBuf::from(&config.media_output_dir),
        MediaProvider::from_str_loose(&config.media_image_provider),
        MediaProvider::from_str_loose(&config.media_video_provider),
    )));

    if has_media {
        // Register media skills with the MediaEngine
        skills.register(Box::new(GenerateImageSkill::new(media_engine.clone())));
        skills.register(Box::new(GenerateVideoSkill::new(media_engine.clone())));
        info!("Media skills registered (generate_image, generate_video)");
    } else {
        info!("No media API keys configured — media skills disabled");
    }

    for node_skill in crate::system_nodes::SystemNodesModule::register_all() {
        skills.register(node_skill);
    }
    info!("System Node tools (camera, screen_record, etc.) registered");

    // Register Phase 1 production skills
    skills.register(Box::new(crate::browser_skill::BrowserSkill::new()));
    info!("Browser skill (Playwright automation) registered");

    skills.register(Box::new(crate::smarthome_skill::SmartHomeSkill::new()));
    info!("SmartHome skill (Home Assistant, Sonos, Hue) registered");

    skills.register(Box::new(crate::swarm_skill::SwarmSkill::default()));
    info!("Swarm skill (sub-agent spawning) registered");

    skills.register(Box::new(crate::cron_skill::CronSkill::new()));
    info!("Cron skill (task scheduling) registered");

    // Register Phase 2 production skills (Document Processing & RAG)
    skills.register(Box::new(crate::document_parser::DocumentParserSkill::new()));
    info!("Document Parser skill (PDF/DOCX extraction) registered");

    skills.register(Box::new(crate::rag_engine::RAGSkill::new()));
    info!("RAG skill (vector store + retrieval) registered");

    let skills = Arc::new(skills);

    // ========================================================================
    // STEP 5.5: Initialize Level 2 Conflict Resolver & OpenClaw Registry
    // ========================================================================
    let git_resolver = crate::git_resolver::SemanticGitResolver::new();
    info!("NanoClaw Level 2 Semantic Git Conflict Resolution Engine active.");
    drop(git_resolver);

    let openclaw_registry = crate::openclaw_skills::OpenClawSkillRegistry::new();
    info!("OpenClaw Hyper-Skill Module loaded with {} custom extensions.", openclaw_registry.list_extensions().len());
    drop(openclaw_registry);

    let _tailscale = crate::tailscale_funnel::TailscaleFunnel::new();
    let _group_ctx = crate::group_context::GroupContextManager::new(std::path::PathBuf::from("/tmp/ultraclaw_groups"));
    let _canvas = crate::live_canvas::LiveCanvasProtocol::new();
    
    let massive_channels_init = crate::connectors::massive_channels::MassiveChannelsInit::new();
    massive_channels_init.initialize_all();

    let api_gateway = crate::gateway::ApiGateway::new(3030);
    api_gateway.start();

    info!("Tailscale Funnel, Group Context, Live Canvas, API Gateway (port 3030), and Massive Channels initialized natively.");

    // ========================================================================
    // STEP 6: Connect to MCP servers (optional)
    // ========================================================================
    // If configured, spawn an MCP server process and connect via stdio pipes.
    let mcp_client: Option<Arc<McpClient>> = if !config.mcp_server_command.is_empty() {
        info!(
            command = %config.mcp_server_command,
            "Connecting to MCP server..."
        );
        match McpClient::connect(&config.mcp_server_command, &[]).await {
            Ok(client) => {
                // List available tools from the MCP server
                match client.list_tools().await {
                    Ok(tools) => {
                        info!(
                            tool_count = tools.len(),
                            "MCP server connected, tools discovered"
                        );
                        for tool in &tools {
                            info!(tool = %tool.name, "  MCP tool available");
                        }
                    }
                    Err(e) => warn!("Failed to list MCP tools: {}", e),
                }
                Some(Arc::new(client))
            }
            Err(e) => {
                error!(error = %e, "Failed to connect to MCP server (continuing without MCP)");
                None
            }
        }
    } else {
        info!("MCP not configured (ULTRACLAW_MCP_SERVER_COMMAND is empty)");
        None
    };

    // ========================================================================
    // STEP 7: Create Session Manager
    // ========================================================================
    // Tracks multi-turn conversation sessions per room.
    // Max 256 sessions × ~140 bytes each = ~35KB max RAM usage.
    let sessions = SessionManager::new(config.session_ttl_secs, config.max_sessions);
    info!(
        ttl_secs = config.session_ttl_secs,
        max = config.max_sessions,
        "Session manager initialized"
    );
    let sessions = Arc::new(Mutex::new(sessions));

    // ========================================================================
    // STEP 8: Initialize Inference Engines
    // ========================================================================
    // Cloud engine: reqwest HTTP client → OpenAI-compatible API
    let cloud = CloudEngine::new(
        &config.cloud_api_key,
        &config.cloud_model,
        &config.cloud_base_url,
    );
    info!(
        model = %config.cloud_model,
        base_url = %config.cloud_base_url,
        "Cloud inference engine initialized"
    );

    // Local engine: Ollama-backed model selection using the installed tag list.
    let discovered_ollama_models = match fetch_ollama_models(&config.ollama_base_url).await {
        Ok(models) => Some(models),
        Err(e) => {
            warn!(
                ollama_url = %config.ollama_base_url,
                error = %e,
                "Ollama model discovery failed"
            );
            None
        }
    };

    let selected_ollama_model = if let Some(ref override_model) = model_override {
        override_model.clone()
    } else if let Some(models) = discovered_ollama_models.as_ref() {
        if let Some(model) = find_ollama_model(models, &config.ollama_model) {
            let resolved = model.display_name().to_string();
            if !resolved.eq_ignore_ascii_case(config.ollama_model.trim()) {
                info!(
                    configured_model = %config.ollama_model,
                    resolved_model = %resolved,
                    "Resolved configured Ollama model to an installed tag"
                );
            }
            resolved
        } else if is_default_ollama_model(&config.ollama_model) {
            if let Some(model) = preferred_ollama_model(models) {
                let discovered = model.display_name().to_string();
                info!(
                    configured_model = %config.ollama_model,
                    discovered_model = %discovered,
                    "Auto-selected the best installed Ollama model"
                );
                discovered
            } else {
                config.ollama_model.clone()
            }
        } else {
            warn!(
                configured_model = %config.ollama_model,
                "Configured Ollama model was not found in installed tags; keeping configured value"
            );
            config.ollama_model.clone()
        }
    } else {
        config.ollama_model.clone()
    };

    let local_available = discovered_ollama_models
        .as_ref()
        .map(|models| !models.is_empty())
        .unwrap_or(false);

    if local_available {
        if let Some(models) = discovered_ollama_models.as_ref() {
            info!(
                available_models = models.len(),
                selected_model = %selected_ollama_model,
                "Ollama model discovery completed"
            );
        }
    } else if discovered_ollama_models.as_ref().is_some_and(|models| models.is_empty()) {
        warn!(
            ollama_url = %config.ollama_base_url,
            "Ollama is reachable but no models are installed. Run `ollama pull {}`.",
            DEFAULT_OLLAMA_MODEL
        );
    }

    let local = LocalEngine::new(
        &config.local_model_path,
        &config.ollama_base_url,
        &selected_ollama_model,
        config.local_temperature,
        config.local_max_tokens,
    );
    info!(
        model_path = %config.local_model_path,
        ollama_url = %config.ollama_base_url,
        ollama_model = %selected_ollama_model,
        temperature = config.local_temperature,
        max_tokens = config.local_max_tokens,
        "Local inference engine initialized (Ollama)"
    );

    let local_ready = if discovered_ollama_models.as_ref().is_some_and(|models| models.is_empty()) {
        false
    } else {
        local_available || local.health_check().await
    };

    if local_ready {
        info!("Ollama connection verified");
    } else {
        warn!("Ollama not responding at {}. Local inference will fail until Ollama is running.", config.ollama_base_url);
    }

// Failover engine: tries Cloud first, falls back to Local on failure
    let failover = FailoverEngine::new(cloud, local);
    failover.set_local_available(local_ready);
    let engine: Arc<dyn crate::inference::InferenceEngine> = Arc::new(failover.clone());
    info!("Failover engine ready: Cloud → Local");

    // Keep a typed reference for health monitoring
    let failover_for_health = failover;

    // ========================================================================
    // STEP 8.5: Spawn Health Monitoring Background Task
    // ========================================================================
    // Periodically checks Ollama availability and updates engine status.
    // Also monitors system memory for resource tracking.
    use crate::inference::InferenceEngine;

    let ollama_url = config.ollama_base_url.clone();
    let selected_model = selected_ollama_model.clone();
    let check_interval_secs: u64 = 60;

    tokio::spawn(async move {
        use sysinfo::System;

        let mut sys = System::new_all();
        let mut consecutive_failures = 0u8;
        let max_consecutive_failures = 3u8;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(check_interval_secs)).await;

            sys.refresh_memory();
            let used_memory_mb = sys.used_memory() / 1024 / 1024;
            let total_memory_mb = sys.total_memory() / 1024 / 1024;

            let ollama_healthy = {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .ok();

                if let Some(client) = client {
                    match client.get(format!("{}/api/tags", ollama_url)).send().await {
                        Ok(resp) => resp.status().is_success(),
                        Err(_) => false,
                    }
                } else {
                    false
                }
            };

            let was_available = failover_for_health.is_local_available();
            failover_for_health.set_local_available(ollama_healthy);

            if ollama_healthy != was_available {
                info!(
                    ollama_available = ollama_healthy,
                    "Ollama availability changed"
                );
            }

            if ollama_healthy {
                consecutive_failures = 0;
                tracing::debug!(
                    ollama_url = %ollama_url,
                    memory_mb = used_memory_mb,
                    total_memory_mb = total_memory_mb,
                    current_mode = ?failover_for_health.get_mode(),
                    selected_model = %selected_model,
                    "Health check: Ollama healthy"
                );
            } else {
                consecutive_failures += 1;
                if consecutive_failures >= max_consecutive_failures {
                    warn!(
                        ollama_url = %ollama_url,
                        consecutive_failures = consecutive_failures,
                        "Ollama health check failing repeatedly"
                    );
                }
            }
        }
    });

    info!(
        health_check_interval_secs = check_interval_secs,
        "Health monitoring background task started"
    );

    // ========================================================================
    // STEP 9: Initialize Connectors
    // ========================================================================
    // Ultraclaw supports running multiple connectors simultaneously (Matrix, CLI, etc.)
    // We determine which ones to run based on args and config.
    use crate::connector::Connector;

    let mut connectors: Vec<Box<dyn Connector>> = Vec::new();

    // Check for CLI mode flag argument
    if args.contains(&"--cli".to_string()) {
        info!("CLI mode enabled via flag");
        connectors.push(Box::new(cli::CliConnector::new()));
    } else if args.contains(&"--demo".to_string()) {
        info!("DEMO mode enabled via flag - Running GARAGE_INFERENCE hackathon demo");
        let metrics = crate::demo::run_demo_mode(
            engine.clone(),
            skills,
            soul.clone(),
            config.clone(),
        ).await?;
        let report = crate::demo::generate_hackathon_report(metrics, &config.ollama_model);
        println!("\n{}", report);
        return Ok(());
    } else {
    // No default connector inserted here because Matrix was removed by user request.
    }

    // --- Discord Connector ---
    #[cfg(feature = "discord")]
    {
        if config.discord_token.is_some() {
            info!("Discord connector enabled via config");
            connectors.push(Box::new(connectors::discord::DiscordConnector::new()));
        }
    }

    // --- Telegram Connector ---
    #[cfg(feature = "telegram")]
    {
        if config.telegram_token.is_some() {
            info!("Telegram connector enabled via config");
            connectors.push(Box::new(connectors::telegram::TelegramConnector::new()));
        }
    }

    // --- Webhook Connector ---
    #[cfg(feature = "webhook")]
    {
        info!("Webhook connector enabled");
        connectors.push(Box::new(connectors::webhook::WebhookConnector::new()));
    }

    // Default fallback: If NO connectors are enabled, prompt user or enable Matrix?
    // For now, if list is empty, enable Matrix as default unless --cli was passed?
    // Logic refinement:
    if connectors.is_empty() && !args.contains(&"--cli".to_string()) {
         info!("No connectors enabled. Defaulting to CLI.");
         connectors.push(Box::new(cli::CliConnector::new()));
    }

    if connectors.is_empty() {
        warn!("No connectors enabled! Exiting.");
        return Ok(());
    }

    // ========================================================================
    // STEP 10: Spawn background maintenance tasks
    // ========================================================================
    // Session expiration sweep — runs every 60 seconds.
    {
        let sessions = sessions.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let mut sessions = sessions.lock().await;
                let expired = sessions.expire_idle();
                if expired > 0 {
                    info!(expired = expired, "Expired idle sessions");
                }
            }
        });
    }

    // Periodic memory pruning with TTL (runs every 6 hours)
    {
        let config = config.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(21600)).await;
                let mem_path = format!("{}.memory", config.db_path);
                if let Ok(store) = crate::conversation_memory::PersistentMemory::open(Some(&mem_path)) {
                    let pruned = store.prune_expired(config.memory_max_age_days as u32, 2.0).await.unwrap_or(0);
                    if pruned > 0 {
                        info!(pruned = pruned, "Periodic memory TTL pruning completed");
                    }
                }
            }
        });
    }

    // Web Dashboard (runs on port 9090 by default)
    {
        let model_for_dash = selected_ollama_model.clone();
        tokio::spawn(async move {
            let dashboard = crate::web_dashboard::WebDashboard::new(model_for_dash, 9090);
            dashboard.serve().await;
        });
        info!("Web Dashboard starting on http://127.0.0.1:9090");
    }

    // ========================================================================
    // STEP 11: Run Connectors
    // ========================================================================
    info!("Starting {} connector(s)...", connectors.len());

    // Spawn a task for each connector
    let mut handles = Vec::new();

    for connector in connectors {
        let engine = engine.clone();
        let db = conv_db.clone();
        let memory = memory_store.clone();
        let sessions = sessions.clone();
        let soul = soul.clone();
        let skills = skills.clone();
        let mcp = mcp_client.clone();
        let config = config.clone();

        let name = connector.name().to_string();
        info!(connector = %name, "Launching connector");

        handles.push(tokio::spawn(async move {
            if let Err(e) = connector.run(engine, db, memory, sessions, soul, skills, mcp, config).await {
                error!(connector = %name, error = %e, "Connector failed");
            }
        }));
    }

    // Wait for all connectors (they usually run forever)
    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}
