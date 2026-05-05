use crate::skill::{Skill, SkillOutput};
use crate::browser_skill::BrowserAutomation;
use crate::browser_recorder::{BrowserRecorder, Recording};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct BrowserReplaySkill {
    recorder: Arc<BrowserRecorder>,
    recordings: Arc<RwLock<HashMap<String, Recording>>>,
}

impl BrowserReplaySkill {
    pub fn new() -> Self {
        let automation = Arc::new(BrowserAutomation::new());
        Self {
            recorder: Arc::new(BrowserRecorder::new(automation)),
            recordings: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for BrowserReplaySkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for BrowserReplaySkill {
    fn name(&self) -> &'static str {
        "browser_replay"
    }

    fn description(&self) -> &'static str {
        "Record and replay browser automation sequences. Record a sequence of \
         actions, save it with a name, and replay it later. Useful for automating \
         repetitive tasks like form fills or test sequences."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Replay action",
                    "enum": ["start", "stop", "replay", "list", "delete", "info"]
                },
                "name": {
                    "type": "string",
                    "description": "Recording name"
                },
                "recording_id": {
                    "type": "string",
                    "description": "Recording ID"
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "browser_replay".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let recorder = self.recorder.clone();
        let recordings = self.recordings.clone();
        let args_clone = args.clone();

        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let action = args_clone.get("action").and_then(|v| v.as_str()).unwrap_or("list");

                match action {
                    "start" => {
                        let name = args_clone.get("name").and_then(|v| v.as_str())
                            .unwrap_or("Unnamed Recording");
                        recorder.start_recording(name).await
                            .map(|_| format!("Started recording: {}", name))
                    }
                    "stop" => {
                        let recording = recorder.stop_recording().await?;
                        let summary = recorder.get_summary(&recording);
                        
                        let mut recs = recordings.write().await;
                        recs.insert(recording.id.clone(), recording.clone());
                        
                        Ok(format!("{}\n\nRecording ID: {}", summary, recording.id))
                    }
                    "replay" => {
                        let recording_id = args_clone.get("recording_id").and_then(|v| v.as_str())
                            .ok_or("Recording ID required")?;
                        
                        let recs = recordings.read().await;
                        let recording = recs.get(recording_id)
                            .ok_or_else(|| format!("Recording {} not found", recording_id))?;
                        
                        recorder.replay(recording).await
                    }
                    "list" => {
                        let recs = recordings.read().await;
                        if recs.is_empty() {
                            Ok("No recordings stored".to_string())
                        } else {
                            let list: Vec<String> = recs.values()
                                .map(|r| format!("{} - {} ({} steps)", r.id, r.name, r.steps.len()))
                                .collect();
                            Ok(format!("Recordings:\n{}", list.join("\n")))
                        }
                    }
                    "info" => {
                        let recording_id = args_clone.get("recording_id").and_then(|v| v.as_str())
                            .ok_or("Recording ID required")?;
                        
                        let recs = recordings.read().await;
                        let recording = recs.get(recording_id)
                            .ok_or_else(|| format!("Recording {} not found", recording_id))?;
                        
                        Ok(recorder.get_summary(recording))
                    }
                    "delete" => {
                        let recording_id = args_clone.get("recording_id").and_then(|v| v.as_str())
                            .ok_or("Recording ID required")?;
                        
                        let mut recs = recordings.write().await;
                        if recs.remove(recording_id).is_some() {
                            Ok(format!("Deleted recording: {}", recording_id))
                        } else {
                            Err(format!("Recording {} not found", recording_id))
                        }
                    }
                    _ => Err(format!("Unknown action: {}", action))
                }
            })
        })
        .join()
        .unwrap_or_else(|_| Err("Browser replay panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "browser_replay".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "browser_replay".to_string(),
                output: format!("Browser replay error: {}", e),
                is_error: true,
            },
        }
    }
}