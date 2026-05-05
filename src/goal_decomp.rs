use crate::db::ChatMessage;
use crate::inference::InferenceEngine;
use crate::skill::{SkillRegistry, ToolCall};
use crate::tools;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalStep {
    pub id: String,
    pub depends_on: Vec<String>,
    pub tool_name: String,
    pub arguments: Value,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalPlan {
    pub overall_goal: String,
    pub steps: Vec<GoalStep>,
    pub final_validation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecompMetrics {
    pub total_steps: usize,
    pub completed: usize,
    pub failed: usize,
    pub plan_valid: bool,
    pub dependency_graph_valid: bool,
}

impl DecompMetrics {
    pub fn summary(&self) -> String {
        format!(
            "Goal Decomp: {}/{} steps done, {} failed, valid plan: {}",
            self.completed, self.total_steps, self.failed, self.plan_valid
        )
    }
}

pub async fn decompose_and_execute(
    engine: &dyn InferenceEngine,
    skill_registry: &SkillRegistry,
    user_message: &str,
    temperature: f32,
    max_tokens: u32,
) -> Result<(String, DecompMetrics), String> {
    let start_time = Instant::now();
    let mut metrics = DecompMetrics {
        total_steps: 0,
        completed: 0,
        failed: 0,
        plan_valid: false,
        dependency_graph_valid: false,
    };

    let tool_schema = skill_registry.to_tool_schema();
    let tool_names: Vec<String> = {
        if let Value::Array(arr) = &tool_schema {
            arr.iter()
                .filter_map(|t| {
                    t.get("function")?
                        .get("name")?
                        .as_str()
                        .map(|s| s.to_string())
                })
                .collect()
        } else {
            Vec::new()
        }
    };

    let decomp_prompt = format!(
        r#"You are a task planner. Break the user's request into discrete, executable steps.

Available tools: {}

User request: "{}"

Output a JSON plan with this exact structure:
```json
{{
  "overall_goal": "one sentence describing the goal",
  "steps": [
    {{
      "id": "step_1",
      "depends_on": [],
      "tool_name": "read_file",
      "arguments": {{"path": "Cargo.toml"}},
      "description": "Read the Cargo.toml file"
    }}
  ],
  "final_validation": "how to verify success"
}}
```

Rules:
1. Each step MUST use one of the available tools
2. depended_on lists step IDs that must complete first
3. Keep steps concrete - no 'analyze' without a tool
4. 3-6 steps maximum
5. First steps should gather information, later steps process it
6. Output ONLY the JSON plan, no extra text"#,
        tool_names.join(", "),
        user_message
    );

    let plan_messages = vec![ChatMessage {
        role: "user".to_string(),
        content: decomp_prompt,
    }];

    let plan_response = engine
        .infer(plan_messages.clone(), None, 0.1, 2048)
        .await
        .map_err(|e| format!("Plan generation failed: {}", e))?;

    let plan: GoalPlan = match extract_plan_json(&plan_response) {
        Ok(p) => {
            metrics.plan_valid = true;
            metrics.total_steps = p.steps.len();
            p
        }
        Err(e) => {
            warn!("Goal decomposition failed: {}", e);

            let fallback_plan = GoalPlan {
                overall_goal: user_message.to_string(),
                steps: vec![GoalStep {
                    id: "step_1".to_string(),
                    depends_on: vec![],
                    tool_name: "list_directory".to_string(),
                    arguments: serde_json::json!({"path": "."}),
                    description: "List current directory".to_string(),
                }],
                final_validation: "Output should contain file listing".to_string(),
            };
            metrics.total_steps = 1;
            fallback_plan
        }
    };

    if let Err(e) = validate_dependency_graph(&plan.steps) {
        warn!("Dependency graph invalid: {}", e);
    } else {
        metrics.dependency_graph_valid = true;
    }

    let exec_order = topological_sort(&plan.steps);
    let mut step_outputs: HashMap<String, String> = HashMap::new();
    let mut step_summaries: Vec<String> = Vec::new();

    for step in &exec_order {
        info!(
            step_id = %step.id,
            tool = %step.tool_name,
            "Executing goal step"
        );

        let tool_call = ToolCall {
            name: step.tool_name.clone(),
            arguments: step.arguments.clone(),
        };

        let output = tools::execute_tool_calls(&[tool_call], skill_registry, None).await;
        step_outputs.insert(step.id.clone(), output.clone());

        let is_error = output.contains("[ERROR]")
            || output.contains("Error:")
            || output.contains("failed");

        if is_error {
            metrics.failed += 1;
            warn!(step_id = %step.id, "Step failed");
        } else {
            metrics.completed += 1;
            info!(step_id = %step.id, "Step completed");
        }

        step_summaries.push(format!(
            "Step '{}' ({}) {}:\n{}",
            step.id,
            step.description,
            if is_error { "FAILED" } else { "OK" },
            if output.len() > 500 {
                format!("{}... [truncated]", &output[..500])
            } else {
                output.clone()
            }
        ));
    }

    let synthesis_prompt = format!(
        r#"You executed a multi-step plan to: "{}"

Step results:
{}

Synthesize a clear, concise response summarizing what was accomplished.
Include any failures and what they mean. Be factual."#,
        plan.overall_goal,
        step_summaries.join("\n\n")
    );

    let synthesis_messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: "You are a helpful assistant synthesizing task results.".to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: synthesis_prompt,
        },
    ];

    let final_response = engine
        .infer(synthesis_messages, None, temperature, max_tokens)
        .await
        .unwrap_or_else(|e| format!("Synthesis failed: {}", e));

    let elapsed = start_time.elapsed();
    info!(
        elapsed_ms = elapsed.as_millis(),
        steps = metrics.total_steps,
        completed = metrics.completed,
        failed = metrics.failed,
        "Goal decomposition complete"
    );

    Ok((final_response, metrics))
}

fn extract_plan_json(response: &str) -> Result<GoalPlan, String> {
    let json_str = if let Some(start) = response.find("```json") {
        let content_start = response[start..]
            .find('\n')
            .map(|p| start + p + 1)
            .unwrap_or(start + 7);
        if let Some(end) = response[content_start..].find("```") {
            &response[content_start..content_start + end]
        } else {
            &response[content_start..]
        }
    } else if response.trim().starts_with('{') {
        response
    } else if let Some(start) = response.find('{') {
        if let Some(end) = response.rfind('}') {
            &response[start..=end]
        } else {
            return Err("No JSON object found in plan response".to_string());
        }
    } else {
        return Err("No JSON found in plan response".to_string());
    };

    serde_json::from_str::<GoalPlan>(json_str.trim())
        .map_err(|e| format!("Failed to parse plan JSON: {} | Raw: {}", e, &json_str[..200.min(json_str.len())]))
}

fn validate_dependency_graph(steps: &[GoalStep]) -> Result<(), String> {
    let step_ids: HashSet<&str> = steps.iter().map(|s| s.id.as_str()).collect();

    for step in steps {
        for dep in &step.depends_on {
            if !step_ids.contains(dep.as_str()) {
                return Err(format!(
                    "Step '{}' depends on unknown step '{}'",
                    step.id, dep
                ));
            }
            if *dep == step.id {
                return Err(format!("Step '{}' depends on itself", step.id));
            }
        }
    }

    let order = topological_sort(steps);
    if order.len() != steps.len() {
        return Err("Circular dependency detected in plan".to_string());
    }

    Ok(())
}

fn topological_sort(steps: &[GoalStep]) -> Vec<GoalStep> {
    let mut result = Vec::new();
    let mut completed: HashSet<&str> = HashSet::new();
    let mut remaining: Vec<&GoalStep> = steps.iter().collect();

    let mut iterations = 0;
    let max_iterations = steps.len() * 2;

    while !remaining.is_empty() && iterations < max_iterations {
        iterations += 1;

        let mut ready = Vec::new();
        let mut not_ready = Vec::new();

        for step in remaining {
            if step
                .depends_on
                .iter()
                .all(|d| completed.contains(d.as_str()))
            {
                ready.push(step);
            } else {
                not_ready.push(step);
            }
        }

        if ready.is_empty() {
            for step in &not_ready {
                warn!(
                    step_id = %step.id,
                    depends_on = ?step.depends_on,
                    "Step has unresolved dependencies, executing anyway"
                );
            }
            for step in not_ready {
                result.push(step.clone());
                completed.insert(&step.id);
            }
            break;
        }

        for step in &ready {
            completed.insert(&step.id);
        }

        result.extend(ready.iter().map(|s| (**s).clone()));
        remaining = not_ready;
    }

    result
}