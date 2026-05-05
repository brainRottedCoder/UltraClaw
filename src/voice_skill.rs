use crate::skill::{Skill, SkillOutput};
use crate::tts_service::TTSService;
use serde_json::Value;
use std::sync::Arc;

pub struct VoiceSkill {
    tts: Arc<TTSService>,
    is_active: bool,
}

impl VoiceSkill {
    pub fn new() -> Self {
        Self {
            tts: Arc::new(TTSService::new()),
            is_active: false,
        }
    }

    pub async fn initialize(&self) -> Result<(), String> {
        self.tts.start().await
    }

    pub async fn speak(&self, text: &str, language: Option<&str>) -> Result<Vec<u8>, String> {
        self.tts.speak(text, language).await
    }
}

impl Default for VoiceSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for VoiceSkill {
    fn name(&self) -> &'static str {
        "voice"
    }

    fn description(&self) -> &'static str {
        "Text-to-Speech: Convert text to audio. Also check voice status and capabilities. \
         Requires Coqui TTS installed (pip install TTS). Use for reading notifications, \
         alerts, or voice output. Returns audio data as base64 WAV."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Voice action to perform",
                    "enum": ["speak", "status", "list_languages", "stop"]
                },
                "text": {
                    "type": "string",
                    "description": "Text to speak (for speak action)"
                },
                "language": {
                    "type": "string",
                    "description": "Language code (e.g., 'en', 'es', 'fr', 'de', 'zh', 'ja')",
                    "default": "en"
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("status").to_string();
        let tts = self.tts.clone();
        let args_owned = args.clone();

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "voice".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let result = std::thread::spawn(move || {
            rt.block_on(async move {
                match action.as_str() {
                    "speak" => {
                        let text = args_owned.get("text").and_then(|v| v.as_str())
                            .unwrap_or("No text provided");
                        let language = args_owned.get("language").and_then(|v| v.as_str());

                        match tts.speak(text, language).await {
                            Ok(audio) => {
                                use base64::Engine;
                                let b64: String = base64::engine::general_purpose::STANDARD.encode(&audio);
                                Ok(serde_json::json!({
                                    "status": "success",
                                    "audio_bytes": audio.len(),
                                    "audio_base64": b64,
                                    "format": "wav",
                                    "language": language.unwrap_or("en")
                                }).to_string())
                            }
                            Err(e) => Err(e)
                        }
                    }
                    "status" => {
                        let available = tts.is_available().await;
                        Ok(serde_json::json!({
                            "status": if available { "ready" } else { "unavailable" },
                            "tts_available": available,
                            "message": if available {
                                "TTS service is ready. Use speak action to generate audio."
                            } else {
                                "TTS not available. Install Coqui TTS: pip install TTS"
                            }
                        }).to_string())
                    }
                    "list_languages" => {
                        Ok(serde_json::json!({
                            "languages": [
                                {"code": "en", "name": "English"},
                                {"code": "es", "name": "Spanish"},
                                {"code": "fr", "name": "French"},
                                {"code": "de", "name": "German"},
                                {"code": "it", "name": "Italian"},
                                {"code": "pt", "name": "Portuguese"},
                                {"code": "pl", "name": "Polish"},
                                {"code": "nl", "name": "Dutch"},
                                {"code": "ar", "name": "Arabic"},
                                {"code": "zh", "name": "Chinese"},
                                {"code": "ja", "name": "Japanese"},
                                {"code": "ko", "name": "Korean"},
                                {"code": "hi", "name": "Hindi"}
                            ]
                        }).to_string())
                    }
                    "stop" => {
                        tts.shutdown().await;
                        Ok("Voice service stopped".to_string())
                    }
                    _ => Err(format!("Unknown action: {}", action))
                }
            })
        })
        .join()
        .unwrap_or_else(|_| Err("Voice operation panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "voice".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "voice".to_string(),
                output: format!("Voice error: {}", e),
                is_error: true,
            },
        }
    }
}