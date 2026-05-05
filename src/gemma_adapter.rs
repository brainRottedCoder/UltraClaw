// ============================================================================
// ULTRACLAW — gemma_adapter.rs
// ============================================================================
// Gemma 4 E4B-specific message formatting and tool schema transformation.
//
// Gemma uses a special token format that's different from OpenAI's JSON format.
// This adapter enables Gemma 4 E4B to properly understand system prompts and
// execute tool calls with high reliability.
//
// MEMORY OPTIMIZATION:
// - All static strings live in .rodata (zero heap allocation).
// - Dynamic transformations use pre-allocated String capacity.
// - No regex engine (saves ~1MB binary size).
//
// ENERGY OPTIMIZATION:
// - Format detection is a simple string contains() check (O(1)).
// - Tool schema transformation only happens when Gemma is detected.
// ============================================================================

use crate::db::ChatMessage;
use crate::skill::{SkillRegistry, ToolCall};
use serde_json::Value;
use tracing::info;

/// Gemma model identifiers (used for detection).
const GEMMA_MODEL_IDENTIFIERS: &[&str] = &[
    "gemma",
    "gemma4",
    "gemma3",
    "gemma2",
    "google/gemma",
];

/// Maximum iterations for ReAct tool loop with Gemma.
/// Gemma 4 E4B can handle 3-5 tool calls reliably before quality degrades.
pub const GEMMA_MAX_TOOL_ITERATIONS: usize = 5;

/// Gemma-specific system prompt directives.
/// These get injected when Gemma is detected as the active model.
const GEMMA_SYSTEM_DIRECTIVES: &str = r#"You have access to tools (functions) that you can call.
Think step-by-step before using a tool. Plan your approach.
Only call a tool when the user's request genuinely requires it.
After calling a tool, analyze the results and decide if more tools are needed.
If you don't know something, say so. Do not make up information.
When you need to call a tool, respond with a JSON object in this exact format:
{"name": "tool_name", "parameters": {"param1": "value1", "param2": "value2"}}
Do NOT call multiple tools in one response. Call ONE tool at a time."#;

/// Determines if a model name is a Gemma variant.
pub fn is_gemma_model(model_name: &str) -> bool {
    let model_lower = model_name.to_lowercase();
    GEMMA_MODEL_IDENTIFIERS
        .iter()
        .any(|id| model_lower.contains(id))
}

/// Transforms the tool schema from OpenAI format to Gemma-compatible format.
///
/// OpenAI format:
///   {"type": "function", "function": {"name": "read_file", "parameters": {...}}}
///
/// Gemma format:
///   {"name": "read_file", "parameters": {...}, "description": "..."}
///
/// Gemma 4 E4B responds significantly better to the compact Gemma format
/// because it was fine-tuned on tool-calling using this structure.
pub fn transform_tool_schema_for_gemma(tools: &Value) -> Value {
    let Some(arr) = tools.as_array() else {
        return tools.clone();
    };

    let gemma_tools: Vec<Value> = arr
        .iter()
        .filter_map(|tool| {
            let func = tool.get("function")?;
            let name = func.get("name")?.as_str()?;
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

/// Transforms OpenAI-style messages to Gemma-compatible text format.
///
/// Gemma 4 E4B is trained on a specific message format:
///   <start_of_turn>user
///   message content
///   <end_of_turn>
///   <start_of_turn>model
///   response
///   <end_of_turn>
///
/// This adapter converts OpenAI's JSON message format into Gemma's native format,
/// which significantly improves tool-calling reliability and response quality.
pub fn format_messages_for_gemma(messages: &[ChatMessage]) -> String {
    let mut output = String::with_capacity(messages.len() * 200);

    for msg in messages {
        let role = match msg.role.as_str() {
            "system" => "system",
            "user" => "user",
            "assistant" | "model" => "model",
            _ => "user",
        };

        output.push_str("<start_of_turn>");
        output.push_str(role);
        output.push('\n');
        output.push_str(&msg.content.trim());
        output.push_str("\n<end_of_turn>\n");
    }

    output
}

/// Formats the system prompt specifically for Gemma 4 E4B.
///
/// Gemma responds best to system prompts that:
///
/// 1. Define the agent's role clearly at the start
/// 2. Explicitly list available tools with descriptions
/// 3. Give clear instructions on when/how to call tools
/// 4. Set behavioral expectations (conciseness, safety, etc.)
///
/// # Arguments
/// * `base_system_prompt` - The original system prompt from Soul module
/// * `registry` - The skill registry to extract tool definitions from
pub fn build_gemma_system_prompt(
    base_system_prompt: &str,
    registry: &SkillRegistry,
) -> String {
    let mut prompt = String::with_capacity(4096);

    // Core personality
    prompt.push_str(base_system_prompt);
    prompt.push_str("\n\n## Gemma-Specific Directives\n");
    prompt.push_str(GEMMA_SYSTEM_DIRECTIVES);
    prompt.push_str("\n\n## Available Tools\n");

    // Inject tool definitions in Gemma format
    let tools = registry.to_tool_schema();
    let gemma_tools = transform_tool_schema_for_gemma(&tools);

    if let Some(arr) = gemma_tools.as_array() {
        for tool in arr {
            let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
            let description = tool.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let params = tool.get("parameters");

            prompt.push_str(&format!("- {}: {}", name, description));
            if let Some(params_obj) = params.and_then(|p| p.as_object()) {
                if let Some(props) = params_obj.get("properties").and_then(|p| p.as_object()) {
                    let args: Vec<String> = props
                        .keys()
                        .map(|k| {
                            let desc = props[k]
                                .get("description")
                                .and_then(|d| d.as_str())
                                .unwrap_or("");
                            format!("{}: {}", k, desc)
                        })
                        .collect();
                    if !args.is_empty() {
                        prompt.push_str(&format!(" Args: [{}]", args.join(", ")));
                    }
                }
            }
            prompt.push('\n');
        }
    }

    prompt.push_str("\nUse tools when needed. Be concise. Be accurate.\n");

    prompt
}

/// Gemma-specific temperature and token settings for optimal tool-calling.
///
/// Gemma 4 E4B performs best with:
/// - Lower temperature (0.2-0.3) for consistent tool-calling
/// - Higher max_tokens for complex responses with tool results
/// - No repetition penalty (handled by model)
pub struct GemmaConfig {
    pub temperature: f32,
    pub max_tokens: u32,
    pub top_p: f32,
    pub top_k: i32,
}

impl Default for GemmaConfig {
    fn default() -> Self {
        Self {
            temperature: 0.2, // Low temp for consistent tool calls
            max_tokens: 4096, // Higher for complex agentic responses
            top_p: 0.95,
            top_k: 64,
        }
    }
}

/// Check if Gemma model is available and configured correctly.
pub async fn check_gemma_health(
    ollama_base_url: &str,
    model_name: &str,
) -> Result<GemmaHealthStatus, String> {
    use reqwest::Client;

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    // Get list of available models
    let url = format!("{}/api/tags", ollama_base_url.trim_end_matches('/'));
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to connect to Ollama: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Ollama returned status: {}", response.status()));
    }

    #[derive(serde::Deserialize)]
    struct OllamaModelList {
        models: Vec<OllamaModelInfo>,
    }

    #[derive(serde::Deserialize)]
    struct OllamaModelInfo {
        name: String,
    }

    let model_list: OllamaModelList = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

    let gemma_available = model_list
        .models
        .iter()
        .any(|m| m.name.to_lowercase().contains("gemma"));

    let gemma_installed = model_list
        .models
        .iter()
        .any(|m| m.name.to_lowercase() == model_name.to_lowercase());

    Ok(GemmaHealthStatus {
        ollama_running: true,
        gemma_available,
        gemma_installed,
        model_name: model_name.to_string(),
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GemmaHealthStatus {
    pub ollama_running: bool,
    pub gemma_available: bool,
    pub gemma_installed: bool,
    pub model_name: String,
}

impl GemmaHealthStatus {
    /// Returns instructions for the user to install Gemma if not present.
    pub fn install_instructions(&self) -> Option<String> {
        if !self.gemma_installed {
            Some(format!(
                "Gemma {} is not installed. Run:\n  ollama pull {}",
                self.model_name, self.model_name
            ))
        } else {
            None
        }
    }
}

/// Log Gemma-specific diagnostic info at startup.
pub fn log_gemma_diagnostics(model_name: &str, gemma_config: &GemmaConfig) {
    if is_gemma_model(model_name) {
        info!(
            model = %model_name,
            temperature = gemma_config.temperature,
            max_tokens = gemma_config.max_tokens,
            "Gemma 4 E4B configuration applied"
        );
        info!(
            "System prompt will be adapted for Gemma's native format",
        );
        info!(
            "Tool schema will be transformed to Gemma-compatible format",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gemma_model_detection() {
        assert!(is_gemma_model("gemma4:e4b"));
        assert!(is_gemma_model("gemma3-2b-it"));
        assert!(is_gemma_model("gemma2-2b"));
        assert!(is_gemma_model("google/gemma4:e4b"));
        assert!(!is_gemma_model("llama3.2:3b"));
        assert!(!is_gemma_model("mistral"));
        assert!(!is_gemma_model("qwen2.5"));
    }

    #[test]
    fn test_tool_schema_transformation() {
        let openai_tools = serde_json::json!([
            {
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a file from filesystem",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string", "description": "File path"}
                        },
                        "required": ["path"]
                    }
                }
            }
        ]);

        let gemma_tools = transform_tool_schema_for_gemma(&openai_tools);

        assert!(gemma_tools.is_array());
        let arr = gemma_tools.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("name").and_then(|v| v.as_str()), Some("read_file"));
        assert!(!arr[0].get("type").is_some()); // No "type" field in Gemma format
    }

    #[test]
    fn test_message_formatting() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You are an assistant".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
            },
        ];

        let formatted = format_messages_for_gemma(&messages);

        assert!(formatted.contains("<start_of_turn>system"));
        assert!(formatted.contains("<start_of_turn>user"));
        assert!(formatted.contains("<start_of_turn>model")); // assistant -> model
        assert!(formatted.contains("<end_of_turn>"));
    }
}