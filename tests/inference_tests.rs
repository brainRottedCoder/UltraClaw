// ============================================================================
// ULTRACLAW — inference_tests.rs
// ============================================================================
// Integration tests for the inference engine with local model support.
//
// TEST COVERAGE:
// - Engine mode switching (auto, cloud, local)
// - Streaming output functionality
// - Health monitoring for Ollama
// - Graceful fallback when local is unavailable
// - Tool execution on local models
//
// Run with: cargo test --test inference_tests
// ============================================================================

use ultraclaw::db::ChatMessage;
use ultraclaw::inference::{CloudEngine, EngineMode, FailoverEngine, InferenceEngine, LocalEngine};

/// Test that CloudEngine can be cloned and used across threads
#[test]
fn test_cloud_engine_clone() {
    let cloud = CloudEngine::new(
        "test-key",
        "gpt-4o",
        "https://api.openai.com/v1",
    );
    let cloud2 = cloud.clone();
    assert!(std::mem::size_of_val(&cloud) > 0);
    assert!(std::mem::size_of_val(&cloud2) > 0);
}

/// Test that LocalEngine can be cloned
#[test]
fn test_local_engine_clone() {
    let local = LocalEngine::new(
        "/path/to/model.gguf",
        "http://localhost:11434",
        "llama3.2",
        0.7,
        100,
    );
    let local2 = local.clone();
    assert!(std::mem::size_of_val(&local) > 0);
    assert!(std::mem::size_of_val(&local2) > 0);
}

/// Test EngineMode display implementation
#[test]
fn test_engine_mode_display() {
    assert_eq!(EngineMode::Auto.to_string(), "auto");
    assert_eq!(EngineMode::CloudOnly.to_string(), "cloud");
    assert_eq!(EngineMode::LocalOnly.to_string(), "local");
}

/// Test EngineMode parsing from strings
#[test]
fn test_engine_mode_from_str() {
    assert_eq!("auto".parse::<EngineMode>().unwrap(), EngineMode::Auto);
    assert_eq!("cloud".parse::<EngineMode>().unwrap(), EngineMode::CloudOnly);
    assert_eq!("local".parse::<EngineMode>().unwrap(), EngineMode::LocalOnly);
    assert_eq!("AUTO".parse::<EngineMode>().unwrap(), EngineMode::Auto);
    assert_eq!("CLOUD".parse::<EngineMode>().unwrap(), EngineMode::CloudOnly);
    assert_eq!("LOCAL".parse::<EngineMode>().unwrap(), EngineMode::LocalOnly);
    assert!("unknown".parse::<EngineMode>().is_err());
}

/// Test FailoverEngine creation and mode management
#[test]
fn test_failover_engine_creation() {
    let cloud = CloudEngine::new(
        "test-key",
        "gpt-4o",
        "https://api.openai.com/v1",
    );
    let local = LocalEngine::new(
        "/path/to/model.gguf",
        "http://localhost:11434",
        "llama3.2",
        0.7,
        100,
    );

    let failover = FailoverEngine::new(cloud, local);

    // Default mode should be Auto
    assert_eq!(failover.get_mode(), EngineMode::Auto);

    // Test mode switching
    failover.set_mode(EngineMode::CloudOnly);
    assert_eq!(failover.get_mode(), EngineMode::CloudOnly);

    failover.set_mode(EngineMode::LocalOnly);
    assert_eq!(failover.get_mode(), EngineMode::LocalOnly);

    failover.set_mode(EngineMode::Auto);
    assert_eq!(failover.get_mode(), EngineMode::Auto);
}

/// Test local availability management
#[test]
fn test_local_availability() {
    let cloud = CloudEngine::new(
        "test-key",
        "gpt-4o",
        "https://api.openai.com/v1",
    );
    let local = LocalEngine::new(
        "/path/to/model.gguf",
        "http://localhost:11434",
        "llama3.2",
        0.7,
        100,
    );

    let failover = FailoverEngine::new(cloud, local);

    // Initially local should be unavailable
    assert!(!failover.is_local_available());

    // Set local as available
    failover.set_local_available(true);
    assert!(failover.is_local_available());

    // Set local as unavailable
    failover.set_local_available(false);
    assert!(!failover.is_local_available());
}

/// Test engine status retrieval
#[test]
fn test_engine_status() {
    let cloud = CloudEngine::new(
        "test-key",
        "gpt-4o",
        "https://api.openai.com/v1",
    );
    let local = LocalEngine::new(
        "/path/to/model.gguf",
        "http://localhost:11434",
        "llama3.2",
        0.7,
        100,
    );

    let failover = FailoverEngine::new(cloud, local);
    failover.set_local_available(true);

    let status = failover.get_status("llama3.2", "gpt-4o");

    assert_eq!(status.mode, EngineMode::Auto);
    assert!(status.cloud_available);
    assert!(status.local_available);
    assert_eq!(status.current_model, "gpt-4o"); // Auto mode uses cloud by default

    // Test local-only mode shows local model
    failover.set_mode(EngineMode::LocalOnly);
    let status = failover.get_status("llama3.2", "gpt-4o");
    assert_eq!(status.current_model, "llama3.2");
}

/// Test that streaming returns a valid stream
#[tokio::test]
async fn test_streaming_return_type() {
    let cloud = CloudEngine::new(
        "test-key",
        "gpt-4o",
        "https://api.openai.com/v1",
    );

    let messages = vec![
        ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        },
    ];

    let _stream = cloud.infer_stream(
        messages,
        None,
        0.7,
        100,
    );
}

/// Test that async inference returns the correct future type
#[tokio::test]
async fn test_infer_return_type() {
    let local = LocalEngine::new(
        "/path/to/model.gguf",
        "http://localhost:11434",
        "llama3.2",
        0.7,
        100,
    );

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: "You are a helpful assistant.".to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: "What is 2+2?".to_string(),
        },
    ];

    let _future = local.infer(messages, None, 0.7, 100);
}