// ============================================================================
// ULTRACLAW — formatter.rs
// ============================================================================
// Platform-aware output formatting.
//
// Because Ultraclaw speaks on 15+ platforms simultaneously, the same response
// must be adapted for each platform's rendering capabilities:
// - Discord/Slack: full markdown, code blocks, embeds
// - Telegram: limited markdown (bold, italic, code, links)
// - WhatsApp: limited markdown (bold, italic, strikethrough, monospace)
// - iMessage/SMS: plain text only
// - LINE/Zalo: basic text with some formatting
//
// ARCHITECTURE:
// The formatter inspects the chat_id to detect the platform.
//
// In practice, the prefix or suffix identifies the connector. 
// This module uses heuristic matching.
//
// MEMORY OPTIMIZATION:
// - Platform is a fieldless enum: 1 byte total.
// - Format operations are in-place on a String. No intermediate copies.
// - Message length caps prevent sending huge messages that bridges
//   would reject anyway (and the retry would waste energy).
//
// ENERGY OPTIMIZATION:
// - Single-pass string transformations. No regex.
// - Length capping is O(1) — we just truncate at a char boundary.
// ============================================================================

/// The platform a user is messaging from, detected via bridge room_id patterns.
///
/// This enum is 1 byte (fieldless, < 256 variants).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Platform {
    /// Discord (full markdown + embeds)
    Discord,
    /// Slack (mrkdwn format, similar to markdown)
    Slack,
    /// Telegram (limited HTML/markdown)
    Telegram,
    /// WhatsApp (limited formatting: *bold*, _italic_, ~strike~, ```mono```)
    WhatsApp,
    /// iMessage / Apple Messages (plain text only)
    IMessage,
    /// SMS (plain text, strict length limits)
    Sms,
    /// LINE messenger (basic text)
    Line,
    /// Zalo (Vietnamese messenger, basic text)
    Zalo,
    /// Signal (basic markdown)
    Signal,
    /// Facebook Messenger (basic text)
    Messenger,
    /// Instagram DMs (basic text)
    Instagram,
    /// WeChat (basic text)
    WeChat,
    /// IRC (plain text, some formatting codes)
    Irc,
    /// Email (full HTML/markdown)
    Email,
    /// Matrix (full markdown, supports的房间 names)
    Matrix,
    /// Unknown platform — default to safe plain text
    #[allow(dead_code)]
    Unknown,
}

/// Maximum message lengths per platform.
/// These prevent bridge rejections and excessive data transfer.
///
/// Note: These are conservative limits. Some platforms allow more, but
/// long messages are almost always truncated or split by the bridge anyway.
impl Platform {
    /// Get the maximum message length for this platform in characters.
    pub fn max_length(&self) -> usize {
        match self {
            Platform::Discord => 2000,   // Discord's hard limit
            Platform::Slack => 4000,     // Slack blocks have a 3000 char limit
            Platform::Telegram => 4096,  // Telegram's limit
            Platform::WhatsApp => 4096,  // WhatsApp's practical limit
            Platform::IMessage => 8000,  // iMessage is generous
            Platform::Sms => 1600,       // 10 SMS segments max (160 * 10)
            Platform::Line => 5000,      // LINE's limit
            Platform::Zalo => 3000,      // Zalo's practical limit
            Platform::Signal => 6000,    // Signal is generous
            Platform::Messenger => 2000, // Facebook Messenger
            Platform::Instagram => 1000, // Instagram DMs are short
            Platform::WeChat => 4000,    // WeChat limit
            Platform::Irc => 500,        // IRC is per-line, keep it short
            Platform::Email => 50000,    // Email has no practical limit
            Platform::Matrix => 16000,   // Matrix has generous limit
            Platform::Unknown => 2000,   // Conservative default
        }
    }

    /// Does this platform support markdown rendering?
    pub fn supports_markdown(&self) -> bool {
        matches!(
            self,
            Platform::Discord | Platform::Slack | Platform::Email | Platform::Matrix
        )
    }

    /// Does this platform support code blocks (``` fenced)?
    pub fn supports_code_blocks(&self) -> bool {
        matches!(
            self,
            Platform::Discord | Platform::Slack | Platform::Telegram | Platform::Matrix
        )
    }
}

/// Detect the platform from a chat_id.
///
/// This is a heuristic matching function.
///
/// # Examples
/// - `whatsapp_1234` → WhatsApp
/// - `telegram_1234` → Telegram
/// - `discord_1234` → Discord
pub fn detect_platform(chat_id: &str) -> Platform {
    // Extract the platform name chunk
    let lower = chat_id.to_lowercase();

    if lower.contains("whatsapp") {
        Platform::WhatsApp
    } else if lower.contains("telegram") || lower.contains("tg.") {
        Platform::Telegram
    } else if lower.contains("discord") {
        Platform::Discord
    } else if lower.contains("slack") {
        Platform::Slack
    } else if lower.contains("imessage") || lower.contains("apple") || lower.contains("beeper") {
        Platform::IMessage
    } else if lower.contains("signal") {
        Platform::Signal
    } else if lower.contains("line.") || lower.contains("linemsg") {
        Platform::Line
    } else if lower.contains("zalo") {
        Platform::Zalo
    } else if lower.contains("messenger") || lower.contains("facebook") || lower.contains("fb.") {
        Platform::Messenger
    } else if lower.contains("instagram") || lower.contains("ig.") {
        Platform::Instagram
    } else if lower.contains("wechat") || lower.contains("weixin") {
        Platform::WeChat
    } else if lower.contains("irc") {
        Platform::Irc
    } else if lower.contains("email") || lower.contains("smtp") {
        Platform::Email
    } else if chat_id.starts_with('!') || lower.contains("matrix") {
        Platform::Matrix
    } else if lower.contains("sms") || lower.contains("gsm") {
        Platform::Sms
    } else {
        // Default
        Platform::Unknown
    }
}

/// Format a response for the target platform.
///
/// Performs platform-specific transformations:
/// 1. Strip markdown for plaintext platforms
/// 2. Convert code blocks to indented text where needed
/// 3. Enforce message length limits
/// 4. Add platform-specific formatting hints
///
/// # Memory Usage
/// This operates in-place on a String. The only allocation is the
/// result String, which is at most `platform.max_length()` characters.
pub fn format_response(text: &str, platform: Platform) -> String {
    let mut output = if platform.supports_markdown() {
        // Markdown-capable platforms: keep formatting as-is
        text.to_string()
    } else if platform.supports_code_blocks() {
        // Platforms with code block support but limited markdown
        strip_advanced_markdown(text)
    } else {
        // Plaintext platforms: strip all markdown
        strip_all_markdown(text)
    };

    // Enforce platform message length limit
    let max_len = platform.max_length();
    if output.len() > max_len {
        // Find a safe truncation point (char boundary)
        let mut end = max_len - 20; // Leave room for truncation notice
        while !output.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        output.truncate(end);
        output.push_str("\n\n[Message truncated]");
    }

    output
}

/// Strip advanced markdown while keeping basic formatting.
///
/// Removes: headers (##), horizontal rules (---), tables, images, links.
/// Keeps: bold, italic, code blocks, lists.
fn strip_advanced_markdown(text: &str) -> String {
    let mut result = String::with_capacity(text.len());

    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            // Convert headers to bold text
            let content = trimmed.trim_start_matches('#').trim();
            result.push_str(&format!("*{}*\n", content));
        } else if trimmed.starts_with("---") || trimmed.starts_with("***") {
            // Skip horizontal rules
            result.push('\n');
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    result
}

/// Strip ALL markdown formatting for plaintext platforms (SMS, iMessage).
///
/// Single-pass character-level processing. No regex dependency.
fn strip_all_markdown(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_code_block = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '`' => {
                // Check for code block (```) or inline code (`)
                if chars.peek() == Some(&'`') {
                    chars.next();
                    if chars.peek() == Some(&'`') {
                        chars.next();
                        in_code_block = !in_code_block;
                        // Skip language tag after opening ```
                        if in_code_block {
                            while let Some(&c) = chars.peek() {
                                if c == '\n' {
                                    chars.next();
                                    break;
                                }
                                chars.next();
                            }
                        }
                        result.push('\n');
                    }
                }
                // Skip single backticks (inline code markers)
            }
            '*' | '_' => {
                // Skip markdown emphasis markers (* ** _ __)
                if chars.peek() == Some(&ch) {
                    chars.next(); // Skip doubled markers
                }
            }
            '~' => {
                // Skip strikethrough markers (~~ ~~)
                if chars.peek() == Some(&'~') {
                    chars.next();
                }
            }
            '#' if result.ends_with('\n') || result.is_empty() => {
                // Strip header markers at the start of a line
                while chars.peek() == Some(&'#') {
                    chars.next();
                }
                if chars.peek() == Some(&' ') {
                    chars.next(); // Skip the space after #
                }
            }
            '[' => {
                // Convert markdown links [text](url) to just "text"
                let mut link_text = String::new();
                let mut found_close = false;
                for c in chars.by_ref() {
                    if c == ']' {
                        found_close = true;
                        break;
                    }
                    link_text.push(c);
                }
                if found_close && chars.peek() == Some(&'(') {
                    // Skip the URL part
                    chars.next(); // consume '('
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                    result.push_str(&link_text);
                } else {
                    result.push('[');
                    result.push_str(&link_text);
                    if found_close {
                        result.push(']');
                    }
                }
            }
            _ => {
                result.push(ch);
            }
        }
    }

    result
}

/// Strip HTML tags from rich-text messages.
///
/// We need to extract just the text content. This is a simple tag stripper, not
/// a full HTML parser.
pub fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}
