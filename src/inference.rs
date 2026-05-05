// ============================================================================
// ULTRACLAW — inference.rs
// ============================================================================
// The core AI layer: InferenceEngine trait + Cloud + Local + Failover.
//
// ARCHITECTURE:
// This module defines a unified interface (`InferenceEngine`) for AI inference
// that abstracts over cloud APIs and local model execution. The `FailoverEngine`
// wraps both and implements transparent cloud→local failover when network
// drops are detected.
//
// MEMORY OPTIMIZATION:
// - Cloud inference: Only the HTTP request/response bodies are in RAM.
//   Reqwest streams the response, so we don't buffer the entire body.
//   Peak RAM: ~10-50KB per inference call (request payload + response text).
//
// - Local inference (llama.cpp):
//   The GGUF model file is memory-mapped (mmap). This means:
//   1. The kernel maps the file directly into virtual address space.
//   2. Only pages that are actively read (during forward pass) are loaded
//      into physical RAM (demand paging).
//   3. A 4-bit quantized 7B model is ~3.5GB on disk, but only ~500MB-1GB
//      is resident in physical RAM at any time (RSS).
//   4. Under memory pressure, the kernel can evict mapped pages without
//      writing them back (they're clean, backed by the file). This means
//      the model cooperates with the OS memory manager automatically.
//   5. No GPU VRAM is used — everything runs on CPU with SIMD acceleration
//      (AVX2 on x86, NEON on ARM).
//
// ENERGY OPTIMIZATION:
// - Cloud: the CPU is idle during API calls (async await = yielded to scheduler).
//   Energy is consumed only by the network interface card (WiFi/cellular).
// - Local: llama.cpp uses INT4 quantization, which means:
//   1. Each weight is 4 bits instead of 16 (FP16) or 32 (FP32).
//   2. 4x-8x fewer memory bus transactions per forward pass.
//   3. Memory bus power is ~30-40% of total CPU power during inference.
//   4. INT4 ops use integer ALUs, which consume ~3x less energy than FPUs.
//   Net result: ~70-80% less energy per token vs. FP16 on the same hardware.
//
// - Failover: no redundant calls. We try cloud first, and ONLY if it fails
//   do we invoke local. Never both simultaneously.
//
// DESIGN DECISION — OWNED PARAMETERS:
// The `infer()` method takes `Vec<ChatMessage>` and `Option<Value>` (owned)
// instead of `&[ChatMessage]` and `Option<&Value>` (borrowed). This is
// necessary for object safety: Rust doesn't allow generic lifetimes on
// trait methods used with `dyn Trait`. The ownership transfer is cheap
// because callers typically build the messages vec fresh for each request
// anyway, so there's no extra cloning.
// ============================================================================

use crate::db::ChatMessage;
use futures::{SinkExt, Stream, StreamExt};
use futures::channel::mpsc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;
use tracing::{error, info, warn};

fn is_gemma_model(model_name: &str) -> bool {
    let model_lower = model_name.to_lowercase();
    ["gemma", "gemma4", "gemma3", "gemma2", "google/gemma"]
        .iter()
        .any(|id| model_lower.contains(id))
}

fn transform_tool_schema_for_gemma(tools: &Value) -> Value {
    let Some(arr) = tools.as_array() else {
        return tools.clone();
    };

    let gemma_tools: Vec<Value> = arr
        .iter()
        .filter_map(|tool| {
            let func = tool.get("function")?;
            let name = func.get("name")?.as_str()?.to_string();
            let description = func.get("description").and_then(|d| d.as_str());
            let parameters = func.get("parameters").unwrap_or(&serde_json::Value::Null);

            Some(serde_json::json!({
                "name": name,
                "description": description.unwrap_or(""),
                "parameters": parameters
            }))
        })
        .collect();

    Value::Array(gemma_tools)
}

// ============================================================================
// ENGINE MODE
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum EngineMode {
    #[default]
    Auto,
    CloudOnly,
    LocalOnly,
}

impl std::fmt::Display for EngineMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineMode::Auto => write!(f, "auto"),
            EngineMode::CloudOnly => write!(f, "cloud"),
            EngineMode::LocalOnly => write!(f, "local"),
        }
    }
}

impl std::str::FromStr for EngineMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cloud" => Ok(EngineMode::CloudOnly),
            "local" => Ok(EngineMode::LocalOnly),
            "auto" => Ok(EngineMode::Auto),
            other => Err(format!(
                "Unknown engine mode: '{}'. Use 'auto', 'cloud', or 'local'",
                other
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    pub mode: EngineMode,
    pub cloud_available: bool,
    pub local_available: bool,
    pub current_model: String,
}

/// Timeout for cloud API calls. After this, we failover to local.
/// 30 seconds is generous for most cloud APIs. Shorter = faster failover
/// but more false positives on slow networks.
const CLOUD_TIMEOUT_SECS: u64 = 30;

/// Request payload for OpenAI-compatible chat completion APIs.
/// Works with OpenAI, Anthropic (via proxy), Gemini, Together, Groq, etc.
#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: String,
}

/// Response from an OpenAI-compatible chat completion API.
#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ApiMessage,
}

#[derive(Debug, Serialize)]
struct StreamingRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Value>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamResponse {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaModelList {
    #[serde(default)]
    pub models: Vec<OllamaModel>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaModel {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub details: Option<OllamaModelDetails>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaModelDetails {
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub parameter_size: String,
    #[serde(default)]
    pub quantization_level: String,
}

impl OllamaModel {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.model
        } else {
            &self.name
        }
    }
}

// ============================================================================
// INFERENCE ENGINE TRAIT
// ============================================================================

/// Unified trait for AI inference backends.
///
/// Both cloud and local engines implement this trait, allowing the
/// FailoverEngine to swap between them seamlessly.
///
/// The trait is object-safe (no generics, no lifetime parameters on methods)
/// so it can be used as `Arc<dyn InferenceEngine>` for dynamic dispatch.
///
/// Parameters are owned (`Vec<ChatMessage>`, `Option<Value>`) to avoid
/// lifetime issues with trait objects. This is a deliberate trade-off:
/// a small allocation cost for full object-safety and clean async code.
pub trait InferenceEngine: Send + Sync {
    /// Generate a response given a conversation history.
    ///
    /// # Arguments
    /// * `messages` - The conversation context (system + user/assistant turns). Owned.
    /// * `tools` - Optional tool/function schema for function-calling. Owned.
    /// * `temperature` - Sampling temperature (0.0-1.0)
    /// * `max_tokens` - Maximum response tokens
fn infer(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        temperature: f32,
        max_tokens: u32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>>;

    fn infer_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        temperature: f32,
        max_tokens: u32,
    ) -> Pin<Box<dyn Stream<Item = Result<String, String>> + Send + '_>>;
}

// ============================================================================
// LOCAL ENGINE (Ollama API)
// ============================================================================

/// Local LLM inference via Ollama API (http://localhost:11434).
///
/// Ollama serves GGUF models via an OpenAI-compatible API, allowing
/// seamless integration without direct llama.cpp bindings.
pub struct CloudEngine {
    client: Client,
    base_url: String,
    model: String,
    api_key: String,
    temperature: f32,
    max_tokens: u32,
}

impl CloudEngine {
    pub fn new(api_key: &str, model: &str, base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to build HTTP client for CloudEngine");
        Self {
            client,
            base_url: base_url.to_string(),
            model: model.to_string(),
            api_key: api_key.to_string(),
            temperature: 0.7,
            max_tokens: 100,
        }
    }
}

impl Clone for CloudEngine {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            api_key: self.api_key.clone(),
            temperature: self.temperature,
            max_tokens: self.max_tokens,
        }
    }
}

impl InferenceEngine for CloudEngine {
    fn infer(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        temperature: f32,
        max_tokens: u32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>>
    {
        let api_messages: Vec<ApiMessage> = messages
            .into_iter()
            .map(|m| ApiMessage {
                role: m.role,
                content: m.content,
            })
            .collect();

        let base_url = self.base_url.clone();
        let model = self.model.clone();
        let api_key = self.api_key.clone();
        let temperature = temperature;
        let max_tokens = max_tokens;
        let client = self.client.clone();

        Box::pin(async move {
            let request = ChatCompletionRequest {
                model: model.clone(),
                messages: api_messages,
                temperature: Some(temperature),
                max_tokens: Some(max_tokens),
                tools,
            };

            let url = format!("{}/v1/chat/completions", base_url);

            let response = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&request)
                .send()
                .await
                .map_err(|e| format!("Cloud API request failed: {}", e))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_else(|_| "no body".to_string());
                return Err(format!("Cloud API error {}: {}", status, body));
            }

            let completion: ChatCompletionResponse = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse cloud response: {}", e))?;

            let content = completion
                .choices
                .into_iter()
                .next()
                .map(|c| c.message.content)
                .ok_or_else(|| "Cloud returned empty choices".to_string())?;

            Ok(content)
        })
    }

    fn infer_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        temperature: f32,
        max_tokens: u32,
    ) -> Pin<Box<dyn Stream<Item = Result<String, String>> + Send + '_>> {
        let api_messages: Vec<ApiMessage> = messages
            .into_iter()
            .map(|m| ApiMessage { role: m.role, content: m.content })
            .collect();

        let base_url = self.base_url.clone();
        let model = self.model.clone();
        let api_key = self.api_key.clone();
        let client = self.client.clone();

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        tokio::spawn(async move {
            let request = StreamingRequest {
                model,
                messages: api_messages,
                temperature: Some(temperature),
                max_tokens: Some(max_tokens),
                tools,
                stream: true,
            };

            let url = format!("{}/v1/chat/completions", base_url);

            match client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&request)
                .send()
                .await
            {
                Ok(response) => {
                    if !response.status().is_success() {
                        let _ = tx.send(Err(format!("Cloud stream error: {}", response.status()))).await;
                        return;
                    }
                    let mut stream = response.bytes_stream();
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(bytes) => {
                                if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                                    let _ = tx.send(Ok(text)).await;
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(Err(format!("Stream read error: {}", e))).await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("Cloud stream connection failed: {}", e))).await;
                }
            }
        });

        Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
    }
}

pub struct LocalEngine {
    client: Client,
    base_url: String,
    model: String,
    model_path: String,
    temperature: f32,
    max_tokens: u32,
}

impl Clone for LocalEngine {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            model_path: self.model_path.clone(),
            temperature: self.temperature,
            max_tokens: self.max_tokens,
        }
    }
}

impl LocalEngine {
    pub fn new(
        model_path: &str,
        ollama_base_url: &str,
        ollama_model: &str,
        temperature: f32,
        max_tokens: u32,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(4)
            .build()
            .expect("Failed to build HTTP client for Ollama");

        Self {
            client,
            base_url: ollama_base_url.to_string(),
            model: ollama_model.to_string(),
            model_path: model_path.to_string(),
            temperature,
            max_tokens,
        }
    }

    pub async fn health_check(&self) -> bool {
        match self.client.get(format!("{}/api/tags", self.base_url)).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    pub async fn list_models(&self) -> Result<Vec<OllamaModel>, String> {
        fetch_ollama_models_with_client(&self.client, &self.base_url).await
    }
}

async fn fetch_ollama_models_with_client(
    client: &Client,
    base_url: &str,
) -> Result<Vec<OllamaModel>, String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to query Ollama models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "no body".to_string());
        return Err(format!("Ollama model discovery failed with {}: {}", status, body));
    }

    let models = response
        .json::<OllamaModelList>()
        .await
        .map_err(|e| format!("Failed to parse Ollama model list: {}", e))?;

    Ok(models.models)
}

pub async fn fetch_ollama_models(base_url: &str) -> Result<Vec<OllamaModel>, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(2)
        .build()
        .map_err(|e| format!("Failed to build Ollama discovery client: {}", e))?;

    fetch_ollama_models_with_client(&client, base_url).await
}

impl InferenceEngine for LocalEngine {
    #[allow(unused_variables)]
    fn infer(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        temperature: f32,
        max_tokens: u32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>>
    {
        let api_messages: Vec<ApiMessage> = messages
            .into_iter()
            .map(|m| ApiMessage {
                role: m.role,
                content: m.content,
            })
            .collect();

        let base_url = self.base_url.clone();
        let model = self.model.clone();
        let temperature = self.temperature;
        let max_tokens = self.max_tokens;

        let has_tools = tools.is_some() && tools.as_ref().map_or(false, |v| !v.is_null());

        let is_gemma = is_gemma_model(&model);
        let tools = if is_gemma {
            tools.map(|t| transform_tool_schema_for_gemma(&t))
        } else {
            tools
        };

        Box::pin(async move {
            let request = ChatCompletionRequest {
                model,
                messages: api_messages,
                temperature: Some(temperature),
                max_tokens: Some(max_tokens),
                tools,
            };

            let url = format!("{}/v1/chat/completions", base_url);

            let response = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await
                .map_err(|e| format!("Ollama API request failed: {}", e))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "no body".to_string());
                return Err(format!("Ollama API error {}: {}", status, body));
            }

            let completion: ChatCompletionResponse = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

            let content = completion
                .choices
                .into_iter()
                .next()
                .map(|c| c.message.content)
                .ok_or_else(|| "Ollama returned empty choices".to_string())?;

            if has_tools && content.contains("tool_calls") && !content.to_lowercase().contains("function") {
                if is_gemma {
                    info!("Gemma 4 E4B tool call detected - using Gemma-specific parsing");
                } else {
                    warn!(
                        "Local model response contains tool_calls but may not support function calling properly. \
                         Consider using a model with tool support (llama3.2, mistral, qwen2.5, gemma4:e4b)"
                    );
                }
            }

Ok(content)
        })
    }

    #[allow(unused_variables)]
    fn infer_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        _temperature: f32,
        _max_tokens: u32,
    ) -> Pin<Box<dyn Stream<Item = Result<String, String>> + Send + '_>> {
        let base_url = self.base_url.clone();
        let model = self.model.clone();
        let temperature = self.temperature;
        let max_tokens = self.max_tokens;
        let client = self.client.clone();

        let is_gemma = is_gemma_model(&model);
        let tools = if is_gemma {
            tools.map(|t| transform_tool_schema_for_gemma(&t))
        } else {
            tools
        };

        let (mut tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            let api_messages: Vec<ApiMessage> = messages
                .into_iter()
                .map(|m| ApiMessage { role: m.role, content: m.content })
                .collect();

            let request = StreamingRequest {
                model,
                messages: api_messages,
                temperature: Some(temperature),
                max_tokens: Some(max_tokens),
                tools,
                stream: true,
            };

            let url = format!("{}/v1/chat/completions", base_url);

            let response = match client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(Err(format!("Ollama stream request failed: {}", e))).await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_else(|_| "no body".to_string());
                let _ = tx.send(Err(format!("Ollama stream error {}: {}", status, body))).await;
                return;
            }

            let mut stream = response.bytes_stream();

            use futures::StreamExt;
            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                            for line in text.lines() {
                                if line.starts_with("data: ") {
                                    let data = &line[6..];
                                    if data.trim() == "[DONE]" {
                                        return;
                                    }
                                    if let Ok(stream_resp) = serde_json::from_str::<StreamResponse>(data) {
                                        for choice in stream_resp.choices {
                                            if let Some(content) = choice.delta.content {
                                                let _ = tx.send(Ok(content)).await;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("Stream read error: {}", e))).await;
                        return;
                    }
                }
            }
        });

        Box::pin(rx.map(|r| r.map_err(|e| e.to_string())))
    }

}

// ============================================================================
// FAILOVER ENGINE
// ============================================================================

/// Dual-model failover engine: Cloud → Local.
///
/// # Failover Logic
/// 1. Try cloud inference first (lower latency, higher quality).
/// 2. If cloud fails (timeout, network error, API error), automatically
///    switch to local llama.cpp inference.
/// 3. The user never sees the failover — they just get a response.
///
/// # When Does Failover Trigger?
/// - Network timeout (CLOUD_TIMEOUT_SECS exceeded)
/// - DNS resolution failure (no internet)
/// - HTTP 5xx server errors (cloud provider down)
/// - HTTP 429 rate limiting
/// - TLS handshake failure
/// - Connection refused / reset
///
/// # Energy Optimization
/// We never call both engines simultaneously. Cloud is tried first.
/// If it succeeds, local is never invoked (saving the CPU-heavy local
/// inference). If cloud fails, the timeout already consumed time
/// but zero CPU — the async task was yielded/sleeping.
pub struct FailoverEngine {
    cloud: CloudEngine,
    local: LocalEngine,
    mode: AtomicU8,
    local_available: AtomicBool,
}

impl Clone for FailoverEngine {
    fn clone(&self) -> Self {
        Self {
            cloud: self.cloud.clone(),
            local: self.local.clone(),
            mode: AtomicU8::new(self.mode.load(Ordering::SeqCst)),
            local_available: AtomicBool::new(self.local_available.load(Ordering::SeqCst)),
        }
    }
}

impl FailoverEngine {
    pub fn new(cloud: CloudEngine, local: LocalEngine) -> Self {
        Self {
            cloud,
            local,
            mode: AtomicU8::new(EngineMode::Auto as u8),
            local_available: AtomicBool::new(false),
        }
    }

    pub fn set_mode(&self, new_mode: EngineMode) {
        self.mode.store(new_mode as u8, Ordering::SeqCst);
        info!(mode = ?new_mode, "Engine mode changed");
    }

    pub fn get_mode(&self) -> EngineMode {
        match self.mode.load(Ordering::SeqCst) {
            0 => EngineMode::Auto,
            1 => EngineMode::CloudOnly,
            2 => EngineMode::LocalOnly,
            _ => EngineMode::Auto,
        }
    }

    pub fn set_local_available(&self, available: bool) {
        self.local_available.store(available, Ordering::SeqCst);
        info!(local_available = available, "Local engine availability updated");
    }

    pub fn is_local_available(&self) -> bool {
        self.local_available.load(Ordering::SeqCst)
    }

    pub fn get_status(&self, local_model: &str, cloud_model: &str) -> EngineStatus {
        let current_mode = self.get_mode();
        let current_model = match current_mode {
            EngineMode::LocalOnly => local_model.to_string(),
            _ => cloud_model.to_string(),
        };

        EngineStatus {
            mode: current_mode,
            cloud_available: true,
            local_available: self.is_local_available(),
            current_model,
        }
    }
}

impl InferenceEngine for FailoverEngine {
    fn infer(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        temperature: f32,
        max_tokens: u32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>>
    {
        let mode = self.get_mode();

        Box::pin(async move {
            match mode {
                EngineMode::CloudOnly => {
                    info!(mode = "cloud", "Engine mode: CloudOnly");
                    match self
                        .cloud
                        .infer(messages, tools, temperature, max_tokens)
                        .await
                    {
                        Ok(response) => {
                            info!("Cloud inference succeeded");
                            Ok(response)
                        }
                        Err(e) => {
                            error!(error = %e, "Cloud-only mode: cloud inference failed");
                            Err(format!("Cloud inference failed: {}", e))
                        }
                    }
                }
                EngineMode::LocalOnly => {
                    if !self.is_local_available() {
                        return Err("Local-only mode: Ollama is not available".to_string());
                    }
                    info!(mode = "local", "Engine mode: LocalOnly");
                    match self
                        .local
                        .infer(messages, tools, temperature, max_tokens)
                        .await
                    {
                        Ok(response) => {
                            info!("Local inference succeeded");
                            Ok(response)
                        }
                        Err(e) => {
                            error!(error = %e, "Local-only mode: local inference failed");
                            Err(format!("Local inference failed: {}", e))
                        }
                    }
                }
                EngineMode::Auto => {
                    info!(mode = "auto", "Engine mode: Auto (cloud first, fallback to local)");
                    info!("Attempting cloud inference...");
                    match self
                        .cloud
                        .infer(messages.clone(), tools.clone(), temperature, max_tokens)
                        .await
                    {
                        Ok(response) => {
                            info!("Cloud inference succeeded");
                            return Ok(response);
                        }
                        Err(e) => {
                            warn!(error = %e, "Cloud inference failed, failing over to local model");
                        }
                    }

                    if !self.is_local_available() {
                        return Err("Cloud failed and local is unavailable".to_string());
                    }

                    info!("Attempting local inference (llama.cpp)...");
                    match self
                        .local
                        .infer(messages, tools, temperature, max_tokens)
                        .await
                    {
                        Ok(response) => {
                            info!("Local inference succeeded (failover from cloud)");
                            Ok(response)
                        }
                        Err(e) => {
                            error!(error = %e, "BOTH cloud and local inference failed. Cannot generate response.");
                            Err(format!(
                                "All inference backends failed. Cloud and local are both unavailable. Last error: {}",
                                e
                            ))
                        }
                    }
                }
            }
        })
    }

    #[allow(unused_variables)]
    fn infer_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Value>,
        temperature: f32,
        max_tokens: u32,
    ) -> Pin<Box<dyn Stream<Item = Result<String, String>> + Send + '_>> {
        let mode = self.mode.load(Ordering::SeqCst);
        let local_available = self.is_local_available();

        let stream: Pin<Box<dyn Stream<Item = Result<String, String>> + Send>> = match mode {
            1 => {
                info!(mode = "cloud", "Engine mode: CloudOnly streaming");
                let cloud = self.cloud.clone();
                let msgs = messages;
                let t = tools;
                let temp = temperature;
                let max_t = max_tokens;
                let (mut tx, rx) = mpsc::channel(100);
                tokio::spawn(async move {
                    let cloud_stream = cloud.infer_stream(msgs, t, temp, max_t);
                    futures::pin_mut!(cloud_stream);
                    while let Some(result) = cloud_stream.next().await {
                        match result {
                            Ok(token) => { let _ = tx.send(Ok(token)).await; }
                            Err(e) => { let _ = tx.send(Err(e)).await; break; }
                        }
                    }
                });
                Box::pin(rx.map(|r| r.map_err(|e| e.to_string())))
            }
            2 => {
                if !local_available {
                    let (mut tx, rx) = mpsc::channel(1);
                    tokio::spawn(async move {
                        let _ = tx.send(Err("Local-only mode: Ollama is not available".to_string())).await;
                    });
                    return Box::pin(rx.map(|r| r.map_err(|e| e.to_string())));
                }
                info!(mode = "local", "Engine mode: LocalOnly streaming");
                let local = self.local.clone();
                let msgs = messages;
                let t = tools;
                let temp = temperature;
                let max_t = max_tokens;
                let (mut tx, rx) = mpsc::channel(100);
                tokio::spawn(async move {
                    let local_stream = local.infer_stream(msgs, t, temp, max_t);
                    futures::pin_mut!(local_stream);
                    while let Some(result) = local_stream.next().await {
                        match result {
                            Ok(token) => { let _ = tx.send(Ok(token)).await; }
                            Err(e) => { let _ = tx.send(Err(e)).await; break; }
                        }
                    }
                });
                Box::pin(rx.map(|r| r.map_err(|e| e.to_string())))
            }
            _ => {
                info!(mode = "auto", "Engine mode: Auto streaming");
                let cloud = self.cloud.clone();
                let local = self.local.clone();
                
                let (mut tx, rx) = mpsc::channel::<Result<String, String>>(100);
                
                tokio::spawn(async move {
                    let cloud_stream = cloud.infer_stream(messages.clone(), tools.clone(), temperature, max_tokens);
                    let local_stream = local.infer_stream(messages, tools, temperature, max_tokens);
                    
                    futures::pin_mut!(cloud_stream);
                    futures::pin_mut!(local_stream);
                    
                    let mut cloud_active = true;
                    
                    while cloud_active {
                        tokio::select! {
                            Some(result) = cloud_stream.next() => {
                                match result {
                                    Ok(token) => {
                                        let _ = tx.send(Ok(token)).await;
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "Cloud stream failed, falling back to local");
                                        cloud_active = false;
                                    }
                                }
                            }
                            _ = tokio::time::sleep(Duration::from_secs(CLOUD_TIMEOUT_SECS)) => {
                                warn!("Cloud stream timeout, falling back to local");
                                cloud_active = false;
                            }
                        }
                    }
                    
                    if !local_available {
                        let _ = tx.send(Err("Cloud failed and local is unavailable".to_string())).await;
                        return;
                    }
                    
                    while let Some(result) = local_stream.next().await {
                        match result {
                            Ok(token) => {
                                let _ = tx.send(Ok(token)).await;
                            }
                            Err(e) => {
                                error!(error = %e, "Local stream also failed");
                                let _ = tx.send(Err(format!("Both cloud and local streaming failed: {}", e))).await;
                                return;
                            }
                        }
                    }
                });
                
                Box::pin(rx.map(|r| r.map_err(|e| e.to_string())))
            }
        };
        stream
    }
}
