use crate::skill::{Skill, SkillOutput};
use crate::browser_skill::BrowserAutomation;
use crate::vision_agent::VisionAgent;
use serde_json::Value;
use std::sync::Arc;

pub struct VisionBrowserSkill {
    automation: Arc<BrowserAutomation>,
}

impl VisionBrowserSkill {
    pub fn new() -> Self {
        Self {
            automation: Arc::new(BrowserAutomation::new()),
        }
    }

    pub fn get_automation(&self) -> Arc<BrowserAutomation> {
        self.automation.clone()
    }
}

impl Default for VisionBrowserSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for VisionBrowserSkill {
    fn name(&self) -> &'static str {
        "vision_browser"
    }

    fn description(&self) -> &'static str {
        "Vision-based browser automation. Uses AI to analyze screenshots and plan actions. \
         Specify a task like 'login to example.com' and the agent will analyze the page, \
         identify elements, and execute clicks/types automatically. More reliable than \
         specifying exact CSS selectors."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Vision browser action",
                    "enum": ["navigate", "task", "analyze", "plan"]
                },
                "task": {
                    "type": "string",
                    "description": "Task description (e.g., 'login with user/pass')"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to"
                },
                "max_iterations": {
                    "type": "integer",
                    "description": "Maximum planning iterations",
                    "default": 5
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
                    name: "vision_browser".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let automation = self.automation.clone();
        let args_clone = args.clone();

        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let action = args_clone.get("action").and_then(|v| v.as_str()).unwrap_or("analyze");

                match action {
                    "navigate" => {
                        let url = args_clone.get("url").and_then(|v| v.as_str())
                            .ok_or("URL required for navigate")?;
                        let session = automation.navigate(url).await
                            .map_err(|e| e.to_string())?;
                        Ok(format!(
                            "Navigated to {}\nTitle: {}",
                            session.current_url.unwrap_or_default(),
                            session.title.unwrap_or_default()
                        ))
                    }
                    "analyze" => {
                        let screenshot = automation.screenshot(false).await
                            .map_err(|e| e.to_string())?;
                        let content = automation.get_content().await
                            .map_err(|e| e.to_string())?;
                        Ok(format!(
                            "Screenshots: {} bytes\nPage content: {} chars",
                            screenshot.len(),
                            content.len()
                        ))
                    }
                    "task" => {
                        let task = args_clone.get("task").and_then(|v| v.as_str())
                            .ok_or("Task required")?;
                        let max_iterations = args_clone.get("max_iterations")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(5) as usize;
                        
                        let agent = VisionAgent::new(automation.clone(), None);
                        let result = agent.complete_task(task, max_iterations).await
                            .map_err(|e| e.to_string())?;
                        Ok(format!("Vision task completed:\n{}", result))
                    }
                    "plan" => {
                        let task = args_clone.get("task").and_then(|v| v.as_str())
                            .unwrap_or("Complete the task on this page");
                        
                        let agent = VisionAgent::new(automation.clone(), None);
                        let plan = agent.analyze_and_plan(task).await
                            .map_err(|e| e.to_string())?;
                        
                        Ok(format!(
                            "Vision plan: {}\nSteps: {}",
                            plan.goal,
                            plan.steps.iter().map(|s| format!("- {} ({})", s.action, s.reasoning)).collect::<Vec<_>>().join("\n")
                        ))
                    }
                    _ => Err(format!("Unknown action: {}", action))
                }
            })
        })
        .join()
        .unwrap_or_else(|_| Err("Vision browser panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "vision_browser".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "vision_browser".to_string(),
                output: format!("Vision browser error: {}", e),
                is_error: true,
            },
        }
    }
}