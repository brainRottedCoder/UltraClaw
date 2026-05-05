use crate::db::ChatMessage;
use crate::inference::InferenceEngine;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{info, warn};

const MAX_REFINEMENT_CYCLES: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinementAttempt {
    pub cycle: usize,
    pub original_error: String,
    pub model_diagnosis: String,
    pub improved_approach: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RefinementHistory {
    pub attempts: Vec<RefinementAttempt>,
    pub final_success: bool,
    pub cycles_used: usize,
    pub total_time_ms: u64,
}

impl RefinementHistory {
    pub fn summary(&self) -> String {
        format!(
            "Prompt Refine: {} cycles, {} attempts logged, {}",
            self.cycles_used,
            self.attempts.len(),
            if self.final_success { "SUCCESS" } else { "FAILED" }
        )
    }

    pub fn comparison_table(&self) -> String {
        let mut table = String::from("| Cycle | Error | Diagnosis | Improved Approach |\n");
        table.push_str("|-------|-------|-----------|--------------------|\n");

        for attempt in &self.attempts {
            table.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                attempt.cycle + 1,
                truncate(&attempt.original_error, 40),
                truncate(&attempt.model_diagnosis, 40),
                truncate(&attempt.improved_approach, 40),
            ));
        }

        table
    }
}

pub async fn refine_and_retry(
    engine: &dyn InferenceEngine,
    original_messages: Vec<ChatMessage>,
    error_context: &str,
    max_cycles: Option<usize>,
) -> Result<(String, RefinementHistory), String> {
    let start_time = Instant::now();
    let max_cycles = max_cycles.unwrap_or(MAX_REFINEMENT_CYCLES);
    let mut history = RefinementHistory {
        attempts: Vec::new(),
        final_success: false,
        cycles_used: 0,
        total_time_ms: 0,
    };

    let mut current_prompt = String::new();
    if let Some(first_msg) = original_messages.first() {
        current_prompt = first_msg.content.clone();
    }

    for cycle in 0..max_cycles {
        history.cycles_used = cycle + 1;

        let diagnosis_prompt = format!(
            r#"Your previous attempt failed with this error:

Previous approach: "{}"

Error: {}

Please honestly diagnose WHY you failed. Consider:
1. Did you misunderstand the task?
2. Did you choose the wrong tool?
3. Did you use incorrect arguments?
4. Was there missing context?

Output your diagnosis as a concise paragraph. Be self-critical."#,
            truncate(&current_prompt, 200),
            error_context,
        );

        let diag_msgs = vec![ChatMessage {
            role: "user".to_string(),
            content: diagnosis_prompt,
        }];

        let diagnosis = engine
            .infer(diag_msgs, None, 0.3, 512)
            .await
            .unwrap_or_else(|e| format!("Could not self-diagnose: {}", e));

        let strategy_prompt = format!(
            r#"Based on your diagnosis:
"{}"

Now write an IMPROVED approach. What would you do differently?
Be specific about:
1. Which tool to use (and correct arguments)
2. What to check before acting
3. The step-by-step approach
4. Fallback if it fails again

Write this as instructions you will follow. Keep under 200 words."#,
            diagnosis
        );

        let strat_msgs = vec![ChatMessage {
            role: "user".to_string(),
            content: strategy_prompt,
        }];

        let improved = engine
            .infer(strat_msgs, None, 0.3, 512)
            .await
            .unwrap_or_else(|e| format!("Could not generate strategy: {}", e));

        let attempt = RefinementAttempt {
            cycle,
            original_error: error_context.to_string(),
            model_diagnosis: diagnosis.clone(),
            improved_approach: improved.clone(),
        };

        history.attempts.push(attempt);

        info!(
            cycle = cycle,
            "Prompt refinement: model diagnosed and improved"
        );

        let retry_prompt = format!(
            r#"SYSTEM: {}"#,
            improved
        );

        let mut retry_msgs = vec![ChatMessage {
            role: "system".to_string(),
            content: retry_prompt,
        }];

        for msg in original_messages.iter().skip(1) {
            retry_msgs.push(msg.clone());
        }

        let retry_messages = retry_msgs.clone();

        let result = engine
            .infer(retry_messages, None, 0.2, 1024)
            .await;

        match result {
            Ok(response) => {
                let error_in_response = response.contains("Error:")
                    || response.contains("failed")
                    || response.contains("cannot");

                if !error_in_response {
                    history.final_success = true;
                    history.total_time_ms = start_time.elapsed().as_millis() as u64;

                    let improved_response = format!(
                        "{}\n\n[Self-improved after {} refinement cycles]",
                        response, cycle + 1
                    );

                    info!(
                        cycle = cycle,
                        ms = history.total_time_ms,
                        "Prompt refinement succeeded"
                    );
                    return Ok((improved_response, history));
                }

                current_prompt = improved.clone();

                let new_error = format!(
                    "Retry generated a response containing error indicators. Response excerpt: {}",
                    truncate(&response, 150)
                );
                warn!(cycle = cycle, "Refinement retry had errors, continuing");
            }
            Err(e) => {
                warn!(cycle = cycle, "Refinement retry failed: {}", e);
            }
        }
    }

    history.total_time_ms = start_time.elapsed().as_millis() as u64;

    let exhausted_msg = format!(
        "Task could not be completed after {} refinement cycles. The model identified:\n{}",
        history.cycles_used,
        history
            .attempts
            .iter()
            .map(|a| format!("- Cycle {}: {}", a.cycle + 1, truncate(&a.model_diagnosis, 80)))
            .collect::<Vec<_>>()
            .join("\n")
    );

    Ok((exhausted_msg, history))
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}