use crate::browser_skill::BrowserAutomation;
use crate::db::ChatMessage;
use crate::inference::InferenceEngine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

const MAX_VISION_CYCLES: usize = 8;
const CYCLE_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionAction {
    pub action: String,
    pub selector: Option<String>,
    pub text: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisionBrowserState {
    pub url: String,
    pub has_screenshot: bool,
    pub cycle: usize,
    pub page_title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisionBrowserMetrics {
    pub cycles_used: usize,
    pub actions_taken: Vec<String>,
    pub final_url: String,
    pub success: bool,
    pub total_time_ms: u64,
}

impl VisionBrowserMetrics {
    pub fn summary(&self) -> String {
        format!(
            "Vision Browser: {} cycles, {} actions, final URL: {}, {}",
            self.cycles_used,
            self.actions_taken.len(),
            self.final_url,
            if self.success { "SUCCESS" } else { "FAILED" }
        )
    }
}

pub async fn run_vision_task(
    task_desc: &str,
    browser: Arc<BrowserAutomation>,
    engine: &dyn InferenceEngine,
) -> Result<(String, VisionBrowserMetrics), String> {
    let start_time = Instant::now();
    let mut metrics = VisionBrowserMetrics {
        cycles_used: 0,
        actions_taken: Vec::new(),
        final_url: String::new(),
        success: false,
        total_time_ms: 0,
    };

    let base_url = extract_url(task_desc);

    if !base_url.is_empty() {
        info!(url = %base_url, "Vision browser navigating");
        match browser.navigate(&base_url).await {
            Ok(session) => {
                metrics.final_url = session.current_url.unwrap_or(base_url.clone());
            }
            Err(e) => {
                warn!("Navigation failed: {}", e);
                metrics.final_url = base_url;
            }
        }
        browser.wait(2.0).await.ok();
    }

    let mut accumulated_info = Vec::new();
    let mut last_actions = Vec::new();

    for cycle in 0..MAX_VISION_CYCLES {
        let cycle_start = Instant::now();
        if cycle_start.duration_since(start_time).as_secs() > CYCLE_TIMEOUT_SECS {
            warn!("Vision browser timeout");
            break;
        }

        metrics.cycles_used = cycle + 1;

        let screenshot_result = browser.screenshot(false).await;
        let content_result = browser.get_content().await;

        let page_info = match (&screenshot_result, &content_result) {
            (Ok(_), Ok(content)) => {
                if content.len() > 3000 {
                    content[..3000].to_string()
                } else {
                    content.clone()
                }
            }
            (_, Ok(content)) => {
                if content.len() > 3000 {
                    content[..3000].to_string()
                } else {
                    content.clone()
                }
            }
            _ => "Could not retrieve page content".to_string(),
        };

        let has_screenshot = screenshot_result.is_ok();

        let page_analysis = format!(
            r#"CYCLE {} PAGE STATE:
URL: {}
Has Screenshot: {}
Page Content (first 3000 chars):
{}

Task objective: "{}"

Analyze the page content and determine the NEXT action to take.
Output a JSON action with:
{{
  "action": "click|type|scroll|navigate|extract|done",
  "selector": "CSS selector for click/type/extract",
  "text": "text to type (for type action)",
  "reason": "why this action is needed"
}}

If the task is complete or you've extracted the needed info, use "done".
For scrolling: "scroll" with no selector needed.
For navigating to a new URL: "navigate" with the URL as "text".
Use only CSS selectors found in the page content."#,
            cycle + 1,
            metrics.final_url,
            has_screenshot,
            page_info,
            task_desc,
        );

        let analysis_msgs = vec![ChatMessage {
            role: "user".to_string(),
            content: page_analysis,
        }];

        let analysis_response = engine
            .infer(analysis_msgs, None, 0.2, 1024)
            .await
            .unwrap_or_else(|e| {
                warn!("Vision analysis failed: {}", e);
                r#"{"action": "done", "reason": "analysis failed"}"#.to_string()
            });

        let action: VisionAction = match extract_action_json(&analysis_response) {
            Ok(a) => a,
            Err(_) => {
                warn!("Could not parse vision action");
                break;
            }
        };

        info!(
            cycle = cycle,
            action = %action.action,
            "Vision browser action"
        );

        last_actions.push(format!("{}: {}", action.action, action.reason));
        metrics.actions_taken = last_actions.clone();

        if action.action == "done" {
            metrics.success = true;
            info!("Vision task marked as done");
            break;
        }

        match action.action.as_str() {
            "click" => {
                if let Some(ref selector) = action.selector {
                    let clicked = browser.click(selector).await.unwrap_or(false);
                    if clicked {
                        accumulated_info.push(format!("Clicked: {}", selector));
                    } else {
                        accumulated_info.push(format!("Failed to click: {}", selector));
                    }
                }
            }
            "type" => {
                if let (Some(ref selector), Some(ref text)) =
                    (&action.selector, &action.text)
                {
                    let typed = browser.type_text(selector, text).await.unwrap_or(false);
                    if typed {
                        accumulated_info.push(format!("Typed '{}' into {}", text, selector));
                    }
                }
            }
            "scroll" => {
                browser.scroll(800).await.ok();
                accumulated_info.push("Scrolled down".to_string());
            }
            "navigate" => {
                if let Some(ref url) = action.text {
                    match browser.navigate(url).await {
                        Ok(session) => {
                            metrics.final_url =
                                session.current_url.unwrap_or(url.clone());
                            accumulated_info.push(format!("Navigated to: {}", url));
                        }
                        Err(e) => {
                            accumulated_info.push(format!("Navigation failed: {}", e));
                        }
                    }
                }
            }
            "extract" => {
                if let Some(ref selector) = action.selector {
                    match browser.extract(selector).await {
                        Ok(extracted) => {
                            accumulated_info.push(format!(
                                "Extracted from {}: {}",
                                selector,
                                if extracted.len() > 500 {
                                    format!("{}... [{} chars]", &extracted[..500], extracted.len())
                                } else {
                                    extracted
                                }
                            ));
                        }
                        Err(e) => {
                            accumulated_info.push(format!("Extract failed: {}", e));
                        }
                    }
                }
            }
            _ => {
                warn!("Unknown vision action: {}", action.action);
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    }

    metrics.total_time_ms = start_time.elapsed().as_millis() as u64;
    metrics.success = !accumulated_info.is_empty() || metrics.success;

    let result_text = format!(
        "Vision browser task completed.
URL: {}
Cycles: {}
Actions taken:
{}

Collected information:
{}",
        metrics.final_url,
        metrics.cycles_used,
        metrics.actions_taken.join("\n  - "),
        accumulated_info.join("\n  - ")
    );

    info!(
        cycles = metrics.cycles_used,
        actions = metrics.actions_taken.len(),
        ms = metrics.total_time_ms,
        "Vision browser task complete"
    );

    Ok((result_text, metrics))
}

fn extract_url(task_desc: &str) -> String {
    let url_patterns = ["http://", "https://", "www."];
    for pattern in &url_patterns {
        if let Some(pos) = task_desc.find(pattern) {
            let start = pos;
            let end = task_desc[start..]
                .find(|c: char| c.is_whitespace())
                .map(|p| start + p)
                .unwrap_or(task_desc.len());
            return task_desc[start..end].to_string();
        }
    }

    let known_sites: &[(&str, &str)] = &[
        ("amazon", "https://www.amazon.com"),
        ("crates.io", "https://crates.io"),
        ("github", "https://github.com"),
        ("google", "https://www.google.com"),
        ("wikipedia", "https://en.wikipedia.org"),
        ("hacker news", "https://news.ycombinator.com"),
        ("reddit", "https://www.reddit.com"),
        ("youtube", "https://www.youtube.com"),
    ];

    for (keyword, url) in known_sites {
        if task_desc.to_lowercase().contains(keyword) {
            return url.to_string();
        }
    }

    String::new()
}

fn extract_action_json(response: &str) -> Result<VisionAction, String> {
    let json_str = if let Some(start) = response.find('{') {
        if let Some(end) = response.rfind('}') {
            &response[start..=end]
        } else {
            return Err("No JSON object".to_string());
        }
    } else {
        return Err("No JSON found".to_string());
    };

    let v: Value = serde_json::from_str(json_str)
        .map_err(|e| format!("Parse error: {}", e))?;

    let action = v
        .get("action")
        .and_then(|a| a.as_str())
        .unwrap_or("done")
        .to_string();
    let selector = v.get("selector").and_then(|s| s.as_str()).map(|s| s.to_string());
    let text = v.get("text").and_then(|s| s.as_str()).map(|s| s.to_string());
    let reason = v
        .get("reason")
        .and_then(|r| r.as_str())
        .unwrap_or("no reason")
        .to_string();

    Ok(VisionAction {
        action,
        selector,
        text,
        reason,
    })
}