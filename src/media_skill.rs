// ============================================================================
// ULTRACLAW — media_skill.rs
// ============================================================================
// LLM-callable skills for image and video generation.
//
// These skills are registered in the SkillRegistry, making them available
// to the LLM via the OpenAI function-calling protocol. When the LLM decides
// to generate media, it emits a tool call with the appropriate parameters,
// which gets dispatched here.
//
// ARCHITECTURE:
// The skills hold an Arc<MediaEngine> reference. When invoked, they:
// 1. Parse the LLM's arguments into ImageParams/VideoParams
// 2. Call the MediaEngine to generate the media
// 3. Return the file path + metadata so the connector can upload the file
//
// MEMORY OPTIMIZATION:
// - Skills are zero-size structs holding only an Arc pointer (8 bytes each).
// - Generated media is saved directly to disk — never fully buffered in RAM.
// - The skill output includes the file path for the active connector to stream-upload.
// ============================================================================

use crate::media::{ImageParams, MediaEngine, MediaProvider, VideoParams};
use crate::skill::{Skill, SkillOutput};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

// ============================================================================
// GENERATE IMAGE SKILL
// ============================================================================

/// Skill that generates images via cloud APIs.
///
/// The LLM can request image generation by emitting a tool call like:
/// ```json
/// { "name": "generate_image", "arguments": { "prompt": "a cat in space" } }
/// ```
pub struct GenerateImageSkill {
    engine: Arc<Mutex<MediaEngine>>,
}

impl GenerateImageSkill {
    pub fn new(engine: Arc<Mutex<MediaEngine>>) -> Self {
        Self { engine }
    }
}

impl Skill for GenerateImageSkill {
    fn name(&self) -> &'static str {
        "generate_image"
    }

    fn description(&self) -> &'static str {
        "Generate an image from a text prompt using AI. Supports multiple cloud providers \
         (DALL-E 3, Stability AI, Flux, Leonardo, etc.). Returns the file path of the \
         generated image which will be sent to the user."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Detailed text description of the image to generate"
                },
                "width": {
                    "type": "integer",
                    "description": "Image width in pixels (default: 1024)",
                    "default": 1024
                },
                "height": {
                    "type": "integer",
                    "description": "Image height in pixels (default: 1024)",
                    "default": 1024
                },
                "style": {
                    "type": "string",
                    "description": "Optional style preset (e.g., 'vivid', 'natural', 'anime')"
                },
                "provider": {
                    "type": "string",
                    "description": "Optional: specific provider to use (openai, stability, replicate, together, fal, leonardo, imagen)"
                },
                "negative_prompt": {
                    "type": "string",
                    "description": "Optional: things to exclude from the image"
                }
            },
            "required": ["prompt"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        // We need an async runtime to call the MediaEngine.
        // Use the current tokio runtime handle.
        let engine = self.engine.clone();
        let args = args.clone();

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "generate_image".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        // Block on the async call from the sync context.
        // This is safe because execute_sync is called from spawn_blocking.
        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let params = ImageParams {
                    prompt: args["prompt"].as_str().unwrap_or("").to_string(),
                    width: args["width"].as_u64().unwrap_or(1024) as u32,
                    height: args["height"].as_u64().unwrap_or(1024) as u32,
                    style: args["style"].as_str().map(String::from),
                    model: args["model"].as_str().map(String::from),
                    negative_prompt: args["negative_prompt"].as_str().map(String::from),
                    count: 1,
                };

                let provider = args["provider"]
                    .as_str()
                    .and_then(MediaProvider::from_str_loose);

                let engine = engine.lock().await;
                engine.generate_image(&params, provider).await
            })
        })
        .join()
        .unwrap_or_else(|_| Err("Image generation thread panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "generate_image".to_string(),
                output: serde_json::json!({
                    "status": "success",
                    "file_path": output.file_path.to_string_lossy(),
                    "mime_type": output.mime_type,
                    "file_size_bytes": output.file_size,
                    "provider": format!("{:?}", output.provider),
                    "metadata": output.metadata
                })
                .to_string(),
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "generate_image".to_string(),
                output: format!("Image generation failed: {}", e),
                is_error: true,
            },
        }
    }
}

// ============================================================================
// GENERATE VIDEO SKILL
// ============================================================================

/// Skill that generates videos via cloud APIs.
///
/// The LLM can request video generation by emitting a tool call like:
/// ```json
/// { "name": "generate_video", "arguments": { "prompt": "a cat walking" } }
/// ```
pub struct GenerateVideoSkill {
    engine: Arc<Mutex<MediaEngine>>,
}

impl GenerateVideoSkill {
    pub fn new(engine: Arc<Mutex<MediaEngine>>) -> Self {
        Self { engine }
    }
}

impl Skill for GenerateVideoSkill {
    fn name(&self) -> &'static str {
        "generate_video"
    }

    fn description(&self) -> &'static str {
        "Generate a short video from a text prompt using AI. Supports providers like \
         Runway ML (Gen-3), Replicate, and Fal.ai. Returns the file path of the \
         generated video which will be sent to the user. Generation may take 1-5 minutes."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Detailed text description of the video to generate"
                },
                "duration_secs": {
                    "type": "integer",
                    "description": "Video duration in seconds (default: 5, max varies by provider)",
                    "default": 5
                },
                "provider": {
                    "type": "string",
                    "description": "Optional: specific provider to use (runway, replicate, fal)"
                },
                "image_url": {
                    "type": "string",
                    "description": "Optional: URL of an image to animate (for image-to-video)"
                }
            },
            "required": ["prompt"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let engine = self.engine.clone();
        let args = args.clone();

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "generate_video".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let params = VideoParams {
                    prompt: args["prompt"].as_str().unwrap_or("").to_string(),
                    duration_secs: args["duration_secs"].as_u64().unwrap_or(5) as u32,
                    image_url: args["image_url"].as_str().map(String::from),
                    model: args["model"].as_str().map(String::from),
                };

                let provider = args["provider"]
                    .as_str()
                    .and_then(MediaProvider::from_str_loose);

                let engine = engine.lock().await;
                engine.generate_video(&params, provider).await
            })
        })
        .join()
        .unwrap_or_else(|_| Err("Video generation thread panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "generate_video".to_string(),
                output: serde_json::json!({
                    "status": "success",
                    "file_path": output.file_path.to_string_lossy(),
                    "mime_type": output.mime_type,
                    "file_size_bytes": output.file_size,
                    "provider": format!("{:?}", output.provider),
                    "metadata": output.metadata
                })
                .to_string(),
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "generate_video".to_string(),
                output: format!("Video generation failed: {}", e),
                is_error: true,
            },
        }
    }
}
