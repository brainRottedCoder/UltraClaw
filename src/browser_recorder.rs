use crate::browser_skill::BrowserAutomation;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedStep {
    pub action: String,
    pub target: Option<String>,
    pub value: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub screenshot_before: Option<String>,
    pub screenshot_after: Option<String>,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub steps: Vec<RecordedStep>,
    pub initial_url: Option<String>,
    pub final_url: Option<String>,
}

pub struct BrowserRecorder {
    automation: Arc<BrowserAutomation>,
    is_recording: RwLock<bool>,
    current_recording: RwLock<Option<Recording>>,
    max_steps: usize,
}

impl BrowserRecorder {
    pub fn new(automation: Arc<BrowserAutomation>) -> Self {
        Self {
            automation,
            is_recording: RwLock::new(false),
            current_recording: RwLock::new(None),
            max_steps: 100,
        }
    }

    pub async fn start_recording(&self, name: &str) -> Result<(), String> {
        let mut is_recording = self.is_recording.write().await;
        if *is_recording {
            return Err("Already recording".to_string());
        }

        let session = self.automation.get_session().await;
        let url = session.and_then(|s| s.current_url);

        let recording = Recording {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            created_at: Utc::now(),
            steps: Vec::new(),
            initial_url: url,
            final_url: None,
        };

        *is_recording = true;
        *self.current_recording.write().await = Some(recording);

        Ok(())
    }

    pub async fn record_action(
        &self,
        action: &str,
        target: Option<&str>,
        value: Option<&str>,
        success: bool,
    ) -> Result<(), String> {
        let is_recording = *self.is_recording.read().await;
        if !is_recording {
            return Ok(());
        }

        let screenshot_before = self.automation.screenshot(false).await.ok();
        let step = RecordedStep {
            action: action.to_string(),
            target: target.map(String::from),
            value: value.map(String::from),
            timestamp: Utc::now(),
            screenshot_before,
            screenshot_after: None,
            success,
        };

        let mut recording = self.current_recording.write().await;
        if let Some(ref mut r) = *recording {
            if r.steps.len() < self.max_steps {
                r.steps.push(step);
            }
        }

        Ok(())
    }

    pub async fn stop_recording(&self) -> Result<Recording, String> {
        let mut is_recording = self.is_recording.write().await;
        *is_recording = false;

        let mut recording = self.current_recording.write().await
            .take()
            .ok_or("No active recording")?;

        let session = self.automation.get_session().await;
        recording.final_url = session.and_then(|s| s.current_url);

        Ok(recording)
    }

    pub async fn replay(&self, recording: &Recording) -> Result<String, String> {
        let mut results = Vec::new();

        if let Some(ref url) = recording.initial_url {
            self.automation.navigate(url).await
                .map_err(|e| format!("Failed to navigate to start URL: {}", e))?;
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }

        for step in &recording.steps {
            let action = step.action.as_str();
            let target = step.target.as_deref();
            let value = step.value.as_deref();

            let result = match action {
                "navigate" => {
                    if let Some(url) = target {
                        self.automation.navigate(url).await
                            .map(|_| format!("Navigated to {}", url))
                    } else {
                        Err("No URL for navigate".to_string())
                    }
                }
                "click" => {
                    if let Some(sel) = target {
                        self.automation.click(sel).await
                            .map(|_| format!("Clicked {}", sel))
                    } else {
                        Err("No selector for click".to_string())
                    }
                }
                "type" => {
                    if let (Some(sel), Some(text)) = (target, value) {
                        self.automation.type_text(sel, text).await
                            .map(|_| format!("Typed into {}", sel))
                    } else {
                        Err("Missing selector or text for type".to_string())
                    }
                }
                "hover" => {
                    if let Some(sel) = target {
                        self.automation.hover(sel).await
                            .map(|_| format!("Hovered {}", sel))
                    } else {
                        Err("No selector for hover".to_string())
                    }
                }
                "scroll" => {
                    let amount = value.and_then(|v| v.parse::<i32>().ok()).unwrap_or(500);
                    self.automation.scroll(amount).await
                        .map(|_| format!("Scrolled {}", amount))
                }
                "wait" => {
                    let seconds = value.and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
                    self.automation.wait(seconds).await
                        .map(|_| format!("Waited {}s", seconds))
                }
                _ => Err(format!("Unknown action: {}", action))
            };

            match result {
                Ok(r) => results.push(format!("OK {}: {}", action, r)),
                Err(e) => results.push(format!("FAIL {}: {}", action, e)),
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        }

        Ok(results.join(" | "))
    }

    pub fn get_summary(&self, recording: &Recording) -> String {
        format!(
            "Recording: {} ({} steps)\nInitial: {} -> Final: {}",
            recording.name,
            recording.steps.len(),
            recording.initial_url.as_deref().unwrap_or("unknown"),
            recording.final_url.as_deref().unwrap_or("unknown")
        )
    }

    pub fn get_recording(&self, id: &str) -> Option<Recording> {
        None
    }

    pub fn list_recordings(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn delete_recording(&self, _id: &str) -> bool {
        true
    }
}

impl Default for BrowserRecorder {
    fn default() -> Self {
        Self::new(Arc::new(BrowserAutomation::new()))
    }
}