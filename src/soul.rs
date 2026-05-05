// ============================================================================
// ULTRACLAW — soul.rs
// ============================================================================
// The Soul module defines the agent's identity, personality, and behavioral
// directives. Think of it as the "system prompt compiler" — it builds the
// system message that shapes every LLM response.
//
// MEMORY OPTIMIZATION:
// - The default persona and directives use `&'static str` wherever possible.
//   These point directly into the binary's read-only data segment (.rodata),
//   consuming ZERO heap memory. The compiler bakes them into the executable.
// - `Cow<'static, str>` is used for fields that might be static OR dynamic.
//   When static, it's just a pointer. When dynamic (user override), it owns
//   the heap allocation. This avoids unnecessary cloning of the default persona.
//
// ENERGY OPTIMIZATION:
// - `build_system_message()` is called once per inference request, not once
//   per token. The resulting string is passed by reference to the inference
//   engine, so we avoid re-computing it during token generation.
// ============================================================================

use std::borrow::Cow;

/// A behavioral directive that modifies the agent's personality.
///
/// Directives are appended to the system message in order. They can be
/// static (compiled into the binary) or dynamic (loaded at runtime).
#[derive(Debug, Clone)]
pub struct Directive {
    /// A short label for logging (e.g., "brevity", "safety")
    #[allow(dead_code)]
    pub label: &'static str,
    /// The actual instruction text injected into the system prompt
    pub instruction: Cow<'static, str>,
    /// Priority (higher = injected earlier in the prompt). Default behavior
    /// directives are priority 0; safety directives should be 100+.
    #[allow(dead_code)]
    pub priority: u8,
}

/// The Soul of the Ultraclaw agent.
///
/// # Memory Layout
/// - `name`: 24 bytes (String header; actual chars in heap or .rodata)
/// - `persona`: 16 or 24 bytes (Cow — either a fat pointer or String)
/// - `directives`: 24 bytes (Vec header) + N * ~56 bytes per directive
/// - `temperature`, `max_tokens`: 4 bytes each
///
/// Total base: ~72 bytes on stack. With 10 directives: ~632 bytes heap.
/// This is negligible — the system message string itself dominates.
#[derive(Debug, Clone)]
pub struct Soul {
    /// The agent's display name.
    pub name: String,
    /// The core persona / system prompt body.
    /// Uses `Cow` so the default persona lives in .rodata (zero heap).
    pub persona: Cow<'static, str>,
    /// Ordered list of behavioral directives.
    pub directives: Vec<Directive>,
    /// LLM temperature (0.0 = deterministic, 1.0 = creative).
    pub temperature: f32,
    /// Maximum tokens for LLM response generation.
    pub max_tokens: u32,
}

/// Default Ultraclaw persona — lives entirely in the binary's .rodata segment.
/// Zero heap allocation for the default case.
const DEFAULT_PERSONA: &str = "\
You are Ultraclaw, a hyper-efficient autonomous AI assistant. \
You communicate across multiple platforms simultaneously (WhatsApp, Telegram, \
Discord, Slack, iMessage, SMS, and more) through a unified interface. \
You are concise, precise, and adapt your formatting to each platform's capabilities. \
You can execute tools (read files, run commands, search) when needed. \
You maintain separate conversation contexts per platform and per room. \
You never leak information between conversations on different platforms.";

/// Default behavioral directives. All `&'static str` — zero heap.
const DEFAULT_DIRECTIVES: &[(&str, &str, u8)] = &[
    // (label, instruction, priority)
    (
        "conciseness",
        "Keep responses concise. Prefer short, actionable answers over verbose explanations unless the user explicitly asks for detail.",
        10,
    ),
    (
        "platform_awareness",
        "Adapt your formatting to the platform. Use markdown on Discord/Slack. Use plain text on SMS/iMessage/WhatsApp. Never send code blocks to platforms that don't render them.",
        20,
    ),
    (
        "tool_discipline",
        "Only invoke tools when the user's request genuinely requires file access, code execution, or external data. Do not invoke tools speculatively.",
        30,
    ),
    (
        "memory_hygiene",
        "When you learn important user preferences or facts, store them in long-term memory. When a user asks you to forget something, comply immediately.",
        40,
    ),
    (
        "safety",
        "Never execute destructive commands (rm -rf, format, drop table) without explicit user confirmation. Never reveal API keys, passwords, or other secrets.",
        100,
    ),
];

impl Soul {
    /// Create the default Ultraclaw soul.
    ///
    /// This is extremely cheap — all strings point to .rodata, only the
    /// Vec<Directive> header (24 bytes) and the directive entries are allocated.
    pub fn default_soul() -> Self {
        let directives = DEFAULT_DIRECTIVES
            .iter()
            .map(|(label, instruction, priority)| Directive {
                label,
                instruction: Cow::Borrowed(instruction),
                priority: *priority,
            })
            .collect();

        Self {
            name: "Ultraclaw".to_string(),
            persona: Cow::Borrowed(DEFAULT_PERSONA),
            directives,
            temperature: 0.3, // Low temp for reliable, consistent responses
            max_tokens: 2048,
        }
    }

    /// Add a runtime directive (e.g., per-platform overrides).
    ///
    /// the Soul. Used for contextual overrides like "be extra brief for SMS".
    #[allow(dead_code)]
    pub fn apply_directive(&mut self, label: &'static str, instruction: String, priority: u8) {
        self.directives.push(Directive {
            label,
            instruction: Cow::Owned(instruction),
            priority,
        });
        // Sort by priority descending so high-priority directives come first
        // in the system prompt. Sorting a small vec (<20 items) is ~nanoseconds.
        self.directives.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Compile the full system message for LLM consumption.
    ///
    /// This concatenates the persona + all directives into a single string.
    /// The result is typically 500-2000 chars (~100-500 tokens), which is
    /// small relative to the context window.
    ///
    /// # Arguments
    /// * `platform_context` - Optional string describing the current platform
    ///   (e.g., "User is on WhatsApp via mautrix bridge"). Injected at the end.
    /// * `session_context` - Optional session metadata (turn count, duration).
    /// * `memory_context` - Optional long-term memory recall for this room.
    pub fn build_system_message(
        &self,
        platform_context: Option<&str>,
        session_context: Option<&str>,
        memory_context: Option<&str>,
    ) -> String {
        // Pre-calculate capacity to avoid reallocations.
        // A single allocation for the final string is optimal.
        let mut capacity = self.persona.len() + 100; // base + overhead
        for d in &self.directives {
            capacity += d.instruction.len() + 20; // instruction + formatting
        }
        if let Some(ctx) = platform_context {
            capacity += ctx.len() + 30;
        }
        if let Some(ctx) = session_context {
            capacity += ctx.len() + 30;
        }
        if let Some(ctx) = memory_context {
            capacity += ctx.len() + 30;
        }

        let mut msg = String::with_capacity(capacity);

        // Core persona
        msg.push_str(&self.persona);
        msg.push_str("\n\n");

        // Directives
        msg.push_str("## Behavioral Directives\n");
        for directive in &self.directives {
            msg.push_str("- ");
            msg.push_str(&directive.instruction);
            msg.push('\n');
        }

        // Platform context
        if let Some(ctx) = platform_context {
            msg.push_str("\n## Current Platform\n");
            msg.push_str(ctx);
            msg.push('\n');
        }

        // Session context
        if let Some(ctx) = session_context {
            msg.push_str("\n## Session Info\n");
            msg.push_str(ctx);
            msg.push('\n');
        }

        // Long-term memory recall
        if let Some(ctx) = memory_context {
            msg.push_str("\n## Recalled Memories\n");
            msg.push_str(ctx);
            msg.push('\n');
        }

        msg
    }
}
