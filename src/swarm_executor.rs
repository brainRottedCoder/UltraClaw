// ============================================================================
// ULTRACLAW — swarm_executor.rs
// ============================================================================
// Extended swarm execution with actual agent spawning and dependency injection.
//
// This module provides the actual execution capability for sub-agents.
// It wraps the SwarmRegistry with engine/soul dependencies that can be
// injected at runtime.
//
// ARCHITECTURE:
// - SwarmExecutor holds optional Arc references to InferenceEngine and Soul
// - Dependencies are injected via configure() method
// - Actual agent spawning uses the existing SwarmRegistry::spawn which
//   properly executes agents with the provided engine and soul
// ============================================================================

use crate::inference::InferenceEngine;
use crate::soul::Soul;
use crate::skill::{Skill, SkillOutput};
use crate::swarm_skill::{SwarmRegistry, SubAgent, AgentStatus, SwarmStats};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error};

/// Extended SwarmSkill with dependency injection and actual execution
pub struct SwarmExecutor {
    registry: Arc<SwarmRegistry>,
    engine: Arc<RwLock<Option<Arc<dyn InferenceEngine>>>>,
    soul: Arc<RwLock<Option<Arc<Soul>>>>,
}

impl SwarmExecutor {
    /// Create a new swarm executor
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            registry: Arc::new(SwarmRegistry::new(max_concurrent)),
            engine: Arc::new(RwLock::new(None)),
            soul: Arc::new(RwLock::new(None)),
        }
    }

    /// Inject dependencies (engine and soul)
    pub async fn configure(&self, engine: Arc<dyn InferenceEngine>, soul: Arc<Soul>) {
        *self.engine.write().await = Some(engine);
        *self.soul.write().await = Some(soul);
        info!("Swarm executor configured with engine and soul");
    }

    /// Check if executor is configured with dependencies
    pub async fn is_configured(&self) -> bool {
        self.engine.read().await.is_some() && self.soul.read().await.is_some()
    }

    /// Actually spawn and execute a sub-agent
    pub async fn spawn_agent(
        &self,
        role: &str,
        objective: &str,
        context: Option<&str>,
    ) -> Result<String, String> {
        let engine_guard = self.engine.read().await;
        let engine = engine_guard.as_ref().ok_or(
            "SwarmExecutor not configured: engine is None. Call configure() first."
        )?;
        let soul_guard = self.soul.read().await;
        let soul = soul_guard.as_ref().ok_or(
            "SwarmExecutor not configured: soul is None. Call configure() first."
        )?;

        info!(role = %role, objective = %objective, "Spawning sub-agent with actual execution");

        // Use the existing registry.spawn which properly executes the agent
        let agent_id = self.registry.clone().spawn(
            role,
            objective,
            engine.clone(),
            soul.clone(),
        ).await?;

        info!(agent_id = %agent_id, "Sub-agent spawned successfully with execution");

        Ok(agent_id)
    }

    /// Spawn multiple agents in parallel for parallel task execution
    pub async fn spawn_multiple(
        &self,
        tasks: Vec<(&str, &str)>, // Vec of (role, objective)
    ) -> Result<Vec<String>, String> {
        let mut agent_ids = Vec::new();

        for (role, objective) in tasks {
            match self.spawn_agent(role, objective, None).await {
                Ok(id) => agent_ids.push(id),
                Err(e) => {
                    warn!("Failed to spawn agent for {}: {}", role, e);
                    // Continue with other agents
                }
            }
        }

        Ok(agent_ids)
    }

    /// Get the registry for direct access
    pub fn get_registry(&self) -> Arc<SwarmRegistry> {
        self.registry.clone()
    }

    /// Get a specific agent by ID
    pub async fn get_agent(&self, agent_id: &str) -> Option<SubAgent> {
        self.registry.get_agent(agent_id).await
    }

    /// List all agents
    pub async fn list_agents(&self) -> Vec<SubAgent> {
        self.registry.list_agents().await
    }

    /// Cancel a running agent
    pub async fn cancel_agent(&self, agent_id: &str) -> Result<(), String> {
        self.registry.cancel_agent(agent_id).await
    }

    /// Wait for an agent to complete
    pub async fn wait_for_agent(&self, agent_id: &str, timeout_secs: u64) -> Result<String, String> {
        self.registry.wait_for_agent(agent_id, timeout_secs).await
    }

    /// Get swarm statistics
    pub async fn stats(&self) -> SwarmStats {
        self.registry.stats().await
    }

    /// Cleanup old agents
    pub async fn cleanup(&self, max_age_secs: i64) {
        self.registry.cleanup(max_age_secs).await;
    }

    /// Wait for all agents to complete
    pub async fn wait_all(&self, timeout_secs: u64) -> Result<Vec<(String, Result<String, String>)>, String> {
        let agents = self.list_agents().await;
        let mut results = Vec::new();

        for agent in agents {
            match self.wait_for_agent(&agent.id, timeout_secs).await {
                Ok(result) => results.push((agent.id, Ok(result))),
                Err(e) => results.push((agent.id, Err(e))),
            }
        }

        Ok(results)
    }
}

impl Default for SwarmExecutor {
    fn default() -> Self {
        Self::new(4) // Default: 4 concurrent agents
    }
}

// ============================================================================
// SKILL IMPLEMENTATION
// ============================================================================

/// Swarm skill that uses SwarmExecutor for actual execution
pub struct SwarmSkillFull {
    executor: Arc<SwarmExecutor>,
}

impl SwarmSkillFull {
    /// Create a new swarm skill with default settings
    pub fn new() -> Self {
        Self {
            executor: Arc::new(SwarmExecutor::new(4)),
        }
    }

    /// Create with custom max concurrent agents
    pub fn with_max_concurrent(max: usize) -> Self {
        Self {
            executor: Arc::new(SwarmExecutor::new(max)),
        }
    }

    /// Get the executor for configuration
    pub fn get_executor(&self) -> Arc<SwarmExecutor> {
        self.executor.clone()
    }

    /// Configure with dependencies
    pub fn configure(&self, engine: Arc<dyn InferenceEngine>, soul: Arc<Soul>) {
        let executor = self.executor.clone();
        tokio::spawn(async move {
            executor.configure(engine, soul).await;
        });
    }
}

impl Default for SwarmSkillFull {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for SwarmSkillFull {
    fn name(&self) -> &'static str {
        "spawn_sub_agent"
    }

    fn description(&self) -> &'static str {
        "Delegate a task to a specialized sub-agent that runs in parallel. \
         Sub-agents can handle independent tasks like analyzing code, researching \
         topics, reviewing content, or documenting findings. Use this for parallel \
         execution when multiple independent tasks need to be processed simultaneously. \
         Available roles: Analyst, Coder, Researcher, Reviewer, Documenter, Assistant. \
         Use action=spawn to create agents, action=status to check progress."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Swarm action to perform",
                    "enum": ["spawn", "status", "list", "cancel", "wait", "stats", "cleanup"]
                },
                "objective": {
                    "type": "string",
                    "description": "The specific objective for the sub-agent to accomplish"
                },
                "role": {
                    "type": "string",
                    "description": "The persona/role for the sub-agent",
                    "enum": ["Analyst", "Coder", "Researcher", "Reviewer", "Documenter", "Assistant"],
                    "default": "Assistant"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Agent ID (for status, cancel, wait actions)"
                },
                "context": {
                    "type": "string",
                    "description": "Additional context or data to provide to the agent"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout for wait action in seconds",
                    "default": 60
                },
                "max_age_secs": {
                    "type": "integer",
                    "description": "Max age in seconds for cleanup action",
                    "default": 3600
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("spawn").to_string();
        let executor = self.executor.clone();
        let args_json = args.to_string();

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "spawn_sub_agent".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let args_owned: Value = serde_json::from_str(&args_json).map_err(|e| e.to_string())?;
                execute_swarm_full_action(&executor, &action, &args_owned).await
            })
        })
        .join()
        .unwrap_or_else(|_| Err("Swarm operation panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "spawn_sub_agent".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "spawn_sub_agent".to_string(),
                output: format!("Swarm error: {}", e),
                is_error: true,
            },
        }
    }
}

/// Execute swarm action with full execution capability
async fn execute_swarm_full_action(
    executor: &Arc<SwarmExecutor>,
    action: &str,
    args: &Value,
) -> Result<String, String> {
    match action {
        "spawn" => {
            let objective = args.get("objective").and_then(|v| v.as_str())
                .ok_or("Objective is required for spawn action")?;
            let role = args.get("role").and_then(|v| v.as_str()).unwrap_or("Assistant");
            let context = args.get("context").and_then(|v| v.as_str());

            // Check if configured
            if !executor.is_configured().await {
                return Err(
                    "Swarm executor not configured with engine and soul. \
                     The system must call configure() before spawning agents.".to_string()
                );
            }

            // Actually spawn the agent
            match executor.spawn_agent(role, objective, context).await {
                Ok(agent_id) => {
                    info!(agent_id = %agent_id, role = %role, "Agent spawned and executing");
                    Ok(serde_json::json!({
                        "status": "spawned",
                        "agent_id": agent_id,
                        "role": role,
                        "objective": objective,
                        "message": format!(
                            "Sub-agent '{}' spawned with role '{}' and is now executing. \
                             Use action=status with agent_id to check progress, \
                             or action=wait to wait for completion.",
                            agent_id, role
                        )
                    }).to_string())
                }
                Err(e) => Err(e),
            }
        }

        "status" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str())
                .ok_or("Agent ID is required for status action")?;

            let agent = executor.get_agent(agent_id).await
                .ok_or_else(|| format!("Agent {} not found", agent_id))?;

            let status_text: String = match agent.status {
                AgentStatus::Pending => "Agent is pending and will start soon".to_string(),
                AgentStatus::Running => "Agent is currently executing".to_string(),
                AgentStatus::Completed => "Agent has completed successfully".to_string(),
                AgentStatus::Failed => format!("Agent failed: {}", agent.error.as_deref().unwrap_or("Unknown error")),
                AgentStatus::Cancelled => "Agent was cancelled".to_string(),
            };

            Ok(serde_json::json!({
                "agent_id": agent.id,
                "role": agent.role,
                "objective": agent.objective,
                "status": format!("{:?}", agent.status),
                "status_text": status_text,
                "result": agent.result,
                "error": agent.error,
                "created_at": agent.created_at,
                "completed_at": agent.completed_at
            }).to_string())
        }

        "list" => {
            let agents = executor.list_agents().await;
            
            let agent_summaries: Vec<Value> = agents.iter().map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "role": a.role,
                    "status": format!("{:?}", a.status),
                    "objective": a.objective,
                    "has_result": a.result.is_some(),
                    "created_at": a.created_at
                })
            }).collect();

            let stats = executor.stats().await;

            Ok(serde_json::json!({
                "total_agents": agents.len(),
                "stats": {
                    "pending": stats.pending,
                    "running": stats.running,
                    "completed": stats.completed,
                    "failed": stats.failed,
                    "cancelled": stats.cancelled
                },
                "agents": agent_summaries
            }).to_string())
        }

        "stats" => {
            let stats = executor.stats().await;
            Ok(serde_json::json!({
                "total": stats.total,
                "pending": stats.pending,
                "running": stats.running,
                "completed": stats.completed,
                "failed": stats.failed,
                "cancelled": stats.cancelled,
                "max_concurrent": 4
            }).to_string())
        }

        "cancel" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str())
                .ok_or("Agent ID is required for cancel action")?;

            executor.cancel_agent(agent_id).await?;
            Ok(format!("Agent {} has been cancelled", agent_id))
        }

        "cleanup" => {
            let max_age = args.get("max_age_secs").and_then(|v| v.as_i64()).unwrap_or(3600);
            executor.cleanup(max_age).await;
            Ok(format!("Cleaned up agents older than {} seconds", max_age))
        }

        "wait" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str())
                .ok_or("Agent ID is required for wait action")?;
            let timeout = args.get("timeout_secs").and_then(|v| v.as_i64()).unwrap_or(60) as u64;

            match executor.wait_for_agent(agent_id, timeout).await {
                Ok(result) => {
                    let agent = executor.get_agent(agent_id).await;
                    let role = agent.as_ref().map(|a| a.role.as_str()).unwrap_or("Unknown");
                    Ok(format!(
                        "Agent {} ({}) completed successfully:\n\n{}",
                        agent_id, role, result
                    ))
                }
                Err(e) => Err(e),
            }
        }

        _ => Err(format!(
            "Unknown action: '{}'. Available: spawn, status, list, cancel, wait, stats, cleanup",
            action
        )),
    }
}