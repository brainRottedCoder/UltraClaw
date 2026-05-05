use crate::db::ChatMessage;
use crate::inference::InferenceEngine;
use crate::skill::{SkillRegistry, SkillOutput};
use crate::tools;
use serde::Serialize;
use serde_json::Value;
use std::time::Instant;
use tracing::{info, warn};

const ERROR_KEYWORDS: &[&str] = &["Error:", "error:", "Failed", "failed", "not found", "Permission denied", "timeout", "invalid", "cannot"];

#[derive(Debug, Clone, Serialize)]
pub struct HealingMetrics {
    pub heal_cycles_used: usize,
    pub errors_encountered: Vec<String>,
    pub final_success: bool,
}

impl HealingMetrics {
    pub fn summary(&self) -> String {
        format!(
            "Self-Healing: {} cycles, {} errors, final: {}",
            self.heal_cycles_used,
            self.errors_encountered.len(),
            if self.final_success { "SUCCESS" } else { "FAILED" }
        )
    }
}

fn detect_error(output: &str) -> Option<String> {
    if output.trim().is_empty() {
        return Some("Tool returned empty output".to_string());
    }
    for keyword in ERROR_KEYWORDS {
        if output.contains(keyword) {
            return Some(format!("Detected error pattern: '{}' in output", keyword));
        }
    }
    None
}

pub async fn execute_with_healing(
    engine: &dyn InferenceEngine,
    messages: &mut Vec<ChatMessage>,
    tools: Value,
    skill_registry: &SkillRegistry,
    temperature: f32,
    max_tokens: u32,
    max_heal_cycles: usize,
) -> Result<(String, HealingMetrics), String> {
    let mut healing = HealingMetrics {
        heal_cycles_used: 0,
        errors_encountered: Vec::new(),
        final_success: false,
    };
    let mut current_messages = messages.clone();
    let start_time = Instant::now();

    for cycle in 0..=max_heal_cycles {
        if cycle > 0 {
            healing.heal_cycles_used = cycle;
            info!(cycle = cycle, "Self-healing cycle");
        }

        let response = engine
            .infer(current_messages.clone(), Some(tools.clone()), temperature, max_tokens)
            .await?;

        let tool_calls = tools::parse_tool_calls(&response);

        if tool_calls.is_empty() {
            messages.clear();
            messages.extend(current_messages.iter().cloned());
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.clone(),
            });
            healing.final_success = true;
            info!("Self-healing: task completed successfully");

            let elapsed = start_time.elapsed().as_millis() as u64;
            info!(elapsed_ms = elapsed, cycles = healing.heal_cycles_used, "Healing cycle stats");
            return Ok((response, healing));
        }

        current_messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: response.clone(),
        });

        let tool_results_text =
            tools::execute_tool_calls(&tool_calls, skill_registry, None).await;

        let mut had_error = false;
        let mut error_descriptions = Vec::new();

        let parts: Vec<&str> = tool_results_text.split("--- Tool:").collect();
        for part in parts.iter().skip(1) {
            if let Some(error) = detect_error(part) {
                had_error = true;
                let tool_name = part.lines().next().unwrap_or("unknown").trim();
                let desc = format!("Tool '{}': {}", tool_name, error);
                error_descriptions.push(desc.clone());
                healing.errors_encountered.push(desc);
            }
        }

        if !had_error {
            current_messages.push(ChatMessage {
                role: "system".to_string(),
                content: format!("Tool execution results:\n{}", tool_results_text),
            });

            let final_response = engine
                .infer(current_messages.clone(), None, temperature, max_tokens)
                .await?;

            current_messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: final_response.clone(),
            });

            messages.clear();
            messages.extend(current_messages.iter().cloned());
            healing.final_success = true;

            let elapsed = start_time.elapsed().as_millis() as u64;
            info!(elapsed_ms = elapsed, "Healing: all tools succeeded");
            return Ok((final_response, healing));
        }

        warn!(
            cycle = cycle,
            error_count = error_descriptions.len(),
            "Self-healing: errors detected, attempting recovery"
        );

        if cycle < max_heal_cycles {
            let mut healing_prompt = String::from(
                "The previous tool execution encountered errors. Please analyze what went wrong and try a different approach.\n\n",
            );
            healing_prompt.push_str("Errors encountered:\n");
            for err in &error_descriptions {
                healing_prompt.push_str(&format!("  - {}\n", err));
            }
            healing_prompt.push_str("\n");
            healing_prompt.push_str(
                "Consider:\n\
                 1. Try a different tool if available\n\
                 2. Try different arguments (e.g., check the path exists first)\n\
                 3. Break the task into smaller steps\n\
                 4. If a file is not found, list the directory first\n\n\
                 Please re-attempt the task with a corrected approach.",
            );

            current_messages.push(ChatMessage {
                role: "system".to_string(),
                content: format!(
                    "Tool results (with errors):\n{}\n\nHEALING GUIDANCE:\n{}",
                    tool_results_text, healing_prompt
                ),
            });
        } else {
            let exhausted_prompt = format!(
                "Maximum self-healing cycles ({}) exhausted. Errors:\n{}",
                max_heal_cycles,
                error_descriptions.join("\n")
            );

            current_messages.push(ChatMessage {
                role: "system".to_string(),
                content: format!(
                    "Tool results:\n{}\n\n{}\n\nPlease provide your best answer explaining what went wrong.",
                    tool_results_text, exhausted_prompt
                ),
            });

            let final_response = engine
                .infer(current_messages.clone(), None, temperature, max_tokens)
                .await?;

            current_messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: final_response.clone(),
            });

            messages.clear();
            messages.extend(current_messages.iter().cloned());

            return Ok((final_response, healing));
        }
    }

    let exhausted_msg = "All healing cycles exhausted without resolution.".to_string();
    messages.clear();
    messages.extend(current_messages.iter().cloned());
    Ok((exhausted_msg, healing))
}

pub async fn run_self_healing_task(
    engine: &dyn InferenceEngine,
    skill_registry: &SkillRegistry,
    user_message: &str,
    temperature: f32,
    max_tokens: u32,
    max_heal_cycles: usize,
) -> Result<(String, HealingMetrics), String> {
    let system_msg = concat!(
        "You are an AI agent that completes tasks using available tools. ",
        "Always use tools when asked to perform file operations, commands, or searches. ",
        "Be persistent: if a tool fails, try a different approach. ",
        "Output JSON tool calls in ```json blocks."
    );

    let mut messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: system_msg.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
        },
    ];

    let tool_schema = skill_registry.to_tool_schema();

    execute_with_healing(
        engine,
        &mut messages,
        tool_schema,
        skill_registry,
        temperature,
        max_tokens,
        max_heal_cycles,
    )
    .await
}