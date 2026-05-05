use crate::db::ChatMessage;
use crate::inference::InferenceEngine;
use crate::skill::SkillRegistry;
use crate::tools;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;
use tracing::{info, warn};

const DEFAULT_MAX_CHAIN_STEPS: usize = 10;
const DEFAULT_CHAIN_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainNode {
    pub step: usize,
    pub tool_name: String,
    pub output_summary: String,
    pub is_error: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChainTrace {
    pub nodes: Vec<ChainNode>,
    pub total_steps: usize,
    pub total_time_ms: u64,
    pub successful_steps: usize,
    pub failed_steps: usize,
    pub terminated_reason: String,
}

impl ChainTrace {
    pub fn summary(&self) -> String {
        format!(
            "Tool Chain: {}/{} steps OK ({} errors) in {}ms | {}",
            self.successful_steps,
            self.total_steps,
            self.failed_steps,
            self.total_time_ms,
            self.terminated_reason
        )
    }
}

pub struct ChainConfig {
    pub max_steps: usize,
    pub timeout_secs: u64,
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            max_steps: DEFAULT_MAX_CHAIN_STEPS,
            timeout_secs: DEFAULT_CHAIN_TIMEOUT_SECS,
        }
    }
}

pub async fn chain_execute(
    engine: &dyn InferenceEngine,
    messages: &mut Vec<ChatMessage>,
    tools: Value,
    skill_registry: &SkillRegistry,
    temperature: f32,
    max_tokens: u32,
    config: &ChainConfig,
) -> Result<(String, ChainTrace), String> {
    let mut trace = ChainTrace {
        nodes: Vec::new(),
        total_steps: 0,
        total_time_ms: 0,
        successful_steps: 0,
        failed_steps: 0,
        terminated_reason: String::new(),
    };

    let start_time = Instant::now();
    let max_duration = std::time::Duration::from_secs(config.timeout_secs);

    for step in 0..config.max_steps {
        if start_time.elapsed() > max_duration {
            trace.terminated_reason = format!(
                "Timeout ({}s) after {} steps",
                config.timeout_secs, step
            );
            warn!("Chain timeout");
            break;
        }

        info!(step = step, "Tool chain iteration");

        let step_start = Instant::now();

        let response = engine
            .infer(messages.clone(), Some(tools.clone()), temperature, max_tokens)
            .await?;

        let tool_calls = tools::parse_tool_calls(&response);

        if tool_calls.is_empty() {
            trace.terminated_reason = "No more tool calls needed".to_string();
            trace.total_steps = step;
            trace.total_time_ms = start_time.elapsed().as_millis() as u64;

            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.clone(),
            });
            return Ok((response, trace));
        }

        for tc in &tool_calls {
            info!(step = step, tool = %tc.name, "Chain tool call");
        }

        let tool_results_text =
            tools::execute_tool_calls(&tool_calls, skill_registry, None).await;

        let step_elapsed = step_start.elapsed().as_millis() as u64;

        let has_error = tool_results_text.contains("[ERROR]")
            || tool_results_text.contains("Error:");

        let output_summary = if tool_results_text.len() > 200 {
            format!("{}... [{} chars]", &tool_results_text[..200], tool_results_text.len())
        } else {
            tool_results_text.clone()
        };

        for tc in &tool_calls {
            trace.nodes.push(ChainNode {
                step,
                tool_name: tc.name.clone(),
                output_summary: output_summary.clone(),
                is_error: has_error,
                elapsed_ms: step_elapsed,
            });
        }

        if has_error {
            trace.failed_steps += tool_calls.len();

            let error_guidance = format!(
                "Tool(s) reported errors. Step {} encountered issues:\n{}\n\nPlease try a different approach or tool to complete the task.",
                step + 1,
                &output_summary
            );

            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.clone(),
            });
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: format!(
                    "Tool results with errors:\n{}\n\n{}",
                    tool_results_text, error_guidance
                ),
            });
        } else {
            trace.successful_steps += tool_calls.len();

            let step_context = format!(
                "Step {} completed successfully. Tools used: {}. Results available for cross-referencing.",
                step + 1,
                tool_calls.iter().map(|tc| tc.name.as_str()).collect::<Vec<_>>().join(", ")
            );

            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.clone(),
            });
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: format!(
                    "{}\n\nTool results:\n{}",
                    step_context, tool_results_text
                ),
            });
        }

        if step == config.max_steps - 1 {
            trace.terminated_reason = format!("Max steps ({}) reached", config.max_steps);
            warn!("Chain max steps reached");

            messages.push(ChatMessage {
                role: "system".to_string(),
                content: "Maximum tool chain steps reached. Please synthesize your findings now.".to_string(),
            });
        }
    }

    let final_response = engine
        .infer(messages.clone(), None, temperature, max_tokens)
        .await?;

    messages.push(ChatMessage {
        role: "assistant".to_string(),
        content: final_response.clone(),
    });

    if trace.terminated_reason.is_empty() {
        trace.terminated_reason = "Chain completed normally".to_string();
    }

    trace.total_steps = trace.nodes.iter().map(|n| n.step).max().unwrap_or(0) + 1;
    trace.total_time_ms = start_time.elapsed().as_millis() as u64;

    info!(
        steps = trace.total_steps,
        ok = trace.successful_steps,
        err = trace.failed_steps,
        ms = trace.total_time_ms,
        "Tool chain finished"
    );

    Ok((final_response, trace))
}