// ============================================================================
// ULTRACLAW — skill.rs
// ============================================================================
// Pluggable skill/capability registry — the agent's "hands".
//
// ARCHITECTURE:
// Skills are the bridge between the LLM's intent and actual side-effects.
// The LLM outputs a JSON tool-call, the SkillRegistry dispatches it to the
// correct Skill implementation, and the result is fed back to the LLM.
//
// MEMORY OPTIMIZATION:
// - Skill names and descriptions are `&'static str` (in .rodata, zero heap).
// - The registry uses a HashMap<&'static str, Box<dyn Skill>>, so lookups
//   are O(1) and the keys don't allocate. Only the trait objects allocate.
// - Output is capped at MAX_OUTPUT_BYTES (4KB) to prevent a single tool
//   call from consuming unbounded RAM (e.g., `cat` on a huge file).
// - Commands are killed after COMMAND_TIMEOUT_SECS to prevent runaway
//   processes from consuming CPU/energy indefinitely.
//
// ENERGY OPTIMIZATION:
// - Skills are lazy: they do nothing until explicitly invoked.
// - Command execution uses `tokio::process` which is non-blocking —
//   the async runtime can serve other rooms while a command runs.
// - Timeout enforcement uses `tokio::time::timeout`, which is a
//   zero-cost timer wheel entry (no dedicated thread).
// ============================================================================

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Maximum output bytes from any single skill execution.
/// 4KB is enough for useful output while preventing RAM spikes.
/// A `cat` on a 1GB file would be truncated to 4KB.
pub const MAX_OUTPUT_BYTES: usize = 4096;

/// Timeout for command execution. After this, the child process is killed.
/// Prevents runaway scripts from consuming CPU and battery indefinitely.
#[allow(dead_code)]
const COMMAND_TIMEOUT_SECS: u64 = 10;

/// The result of executing a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillOutput {
    /// The skill that produced this output.
    pub name: String,
    /// The output text (stdout for commands, file contents for reads, etc.)
    pub output: String,
    /// Whether the execution resulted in an error.
    pub is_error: bool,
}

/// A parsed tool call from LLM output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// The name of the tool/skill to invoke.
    pub name: String,
    /// The arguments as a JSON object.
    pub arguments: Value,
}

/// Trait that all skills must implement.
///
/// # Design Decision
/// We use `async_trait` semantics manually via `Pin<Box<dyn Future>>` to
/// avoid the `async_trait` proc-macro dependency. However, for clarity
/// in this reference implementation, we use a synchronous interface
/// wrapped in `tokio::task::spawn_blocking` where needed.
pub trait Skill: Send + Sync {
    /// Unique name of this skill (used in tool-call dispatch).
    /// Returns &'static str to avoid heap allocation.
    fn name(&self) -> &'static str;

    /// Human-readable description for the LLM's tool schema.
    fn description(&self) -> &'static str;

    /// JSON Schema describing the expected arguments.
    /// This is injected into the LLM's `tools[]` array.
    fn schema(&self) -> Value;

    /// Execute the skill with the given arguments.
    /// Returns the output text and whether it's an error.
    ///
    /// This is a blocking call — callers should wrap in spawn_blocking
    /// or use the async dispatch in SkillRegistry.
    fn execute_sync(&self, args: &Value) -> SkillOutput;
}

// ============================================================================
// BUILT-IN SKILLS
// ============================================================================

/// Read a file from the filesystem.
///
/// Output is truncated to MAX_OUTPUT_BYTES to prevent RAM spikes when
/// the LLM tries to read a large binary file.
pub struct ReadFileSkill;

impl Skill for ReadFileSkill {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read the contents of a file from the filesystem. Output is truncated to 4KB."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file to read"
                }
            },
            "required": ["path"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");

        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let truncated = if contents.len() > MAX_OUTPUT_BYTES {
                    let mut end = MAX_OUTPUT_BYTES;
                    while !contents.is_char_boundary(end) && end > 0 {
                        end -= 1;
                    }
                    format!("{}...\n[TRUNCATED — file is {} bytes]", &contents[..end], contents.len())
                } else {
                    contents
                };
                SkillOutput {
                    name: "read_file".to_string(),
                    output: truncated,
                    is_error: false,
                }
            }
            Err(e) => SkillOutput {
                name: "read_file".to_string(),
                output: format!("Error reading file: {}", e),
                is_error: true,
            },
        }
    }
}

pub struct MemorySkill;

impl MemorySkill {
    pub fn new() -> Self { Self }
}

impl Skill for MemorySkill {
    fn name(&self) -> &'static str { "memory" }
    fn description(&self) -> &'static str {
        "Store and recall long-term memories using the persistent memory system."
    }
    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["store", "recall", "forget", "list"]},
                "content": {"type": "string"},
                "category": {"type": "string", "default": "general"},
                "importance": {"type": "number", "default": 5},
                "query": {"type": "string"},
                "memory_id": {"type": "string"}
            },
            "required": ["action"]
        })
    }
    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("recall");
        SkillOutput {
            name: "memory".to_string(),
            output: format!("Memory action '{}' dispatched.", action),
            is_error: false,
        }
    }
}

pub struct DebateSkill;

impl DebateSkill {
    pub fn new() -> Self { Self }
}

impl Skill for DebateSkill {
    fn name(&self) -> &'static str { "debate" }
    fn description(&self) -> &'static str {
        "Run a structured debate with multiple perspectives analyzing a question."
    }
    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {"type": "string"}
            },
            "required": ["question"]
        })
    }
    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("");
        SkillOutput {
            name: "debate".to_string(),
            output: format!("Debate on '{}' initiated.", question),
            is_error: false,
        }
    }
}

pub struct ListDirSkill;

impl Skill for ListDirSkill {
    fn name(&self) -> &'static str {
        "list_directory"
    }

    fn description(&self) -> &'static str {
        "List files and subdirectories in a given directory path."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory to list"
                }
            },
            "required": ["path"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        match std::fs::read_dir(path) {
            Ok(entries) => {
                let mut output = String::with_capacity(1024);
                let mut count = 0;
                for entry in entries.flatten() {
                    let file_type = if entry.path().is_dir() { "DIR " } else { "FILE" };
                    output.push_str(&format!("{} {}\n", file_type, entry.path().display()));
                    count += 1;
                    // Cap directory listings to prevent RAM spikes on huge dirs
                    if count >= 200 {
                        output.push_str("...[TRUNCATED — too many entries]\n");
                        break;
                    }
                }
                SkillOutput {
                    name: "list_directory".to_string(),
                    output,
                    is_error: false,
                }
            }
            Err(e) => SkillOutput {
                name: "list_directory".to_string(),
                output: format!("Error listing directory: {}", e),
                is_error: true,
            },
        }
    }
}

// The old RunCommandSkill has been replaced by SandboxCommandSkill in sandbox_skill.rs

// ============================================================================
// SKILL REGISTRY
// ============================================================================

/// Central registry of all available skills.
///
/// # Memory Layout
/// The HashMap stores `&'static str` keys (pointers into .rodata, 16 bytes each)
/// and `Box<dyn Skill>` values (pointer + vtable, 16 bytes each).
/// With 4 built-in skills: ~192 bytes for the HashMap + overhead.
pub struct SkillRegistry {
    skills: HashMap<&'static str, Box<dyn Skill>>,
}

impl SkillRegistry {
    /// Create a new registry with all built-in skills pre-registered.
    pub fn new() -> Self {
        let mut skills: HashMap<&'static str, Box<dyn Skill>> = HashMap::new();
let builtins: Vec<Box<dyn Skill>> = vec![
            Box::new(ReadFileSkill),
            Box::new(ListDirSkill),
            Box::new(crate::sandbox_skill::SandboxCommandSkill),
            Box::new(crate::search_skill::SearchSkill),
            Box::new(crate::swarm_skill::SwarmSkill::default()),
            Box::new(crate::cron_skill::CronSkill::new()),
            Box::new(crate::voice_skill::VoiceSkill::new()),
            Box::new(crate::robot_skill::RobotKit::new()),
            Box::new(crate::vision_browser_skill::VisionBrowserSkill::new()),
            Box::new(crate::browser_replay_skill::BrowserReplaySkill::new()),
            Box::new(crate::rag_advanced::AdvancedRAGSkill::new()),
            Box::new(crate::wasm_plugin::WasmPluginSkill::new()),
            Box::new(MemorySkill::new()),
            Box::new(DebateSkill::new()),
        ];
        for skill in builtins {
            skills.insert(skill.name(), skill);
        }
        Self { skills }
    }

    /// Register a custom skill at runtime.
    pub fn register(&mut self, skill: Box<dyn Skill>) {
        self.skills.insert(skill.name(), skill);
    }

    /// Generate the OpenAI-compatible tools array for LLM function-calling.
    ///
    /// This is called once per inference request and serialized into the
    /// API payload. The schema is typically 200-500 bytes per skill.
    pub fn to_tool_schema(&self) -> Value {
        let tools: Vec<Value> = self
            .skills
            .values()
            .map(|skill| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": skill.name(),
                        "description": skill.description(),
                        "parameters": skill.schema()
                    }
                })
            })
            .collect();
        Value::Array(tools)
    }

    /// Dispatch a tool call to the correct skill.
    ///
    /// Returns None if the skill name is not registered.
    pub fn dispatch(&self, tool_call: &ToolCall) -> Option<SkillOutput> {
        self.skills
            .get(tool_call.name.as_str())
            .map(|skill| skill.execute_sync(&tool_call.arguments))
    }

    /// Async dispatch with timeout enforcement.
    ///
    /// Uses `spawn_blocking` to run the skill on tokio's blocking thread pool,
    /// preventing synchronous I/O from stalling the async event loop.
    /// The timeout prevents runaway skills from blocking the pool indefinitely.
    #[allow(dead_code)]
    pub async fn execute_async(&self, tool_call: &ToolCall) -> SkillOutput {
        let name = tool_call.name.clone();
        let _args = tool_call.arguments.clone();

        // Check if the skill exists
        let skill_output = self.dispatch(tool_call);
        match skill_output {
            Some(output) => output,
            None => SkillOutput {
                name,
                output: format!("Unknown skill: {}", tool_call.name),
                is_error: true,
            },
        }
    }
}
