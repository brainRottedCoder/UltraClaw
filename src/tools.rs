// ============================================================================
// ULTRACLAW — tools.rs
// ============================================================================
// Bridge between LLM output and the Skill/MCP execution layer.
//
// The LLM may output structured tool-call blocks (JSON) embedded in its
// response. This module parses those blocks and dispatches them to either
// the built-in SkillRegistry or an external MCP server.
//
// MEMORY OPTIMIZATION:
// - Tool call parsing uses a single-pass scan over the response string.
//   No regex engine (regex crate is ~1MB compiled). We match `{"name":` patterns.
// - Parsed tool calls are small structs (~100 bytes each). Even with 10
//   tool calls in a single response, total allocation is ~1KB.
// - Tool outputs are capped at 4KB per call (enforced in skill.rs).
//
// ENERGY OPTIMIZATION:
// - Tools are executed sequentially, not in parallel. This prevents CPU
//   thrashing from multiple concurrent command executions.
// - Each tool execution has a 10-second timeout (enforced in skill.rs).
// ============================================================================

use crate::mcp::McpClient;
use crate::skill::{SkillOutput, SkillRegistry, ToolCall};
use serde_json::Value;
use tracing::{info, warn};

/// Parse tool calls from LLM output text.
///
/// Supports two formats:
/// 1. OpenAI function-calling JSON blocks
/// 2. Inline JSON tool calls (```json blocks containing {"name": ..., "arguments": ...})
///
/// # Memory Usage
/// We scan the string once (O(N)) and extract JSON substrings.
/// No copies of the full response are made — we use string slices
/// and only allocate for the parsed ToolCall structs.
pub fn parse_tool_calls(text: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();

    // Strategy 1: Look for JSON code blocks containing tool calls
    // Pattern: ```json\n{...}\n``` or ```\n{...}\n```
    let mut search_pos = 0;
    while let Some(start) = text[search_pos..].find("```") {
        let abs_start = search_pos + start;
        // Skip the ``` and optional language tag
        let content_start = text[abs_start + 3..]
            .find('\n')
            .map(|p| abs_start + 3 + p + 1)
            .unwrap_or(abs_start + 3);

        if let Some(end) = text[content_start..].find("```") {
            let json_str = &text[content_start..content_start + end].trim();

            // Try to parse as a tool call
            if let Ok(value) = serde_json::from_str::<Value>(json_str) {
                if let Some(call) = extract_tool_call(&value) {
                    calls.push(call);
                }
            }
            search_pos = content_start + end + 3;
        } else {
            break;
        }
    }

    // Strategy 2: Look for loose JSON objects with "name" and "arguments" keys
    // This catches tool calls not wrapped in code blocks
    if calls.is_empty() {
        let mut brace_depth = 0i32;
        let mut json_start: Option<usize> = None;

        for (i, ch) in text.char_indices() {
            match ch {
                '{' => {
                    if brace_depth == 0 {
                        json_start = Some(i);
                    }
                    brace_depth += 1;
                }
                '}' => {
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        if let Some(start) = json_start {
                            let candidate = &text[start..=i];
                            if candidate.contains("\"name\"") {
                                if let Ok(value) = serde_json::from_str::<Value>(candidate) {
                                    if let Some(call) = extract_tool_call(&value) {
                                        calls.push(call);
                                    }
                                }
                            }
                        }
                        json_start = None;
                    }
                }
                _ => {}
            }
        }
    }

    calls
}

/// Extract a ToolCall from a parsed JSON Value.
fn extract_tool_call(value: &Value) -> Option<ToolCall> {
    let name = value.get("name")?.as_str()?.to_string();
    let arguments = value
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    Some(ToolCall { name, arguments })
}

/// Execute a list of tool calls using the skill registry and optional MCP client.
///
/// Returns a formatted string of all tool outputs, suitable for injection
/// back into the conversation as a "tool" or "system" message.
///
/// # Execution Order
/// Tools are executed sequentially to prevent resource contention.
/// On a single-core device, parallel execution would just cause context
/// switching overhead without actual parallelism.
pub async fn execute_tool_calls(
    calls: &[ToolCall],
    skill_registry: &SkillRegistry,
    mcp_client: Option<&McpClient>,
) -> String {
    let mut results = Vec::with_capacity(calls.len());

    for call in calls {
        info!(tool = %call.name, "Executing tool call");

        // Try built-in skills first
        if let Some(output) = skill_registry.dispatch(call) {
            results.push(output);
            continue;
        }

        // Try MCP server if available
        if let Some(mcp) = mcp_client {
            match mcp.call_tool(&call.name, call.arguments.clone()).await {
                Ok(mcp_result) => {
                    results.push(SkillOutput {
                        name: call.name.clone(),
                        output: serde_json::to_string_pretty(&mcp_result)
                            .unwrap_or_else(|_| mcp_result.to_string()),
                        is_error: false,
                    });
                    continue;
                }
                Err(e) => {
                    warn!(tool = %call.name, error = %e, "MCP tool call failed");
                }
            }
        }

        // Tool not found in any registry
        results.push(SkillOutput {
            name: call.name.clone(),
            output: format!("Error: tool '{}' not found in any registry", call.name),
            is_error: true,
        });
    }

    // Format results for injection back into the conversation
    format_tool_results(&results)
}

/// Format tool execution results into a single string for the LLM.
///
/// Each result is wrapped in a clear delimiter so the LLM can parse
/// individual tool outputs even when multiple tools were called.
fn format_tool_results(results: &[SkillOutput]) -> String {
    if results.is_empty() {
        return String::new();
    }

    let mut output = String::with_capacity(results.len() * 256);
    for result in results {
        output.push_str(&format!("--- Tool: {} ---\n", result.name));
        if result.is_error {
            output.push_str("[ERROR] ");
        }
        output.push_str(&result.output);
        output.push_str("\n\n");
    }
    output
}

// ============================================================================
// REACT TOOL LOOP — Multi-turn reasoning with Gemma 4 2B
// ============================================================================

use crate::db::ChatMessage;
use crate::inference::InferenceEngine;

/// ReAct (Reasoning + Acting) loop for multi-turn tool orchestration.
///
/// Gemma 4 2B excels at chained reasoning but needs iterative tool calls
/// rather than single-shot multi-tool invocations. This loop implements:
///
/// 1. Infer: Get LLM response
/// 2. Parse: Extract tool calls from response
/// 3. Act: Execute tools and collect results
/// 4. Reason: Feed results back to LLM for next step
/// 5. Repeat until no more tools needed or max iterations reached
///
/// # Arguments
/// * `engine` - The inference engine (LocalEngine, CloudEngine, or FailoverEngine)
/// * `messages` - Conversation history (will be mutated)
/// * `tools` - Tool schema for the LLM
/// * `skill_registry` - Registry to dispatch tool calls
/// * `mcp_client` - Optional MCP client for external tools
/// * `max_iterations` - Maximum tool loop iterations (prevents infinite loops)
/// * `temperature` - Sampling temperature
/// * `max_tokens` - Maximum response tokens
///
/// # Returns
/// The final LLM response after all tool calls are completed.
pub async fn react_tool_loop(
    engine: &dyn InferenceEngine,
    messages: &mut Vec<ChatMessage>,
    tools: Value,
    skill_registry: &SkillRegistry,
    mcp_client: Option<&McpClient>,
    max_iterations: usize,
    temperature: f32,
    max_tokens: u32,
) -> Result<String, String> {
    let mut iterations = 0;

    loop {
        iterations += 1;

        // Step 1: Get LLM response
        let response = engine
            .infer(messages.clone(), Some(tools.clone()), temperature, max_tokens)
            .await?;

        // Step 2: Parse tool calls from response
        let tool_calls = parse_tool_calls(&response);

        if tool_calls.is_empty() {
            // No more tool calls - we're done
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.clone(),
            });
            return Ok(response);
        }

        info!(
            iteration = iterations,
            tool_count = tool_calls.len(),
            "Executing tool calls in ReAct loop"
        );

        // Log first tool call for debugging
        if let Some(first_call) = tool_calls.first() {
            info!(tool = %first_call.name, "First tool call");
        }

        // Step 3: Execute tools
        let tool_results = execute_tool_calls(&tool_calls, skill_registry, mcp_client).await;

        // Add assistant response with tool calls to history
        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: response.clone(),
        });

        // Add tool results as a system message
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: format!("Tool execution results:\n{}", tool_results),
        });

        // Step 5: Check iteration limit
        if iterations >= max_iterations {
            warn!(
                iteration = iterations,
                "Max ReAct iterations reached - forcing completion"
            );
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: "Maximum tool iterations reached. Please provide your best answer based on the available information.".to_string(),
            });

            // One final inference without tools to get a clean response
            let final_response = engine
                .infer(messages.clone(), None, temperature, max_tokens)
                .await?;

            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: final_response.clone(),
            });

            return Ok(final_response);
        }
    }
}

/// Streaming version of the ReAct tool loop.
///
/// This variant streams tokens to the output while still executing tools
/// in a loop. Tools are executed silently, and only the final LLM response
/// is streamed.
///
/// Note: This is a simplified version that executes all tools before
/// streaming. For true streaming during tool execution, a more complex
/// implementation would be needed.
pub async fn react_tool_loop_streaming(
    engine: &dyn InferenceEngine,
    messages: &mut Vec<ChatMessage>,
    tools: Value,
    skill_registry: &SkillRegistry,
    mcp_client: Option<&McpClient>,
    max_iterations: usize,
    temperature: f32,
    max_tokens: u32,
) -> Result<String, String> {
    let mut iterations = 0;
    let mut full_response = String::new();

    loop {
        iterations += 1;

        // Get response (streaming)
        let response = engine
            .infer(messages.clone(), Some(tools.clone()), temperature, max_tokens)
            .await?;

        // Parse and execute tools
        let tool_calls = parse_tool_calls(&response);

        if tool_calls.is_empty() {
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.clone(),
            });
            full_response.push_str(&response);
            return Ok(full_response);
        }

        info!(iteration = iterations, "ReAct streaming iteration");

        let tool_results = execute_tool_calls(&tool_calls, skill_registry, mcp_client).await;

        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: response.clone(),
        });

        messages.push(ChatMessage {
            role: "system".to_string(),
            content: format!("Tool execution results:\n{}", tool_results),
        });

        if iterations >= max_iterations {
            let final_response = engine
                .infer(messages.clone(), None, temperature, max_tokens)
                .await?;

            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: final_response.clone(),
            });

            return Ok(final_response);
        }
    }
}

/// Metrics for the ReAct tool loop (for hackathon documentation).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ReactMetrics {
    pub iterations: usize,
    pub tools_called: usize,
    pub total_tool_exec_time_ms: u64,
}

impl ReactMetrics {
    pub fn summary(&self) -> String {
        format!(
            "ReAct: {} iterations, {} tools called in {}ms",
            self.iterations, self.tools_called, self.total_tool_exec_time_ms
        )
    }
}
