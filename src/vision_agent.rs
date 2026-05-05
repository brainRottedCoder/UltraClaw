use crate::browser_skill::BrowserAutomation;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionAction {
    pub action: String,
    pub target: Option<String>,
    pub value: Option<String>,
    pub reasoning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionPlan {
    pub goal: String,
    pub steps: Vec<VisionAction>,
}

pub struct VisionAgent {
    browser: Arc<BrowserAutomation>,
    llm_url: Option<String>,
}

impl VisionAgent {
    pub fn new(browser: Arc<BrowserAutomation>, llm_url: Option<String>) -> Self {
        Self { browser, llm_url }
    }

    pub async fn analyze_and_plan(&self, task: &str) -> Result<VisionPlan, String> {
        let screenshot = self.browser.screenshot(false).await
            .map_err(|e| format!("Failed to capture screenshot: {}", e))?;

        let content = self.browser.get_content().await
            .map_err(|e| format!("Failed to get content: {}", e))?;

        let json_template = r#"{"goal": "description", "steps": [{"action": "click", "target": "selector", "reasoning": "why"}]}"#;
        let prompt = format!(
            "You are a browser automation vision agent. Analyze the current page and plan actions.\n\n\
             Task: {}\n\n\
             Based on the page content and screenshot, output a JSON plan with the next action:\n\
             {}\n\n\
             Common CSS selectors: #id for IDs, .class for classes, tag for tag names.\n\n\
             Page content ({} chars): {}",
            task,
            json_template,
            content.len(),
            if content.len() > 2000 { &content[..2000] } else { &content }
        );

        if let Some(ref url) = self.llm_url {
            let client = reqwest::Client::new();
            let response = client.post(url)
                .json(&serde_json::json!({
                    "messages": [
                        {"role": "user", "content": prompt}
                    ],
                    "max_tokens": 1024
                }))
                .send()
                .await
                .map_err(|e| format!("LLM request failed: {}", e))?;

            let json: serde_json::Value = response.json().await
                .map_err(|e| format!("Failed to parse LLM response: {}", e))?;

            let content = json.get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .ok_or("Invalid LLM response format")?;

            let plan: VisionPlan = serde_json::from_str(content)
                .map_err(|e| format!("Failed to parse vision plan: {}", e))?;
            Ok(plan)
        } else {
            Ok(VisionPlan {
                goal: task.to_string(),
                steps: vec![],
            })
        }
    }

    pub async fn execute_plan(&self, plan: &VisionPlan) -> Result<String, String> {
        let mut last_result = String::new();

        for step in &plan.steps {
            info!("Executing vision step: action={} target={:?}", step.action, step.target);

            match step.action.as_str() {
                "click" => {
                    let target = step.target.as_ref().ok_or("No target for click")?;
                    if !self.browser.click(target).await? {
                        return Err(format!("Click failed on: {}", target));
                    }
                    last_result = format!("Clicked: {}", target);
                }
                "type" => {
                    let target = step.target.as_ref().ok_or("No target for type")?;
                    let value = step.value.as_ref().ok_or("No value for type")?;
                    if !self.browser.type_text(target, value).await? {
                        return Err(format!("Type failed on: {}", target));
                    }
                    last_result = format!("Typed into: {}", target);
                }
                "navigate" => {
                    let target = step.target.as_ref().ok_or("No target for navigate")?;
                    let session = self.browser.navigate(target).await
                        .map_err(|e| format!("Navigation failed: {}", e))?;
                    last_result = format!("Navigated to: {}", session.current_url.unwrap_or_default());
                }
                "wait" => {
                    let seconds = step.value.as_ref().and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
                    self.browser.wait(seconds).await?;
                    last_result = format!("Waited");
                }
                "scroll" => {
                    let amount = step.value.as_ref().and_then(|v| v.parse::<i32>().ok()).unwrap_or(500);
                    self.browser.scroll(amount).await?;
                    last_result = format!("Scrolled");
                }
                "hover" => {
                    let target = step.target.as_ref().ok_or("No target for hover")?;
                    if !self.browser.hover(target).await? {
                        return Err(format!("Hover failed on: {}", target));
                    }
                    last_result = format!("Hovered: {}", target);
                }
                "screenshot" => {
                    let screenshot = self.browser.screenshot(false).await?;
                    last_result = format!("Screenshot: {} bytes", screenshot.len());
                }
                _ => {
                    warn!("Unknown vision action: {}", step.action);
                    last_result = format!("Unknown action: {}", step.action);
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        Ok(last_result)
    }

    pub async fn complete_task(&self, task: &str, max_iterations: usize) -> Result<String, String> {
        let mut iterations = 0;
        let mut results = Vec::new();

        loop {
            iterations += 1;
            if iterations > max_iterations {
                return Err("Max iterations reached".to_string());
            }

            let plan = self.analyze_and_plan(task).await?;
            
            if plan.steps.is_empty() {
                break;
            }

            let result = self.execute_plan(&plan).await?;
            results.push(result);

            let content = self.browser.get_content().await?;
            if content.contains("success") || content.contains("complete") {
                break;
            }
        }

        Ok(results.join(" | "))
    }
}

impl Default for VisionAgent {
    fn default() -> Self {
        Self::new(Arc::new(BrowserAutomation::new()), None)
    }
}