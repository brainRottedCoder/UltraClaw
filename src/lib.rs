// ============================================================================
// ULTRACLAW — lib.rs
// ============================================================================
// Library interface for Ultraclaw components.
//
// This module exports the public API for integration testing and external
// consumption. Only the inference engine and core data structures are
// exported to keep the library surface minimal.
// ============================================================================

pub mod auth;
pub mod browser_skill;
pub mod cli;
pub mod config;
pub mod connector;
pub mod connectors;
pub mod cron_skill;
pub mod db;
pub mod document_parser;
pub mod embedding_service;
pub mod eval;
pub mod formatter;
pub mod gateway;
pub mod gemma_adapter;
pub mod git_resolver;
pub mod group_context;
pub mod inference;
pub mod live_canvas;
pub mod mcp;
pub mod media;
pub mod media_skill;
pub mod memory;
pub mod memory_vector;
pub mod onboarding;
pub mod offline;
pub mod openclaw_skills;
pub mod quota;
pub mod rag_advanced;
pub mod rag_engine;
pub mod rag_sop;
pub mod robot_hardware;
pub mod robot_skill;
pub mod sandbox_skill;
pub mod search_skill;
pub mod security_landlock;
pub mod session;
pub mod skill;
pub mod skill_manager;
pub mod smarthome_skill;
pub mod soul;
pub mod swarm_executor;
pub mod swarm_skill;
pub mod system_nodes;
pub mod tailscale_funnel;
pub mod tools;
pub mod tui;
pub mod tts_service;
pub mod voice_skill;
pub mod vision_agent;
pub mod vision_browser_skill;
pub mod wasm_plugin;
pub mod web_dashboard;
pub mod browser_recorder;
pub mod browser_replay_skill;

pub mod self_healing;
pub mod goal_decomp;
pub mod vision_agent_impl;
pub mod chain_tools;
pub mod hallucination_shield;
pub mod conversation_memory;
pub mod debate_orchestrator;
pub mod prompt_refine;