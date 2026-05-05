// ============================================================================
// ULTRACLAW — OpenClaw Extended Capabilities
// ============================================================================
// Registry for the 53 OpenClaw extension capabilities.
// Each capability can be activated, deactivated, and queried.
// Integrates with the Skill trait system for agent use.

use crate::skill::{Skill, SkillOutput};
use std::collections::HashMap;
use std::sync::Mutex;

const ALL_EXTENSIONS: &[(&str, &str, &str)] = &[
    ("1password", "Password Manager", "Securely access and manage credentials stored in 1Password"),
    ("apple-notes", "Apple Notes", "Read, create, and search Apple Notes"),
    ("apple-reminders", "Apple Reminders", "Manage reminders and to-do lists on Apple devices"),
    ("bear-notes", "Bear Notes", "Access and create notes in the Bear app"),
    ("blogwatcher", "Blog Monitor", "Watch blogs and RSS feeds for new content"),
    ("blucli", "Bluetooth CLI", "Control and monitor Bluetooth devices via CLI"),
    ("bluebubbles", "BlueBubbles", "Send and receive iMessages through BlueBubbles"),
    ("camsnap", "Camera Snapshot", "Capture photos from connected cameras"),
    ("canvas", "Canvas LMS", "Interact with Canvas Learning Management System"),
    ("clawhub", "Claw Hub", "Central hub for Ultraclaw extensions and plugins"),
    ("coding-agent", "Coding Agent", "Write, review, and debug code in multiple languages"),
    ("discord", "Discord", "Send messages and interact with Discord servers"),
    ("eightctl", "Eight Sleep", "Control Eight Sleep mattress and temperature settings"),
    ("gemini", "Google Gemini", "Query Google's Gemini AI models for advanced reasoning"),
    ("gh-issues", "GitHub Issues", "Create, list, and manage GitHub issues"),
    ("gifgrep", "GIF Search", "Search and retrieve GIFs from various sources"),
    ("github", "GitHub", "Full GitHub integration: repos, PRs, actions, and more"),
    ("gog", "GOG", "Access GOG.com game library and store"),
    ("goplaces", "Google Places", "Search for places, businesses, and reviews on Google Maps"),
    ("healthcheck", "Health Monitor", "Monitor system and service health with alerts"),
    ("himalaya", "Himalaya CLI", "Send and receive email via Himalaya email client"),
    ("imsg", "iMessage", "Send and receive messages through iMessage"),
    ("mcporter", "Minecraft Porter", "Manage and transfer Minecraft worlds and data"),
    ("model-usage", "Model Usage Tracker", "Track and report LLM token usage and costs"),
    ("nano-banana-pro", "Nano Banana Pro", "Access advanced image manipulation capabilities"),
    ("nano-pdf", "Nano PDF", "Create, read, and manipulate PDF documents"),
    ("notion", "Notion", "Read and write to Notion databases and pages"),
    ("obsidian", "Obsidian", "Access and manage Obsidian vaults and notes"),
    ("openai-image-gen", "OpenAI Image Gen", "Generate images using DALL-E models"),
    ("openai-whisper", "Whisper Local", "Transcribe audio using local Whisper models"),
    ("openai-whisper-api", "Whisper API", "Transcribe audio via OpenAI Whisper API"),
    ("openhue", "Philips Hue", "Control Philips Hue smart lights"),
    ("oracle", "Oracle DB", "Query and manage Oracle databases"),
    ("ordercli", "Order CLI", "Place orders and manage purchases via CLI"),
    ("peekaboo", "Peekaboo", "Preview files and media in the terminal"),
    ("sag", "SAG", "System Analysis and Graphing capabilities"),
    ("session-logs", "Session Logs", "View and search agent session history logs"),
    ("sherpa-onnx-tts", "Sherpa TTS", "Text-to-speech using Sherpa-ONNX models"),
    ("skill-creator", "Skill Creator", "Create and register new agent skills dynamically"),
    ("slack", "Slack", "Send messages and interact with Slack workspaces"),
    ("songsee", "Song Recognition", "Identify songs playing in your environment"),
    ("sonoscli", "Sonos CLI", "Control Sonos speakers and playlists"),
    ("spotify-player", "Spotify Player", "Control Spotify playback and playlists"),
    ("summarize", "Summarizer", "Summarize long texts, articles, and documents"),
    ("things-mac", "Things (Mac)", "Manage tasks in the Things app on macOS"),
    ("tmux", "Tmux", "Control and manage terminal tmux sessions"),
    ("trello", "Trello", "Manage Trello boards, lists, and cards"),
    ("video-frames", "Video Frame Extractor", "Extract and analyze frames from video files"),
    ("voice-call", "Voice Call", "Handle voice calls with speech recognition and synthesis"),
    ("wacli", "WhatsApp CLI", "Send and receive WhatsApp messages via CLI"),
    ("weather", "Weather", "Get weather forecasts and alerts for any location"),
    ("xurl", "X/URL", "Expand, shorten, and analyze URLs"),
];

#[derive(Debug, Clone)]
struct ExtensionInfo {
    display_name: &'static str,
    description: &'static str,
    active: bool,
}

pub struct OpenClawSkillRegistry {
    extensions: Mutex<HashMap<String, ExtensionInfo>>,
}

impl Default for OpenClawSkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenClawSkillRegistry {
    pub fn new() -> Self {
        let mut extensions = HashMap::new();
        for &(id, display_name, description) in ALL_EXTENSIONS {
            extensions.insert(id.to_string(), ExtensionInfo {
                display_name,
                description,
                active: false,
            });
        }
        Self { extensions: Mutex::new(extensions) }
    }

    pub fn list_extensions(&self) -> Vec<(String, &'static str, &'static str, bool)> {
        let exts = self.extensions.lock().unwrap();
        exts.iter()
            .map(|(id, info)| (id.clone(), info.display_name, info.description, info.active))
            .collect()
    }

    pub fn list_active(&self) -> Vec<(String, &'static str, &'static str)> {
        let exts = self.extensions.lock().unwrap();
        exts.iter()
            .filter(|(_, info)| info.active)
            .map(|(id, info)| (id.clone(), info.display_name, info.description))
            .collect()
    }

    pub fn activate_extension(&self, extension: &str) -> Result<String, String> {
        let mut exts = self.extensions.lock().unwrap();
        if let Some(info) = exts.get_mut(extension) {
            if info.active {
                return Ok(format!("Extension '{}' is already active", info.display_name));
            }
            info.active = true;
            Ok(format!("Activated: {}", info.display_name))
        } else {
            Err(format!("Unknown extension: '{}'", extension))
        }
    }

    pub fn deactivate_extension(&self, extension: &str) -> Result<String, String> {
        let mut exts = self.extensions.lock().unwrap();
        if let Some(info) = exts.get_mut(extension) {
            if !info.active {
                return Ok(format!("Extension '{}' is already inactive", info.display_name));
            }
            info.active = false;
            Ok(format!("Deactivated: {}", info.display_name))
        } else {
            Err(format!("Unknown extension: '{}'", extension))
        }
    }

    pub fn is_active(&self, extension: &str) -> bool {
        self.extensions.lock().unwrap().get(extension).map(|e| e.active).unwrap_or(false)
    }

    pub fn get_info(&self, extension: &str) -> Option<(String, &'static str, &'static str, bool)> {
        self.extensions.lock().unwrap().get(extension).map(|e| (extension.to_string(), e.display_name, e.description, e.active))
    }

    pub fn active_count(&self) -> usize {
        self.extensions.lock().unwrap().values().filter(|e| e.active).count()
    }

    pub fn total_count(&self) -> usize {
        self.extensions.lock().unwrap().len()
    }
}

impl Skill for OpenClawSkillRegistry {
    fn name(&self) -> &'static str {
        "openclaw_extensions"
    }

    fn description(&self) -> &'static str {
        "OpenClaw Extension Registry: Manage 53 hyper-capabilities including \
         password management, notes apps, smart home, messaging, email, \
         GitHub, Notion, Trello, weather, voice calls, and more. \
         Activate, deactivate, list, and query extension statuses."
    }

    fn schema(&self) -> serde_json::Value {
        let extension_names: Vec<&str> = ALL_EXTENSIONS.iter().map(|e| e.0).collect();
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform on OpenClaw extensions",
                    "enum": ["list", "list_active", "activate", "deactivate", "info", "status"]
                },
                "extension": {
                    "type": "string",
                    "description": "Extension name to activate/deactivate/inspect",
                    "enum": extension_names
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, params: &serde_json::Value) -> SkillOutput {
        let action = params.get("action").and_then(|v| v.as_str()).unwrap_or("list");

        let result: Result<String, String> = match action {
            "list" => {
                let all = self.list_extensions();
                let lines: Vec<String> = all.iter().map(|(id, name, desc, active)| {
                    let status = if *active { "[ON]" } else { "[OFF]" };
                    format!("  {} {} - {}: {}", status, id, name, desc)
                }).collect();
                Ok(format!(
                    "OpenClaw Extensions ({} total, {} active):\n{}",
                    self.total_count(),
                    self.active_count(),
                    lines.join("\n")
                ))
            }
            "list_active" => {
                let active = self.list_active();
                if active.is_empty() {
                    Ok("No extensions are currently active.".to_string())
                } else {
                    let lines: Vec<String> = active.iter()
                        .map(|(id, name, _desc)| format!("  {} - {}", id, name))
                        .collect();
                    Ok(format!("Active extensions ({}):\n{}", active.len(), lines.join("\n")))
                }
            }
            "activate" => {
                let ext = match params.get("extension").and_then(|v| v.as_str()) {
                    Some(e) => e,
                    None => return SkillOutput {
                        name: "openclaw_extensions".into(),
                        output: "extension name is required for activate action".into(),
                        is_error: true,
                    },
                };
                self.activate_extension(ext)
            }
            "deactivate" => {
                let ext = match params.get("extension").and_then(|v| v.as_str()) {
                    Some(e) => e,
                    None => return SkillOutput {
                        name: "openclaw_extensions".into(),
                        output: "extension name is required for deactivate action".into(),
                        is_error: true,
                    },
                };
                self.deactivate_extension(ext)
            }
            "info" => {
                let ext = match params.get("extension").and_then(|v| v.as_str()) {
                    Some(e) => e,
                    None => return SkillOutput {
                        name: "openclaw_extensions".into(),
                        output: "extension name is required for info action".into(),
                        is_error: true,
                    },
                };
                match self.get_info(ext) {
                    Some((_id, name, desc, active)) => {
                        let status = if active { "Active" } else { "Inactive" };
                        Ok(format!("Extension: {}\nID: {}\nStatus: {}\nDescription: {}", name, ext, status, desc))
                    }
                    None => Err(format!("Unknown extension: '{}'", ext))
                }
            }
            "status" => {
                Ok(format!(
                    "OpenClaw Registry: {} extensions available, {} currently active",
                    self.total_count(),
                    self.active_count()
                ))
            }
            _ => Err(format!("Unknown action: '{}'. Available: list, list_active, activate, deactivate, info, status", action))
        };

        match result {
            Ok(output) => SkillOutput {
                name: "openclaw_extensions".into(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "openclaw_extensions".into(),
                output: format!("OpenClaw error: {}", e),
                is_error: true,
            },
        }
    }
}