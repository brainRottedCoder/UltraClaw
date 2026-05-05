use crate::skill::{Skill, SkillOutput, MAX_OUTPUT_BYTES};
use serde_json::Value;

/// Sandboxed Shell Command execution using Docker.
/// This executes commands safely within an isolated container.
pub struct SandboxCommandSkill;

impl Skill for SandboxCommandSkill {
    fn name(&self) -> &'static str {
        "run_command"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command inside a secure Docker sandbox. Use this instead of direct host execution."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute in the sandbox"
                }
            },
            "required": ["command"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let command_str = args.get("command").and_then(|v| v.as_str()).unwrap_or("");

        // Execute via Docker for isolation
        let output = std::process::Command::new("docker")
            .args([
                "run", "--rm", 
                "--network", "none", // Prevent network access
                "--memory", "256m",  // Limit memory
                "alpine", "sh", "-c", command_str
            ])
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr)
                };

                let truncated = if combined.len() > MAX_OUTPUT_BYTES {
                    format!("{}...\n[TRUNCATED]", &combined[..MAX_OUTPUT_BYTES])
                } else {
                    combined
                };

                SkillOutput {
                    name: self.name().to_string(),
                    output: truncated,
                    is_error: !out.status.success(),
                }
            }
            Err(e) => SkillOutput {
                name: self.name().to_string(),
                output: format!("Error executing sandboxed command: {}", e),
                is_error: true,
            },
        }
    }
}
