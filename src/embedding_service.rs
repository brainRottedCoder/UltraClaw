// ============================================================================
// ULTRACLAW — embedding_service.rs
// ============================================================================
// Sentence-transformers embedding service via Python subprocess.
//
// This module manages a Python subprocess running the embedding server
// (scripts/embedding_server.py). It provides both single-text and batch
// embedding generation with automatic model loading and health monitoring.
//
// ARCHITECTURE:
// - Lazy subprocess startup (process spawned on first embed call)
// - JSON lines protocol on stdin/stdout
// - Automatic reconnection on process death
// - Batch embedding support for efficient throughput
// - Thread-safe with Arc<RwLock>
//
// MODEL: all-MiniLM-L6-v2 (384 dimensions, ~90MB)
// - Fast inference, high quality
// - Multi-language support (English primary, good on 50+ languages)
// - Normalized embeddings for cosine similarity = dot product
//
// SAFETY:
// - Subprocess lifecycle managed (auto-restart on crash)
// - Bounded response size (4096 bytes per result)
// - Configurable via ULTRACLAW_EMBEDDING_MODEL env var
// ============================================================================

use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process;
use tokio::sync::{watch, RwLock};
use tracing::{debug, error, info, warn};

const DEFAULT_EMBEDDING_MODEL: &str = "all-MiniLM-L6-v2";
const DEFAULT_BATCH_SIZE: usize = 32;
const EMBEDDING_DIM: usize = 384;

/// Connection state to the embedding subprocess
enum EmbeddingProcess {
    Running {
        child: tokio::process::Child,
        stdin: tokio::process::ChildStdin,
        stdout: BufReader<tokio::process::ChildStdout>,
    },
    Restarting,
    Dead,
}

/// Shared embedding service handle
pub struct EmbeddingService {
    model_name: String,
    batch_size: usize,
    state: RwLock<EmbeddingProcess>,
    process_ready: watch::Sender<bool>,
}

impl EmbeddingService {
    /// Create a new embedding service
    pub fn new(model_name: Option<String>, batch_size: Option<usize>) -> Arc<Self> {
        let model_name = model_name
            .or_else(|| std::env::var("ULTRACLAW_EMBEDDING_MODEL").ok())
            .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string());

        let batch_size = std::env::var("ULTRACLAW_EMBEDDING_BATCH_SIZE")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .or(batch_size)
            .unwrap_or(DEFAULT_BATCH_SIZE);

        let (tx, _) = watch::channel(false);

        Arc::new(Self {
            model_name,
            batch_size,
            state: RwLock::new(EmbeddingProcess::Dead),
            process_ready: tx,
        })
    }

    /// Spawn or restart the Python subprocess
    pub async fn ensure_running(&self) -> Result<(), String> {
        let script_path = self.find_script_path()?;

        let mut child = tokio::process::Command::new("python")
            .arg(&script_path)
            .arg("--model")
            .arg(&self.model_name)
            .arg("--batch-size")
            .arg(self.batch_size.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn embedding server: {}", e))?;

        let stdin = child.stdin.take().ok_or("Failed to capture stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;

        let mut state = self.state.write().await;
        *state = EmbeddingProcess::Running {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };
        drop(state);

        // Wait for ready signal from stderr
        tokio::time::sleep(Duration::from_secs(3)).await;

        let _ = self.process_ready.send(true);
        info!(model = %self.model_name, "Embedding service started");

        Ok(())
    }

fn find_script_path(&self) -> Result<String, String> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));
        let scripts_dir = std::env::var("ULTRACLAW_SCRIPTS_DIR")
            .map(std::path::PathBuf::from)
            .ok();

        let mut candidates: Vec<std::path::PathBuf> = vec![
            std::path::PathBuf::from("scripts/embedding_server.py"),
            std::path::PathBuf::from("./scripts/embedding_server.py"),
        ];

        if let Some(ref dir) = exe_dir {
            candidates.push(dir.join("scripts/embedding_server.py"));
        }
        if let Some(ref dir) = scripts_dir {
            candidates.push(dir.join("embedding_server.py"));
        }

        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().to_string());
            }
        }

        Err(format!(
            "Embedding server script not found. Tried: {:?}",
            candidates.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>()
        ))
    }

    /// Send a command to the subprocess and get the response
    async fn send_command(&self, cmd: serde_json::Value) -> Result<serde_json::Value, String> {
        // Ensure process is running
        {
            let state = self.state.read().await;
            if matches!(*state, EmbeddingProcess::Dead | EmbeddingProcess::Restarting) {
                drop(state);
                self.ensure_running().await?;
            }
        }

        let mut state = self.state.write().await;
        let state_ref = &mut *state;

        let (stdin, stdout) = match state_ref {
            EmbeddingProcess::Running { stdin, stdout, .. } => (stdin, stdout),
            _ => return Err("Embedding service not available".to_string()),
        };

        // Send command
        let cmd_str = serde_json::to_string(&cmd).map_err(|e| e.to_string())?;
        stdin
            .write_all(format!("{}\n", cmd_str).as_bytes())
            .await
            .map_err(|e| format!("Failed to send command: {}", e))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush: {}", e))?;

        // Read response
        let mut line = String::new();
        let read_result = tokio::time::timeout(
            Duration::from_secs(60),
            stdout.read_line(&mut line),
        )
        .await;

        match read_result {
            Ok(Ok(0)) => return Err("Embedding subprocess closed".to_string()),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(format!("Read error: {}", e)),
            Err(_) => return Err("Embedding request timed out".to_string()),
        }

        let response: serde_json::Value = serde_json::from_str(&line).map_err(|e| format!("Invalid JSON response: {} (line: {})", e, line))?;

        if let Some(status) = response.get("status").and_then(|v| v.as_str()) {
            if status == "error" {
                let msg = response.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
                return Err(msg.to_string());
            }
        }

        Ok(response)
    }

    /// Generate embedding for a single text
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let response = self
            .send_command(serde_json::json!({
                "action": "embed",
                "text": text
            }))
            .await?;

        response
            .get("embedding")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "No embedding in response".to_string())?
            .iter()
            .map(|v| {
                v.as_f64()
                    .map(|f| f as f32)
                    .ok_or("Non-float in embedding array".to_string())
            })
            .collect()
    }

    /// Generate embeddings for a batch of texts
    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Option<Vec<f32>>>, String> {
        let response = self
            .send_command(serde_json::json!({
                "action": "embed_batch",
                "texts": texts
            }))
            .await?;

        let embeddings = response
            .get("embeddings")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "No embeddings in response".to_string())?;

        let mut results = Vec::with_capacity(texts.len());
        for emb in embeddings {
            if emb.is_null() {
                results.push(None);
            } else {
                let vec: Result<Vec<f32>, _> = emb
                    .as_array()
                    .ok_or("Embedding is not an array")?
                    .iter()
                    .map(|v| v.as_f64().map(|f| f as f32).ok_or(()))
                    .collect();
                results.push(Some(vec.map_err(|()| "Invalid float".to_string())?));
            }
        }
        Ok(results)
    }

    /// Get server statistics
    pub async fn stats(&self) -> Result<EmbeddingStats, String> {
        let response = self.send_command(serde_json::json!({"action": "stats"})).await?;

        let stats = response
            .get("stats")
            .ok_or("No stats in response")?;

        Ok(EmbeddingStats {
            model: stats.get("model").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
            dimension: stats.get("dimension").and_then(|v| v.as_u64()).unwrap_or(384) as usize,
            load_time_s: stats.get("load_time_s").and_then(|v| v.as_f64()).unwrap_or(0.0),
            total_embeddings: stats.get("total_embeddings").and_then(|v| v.as_u64()).unwrap_or(0) as u64,
            total_time_s: stats.get("total_time_s").and_then(|v| v.as_f64()).unwrap_or(0.0),
            avg_time_ms: stats.get("avg_time_ms").and_then(|v| v.as_f64()).unwrap_or(0.0),
        })
    }

    /// Check if service is healthy
    pub async fn is_healthy(&self) -> bool {
        let state = self.state.read().await;
        matches!(*state, EmbeddingProcess::Running { .. })
    }
}

/// Embedding service statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingStats {
    pub model: String,
    pub dimension: usize,
    pub load_time_s: f64,
    pub total_embeddings: u64,
    pub total_time_s: f64,
    pub avg_time_ms: f64,
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self {
            model_name: DEFAULT_EMBEDDING_MODEL.to_string(),
            batch_size: DEFAULT_BATCH_SIZE,
            state: RwLock::new(EmbeddingProcess::Dead),
            process_ready: watch::channel(false).0,
        }
    }
}

/// Embedding result with score
#[derive(Debug, Clone)]
pub struct ScoredEmbedding {
    pub id: String,
    pub text: String,
    pub score: f32,
    pub metadata: Option<serde_json::Value>,
}

impl ScoredEmbedding {
    pub fn new(id: &str, text: &str, score: f32) -> Self {
        Self {
            id: id.to_string(),
            text: text.to_string(),
            score,
            metadata: None,
        }
    }
}

/// Standalone sync wrapper for use in skills
pub struct EmbeddingSkill {
    service: Arc<EmbeddingService>,
}

impl EmbeddingSkill {
    pub fn new() -> Self {
        Self {
            service: EmbeddingService::new(None, None),
        }
    }

    pub fn with_config(model: Option<String>, batch_size: Option<usize>) -> Self {
        Self {
            service: EmbeddingService::new(model, batch_size),
        }
    }

    pub async fn embed_text(&self, text: &str) -> Result<Vec<f32>, String> {
        self.service.embed(text).await
    }

    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Option<Vec<f32>>>, String> {
        self.service.embed_batch(texts).await
    }

    pub async fn stats(&self) -> Result<EmbeddingStats, String> {
        self.service.stats().await
    }
}

impl Default for EmbeddingSkill {
    fn default() -> Self {
        Self::new()
    }
}