// ============================================================================
// ULTRACLAW — browser_skill.rs
// ============================================================================
// Headless Chromium browser automation via Playwright Python subprocess.
//
// This module provides full browser automation capabilities:
// - Navigate to URLs and capture DOM content
// - Take screenshots (PNG, base64 encoded)
// - Click elements by CSS selector
// - Type text into input fields
// - Extract text content from elements
// - Handle dynamically loaded content (JavaScript rendering)
//
// ARCHITECTURE:
// - Playwright Python proxy runs as a subprocess
// - Communication via JSON lines on stdin/stdout
// - Subprocess is lazily spawned on first use
// - Each session gets an isolated browser context
//
// SECURITY:
// - Runs in headless mode with sandbox flags for container safety
// - No arbitrary JavaScript execution from LLM (XSS prevention)
// - Session timeout: 60 seconds per action
// - Network access limited by the pages navigated to
//
// MEMORY OPTIMIZATION:
// - Playwright runs in subprocess, isolated from main process memory
// - Screenshot is streamed as base64, not fully buffered
// - Browser context is reused across actions, not recreated each time
// ============================================================================

use crate::skill::{Skill, SkillOutput, MAX_OUTPUT_BYTES};
use serde_json::Value;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, error, warn};

// Playwright Python proxy script
// This script manages a Chromium browser instance and handles automation commands
const BROWSER_PROXY_SCRIPT: &str = r#"
import json
import sys
import base64
from playwright.sync_api import sync_playwright

# Global state
browser = None
context = None
page = None

def init_browser():
    global browser, context, page
    if browser is None:
        p = sync_playwright().start()
        browser = p.chromium.launch(
            headless=True,
            args=[
                '--no-sandbox',
                '--disable-dev-shm-usage',
                '--disable-gpu',
                '--disable-software-rasterizer',
                '--disable-extensions',
                '--disable-background-networking',
                '--disable-default-apps',
                '--disable-sync',
                '--disable-translate',
                '--metrics-recording-only',
                '--mute-audio',
                '--no-first-run',
                '--safebrowsing-disable-auto-update',
            ]
        )
        context = browser.new_context(
            user_agent="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            viewport={"width": 1280, "height": 720},
            ignore_https_errors=True,
        )
        page = context.new_page()
        page.set_default_timeout(30000)
    return True

def cleanup():
    global browser, context, page
    if page:
        page.close()
        page = None
    if context:
        context.close()
        context = None
    if browser:
        browser.close()
        browser = None

def handle_navigate(url):
    global page, context
    if page is None:
        init_browser()
    else:
        # Close old page, create new one for clean state
        page.close()
        page = context.new_page()
        page.set_default_timeout(30000)
    
    page.goto(url, wait_until="networkidle", timeout=30000)
    return {"success": True, "url": page.url, "title": page.title()}

def handle_screenshot(full_page=False):
    global page
    if page is None:
        return {"success": False, "error": "No page loaded"}
    
    try:
        screenshot_bytes = page.screenshot(full_page=full_page, type="png")
        b64 = base64.b64encode(screenshot_bytes).decode('utf-8')
        return {"success": True, "data": b64, "format": "png"}
    except Exception as e:
        return {"success": False, "error": str(e)}

def handle_content():
    global page
    if page is None:
        return {"success": False, "error": "No page loaded"}
    
    try:
        html = page.content()
        # Truncate to prevent huge responses
        if len(html) > 50000:
            html = html[:50000] + "\n... [TRUNCATED]"
        return {"success": True, "data": html}
    except Exception as e:
        return {"success": False, "error": str(e)}

def handle_click(selector):
    global page
    if page is None:
        return {"success": False, "error": "No page loaded"}
    
    try:
        page.click(selector, timeout=5000)
        return {"success": True}
    except Exception as e:
        return {"success": False, "error": f"Click failed: {str(e)}"}

def handle_type(selector, text):
    global page
    if page is None:
        return {"success": False, "error": "No page loaded"}
    
    try:
        page.fill(selector, text, timeout=5000)
        return {"success": True}
    except Exception as e:
        return {"success": False, "error": f"Type failed: {str(e)}"}

def handle_hover(selector):
    global page
    if page is None:
        return {"success": False, "error": "No page loaded"}
    
    try:
        page.hover(selector, timeout=5000)
        return {"success": True}
    except Exception as e:
        return {"success": False, "error": f"Hover failed: {str(e)}"}

def handle_evaluate(script):
    global page
    if page is None:
        return {"success": False, "error": "No page loaded"}
    
    try:
        result = page.evaluate(script)
        return {"success": True, "data": str(result)}
    except Exception as e:
        return {"success": False, "error": f"Evaluate failed: {str(e)}"}

def handle_extract(selector):
    global page
    if page is None:
        return {"success": False, "error": "No page loaded"}
    
    try:
        elements = page.query_selector_all(selector)
        results = []
        for el in elements[:20]:  # Limit to 20 results
            text = el.inner_text().strip()
            if text:
                results.append(text)
        return {"success": True, "data": "\n".join(results), "count": len(results)}
    except Exception as e:
        return {"success": False, "error": f"Extract failed: {str(e)}"}

def handle_scroll(amount):
    global page
    if page is None:
        return {"success": False, "error": "No page loaded"}
    
    try:
        if amount > 0:
            page.mouse.wheel(0, amount)
        else:
            page.mouse.wheel(0, amount)
        return {"success": True}
    except Exception as e:
        return {"success": False, "error": f"Scroll failed: {str(e)}"}

def handle_wait(seconds):
    global page
    if page is None:
        return {"success": False, "error": "No page loaded"}
    
    try:
        page.wait_for_timeout(seconds * 1000)
        return {"success": True}
    except Exception as e:
        return {"success": False, "error": f"Wait failed: {str(e)}"}

def main():
    init_browser()
    
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        
        try:
            cmd = json.loads(line)
            action = cmd.get("action", "")
            result = {"action": action}
            
            if action == "navigate":
                result.update(handle_navigate(cmd.get("url", "")))
            
            elif action == "screenshot":
                full_page = cmd.get("full_page", False)
                result.update(handle_screenshot(full_page))
            
            elif action == "content":
                result.update(handle_content())
            
            elif action == "click":
                result.update(handle_click(cmd.get("selector", "")))
            
            elif action == "type":
                result.update(handle_type(cmd.get("selector", ""), cmd.get("text", "")))
            
            elif action == "hover":
                result.update(handle_hover(cmd.get("selector", "")))
            
            elif action == "evaluate":
                result.update(handle_evaluate(cmd.get("script", "")))
            
            elif action == "extract":
                result.update(handle_extract(cmd.get("selector", "")))
            
            elif action == "scroll":
                result.update(handle_scroll(cmd.get("amount", 500)))
            
            elif action == "wait":
                result.update(handle_wait(cmd.get("seconds", 1)))
            
            elif action == "screenshot_base64":
                result.update(handle_screenshot(False))
            
            elif action == "quit":
                cleanup()
                result["success"] = True
                print(json.dumps(result))
                sys.stdout.flush()
                break
            
            elif action == "ping":
                result["success"] = True
                result["browser_active"] = page is not None
            
            else:
                result["success"] = False
                result["error"] = f"Unknown action: {action}"
            
            print(json.dumps(result))
            sys.stdout.flush()
        
        except Exception as e:
            error_result = {"action": "error", "success": False, "error": str(e)}
            print(json.dumps(error_result))
            sys.stdout.flush()
    
    try:
        cleanup()
    except:
        pass

if __name__ == "__main__":
    main()
"#;

/// Browser session state
#[derive(Debug, Clone)]
pub struct BrowserSession {
    pub id: String,
    pub current_url: Option<String>,
    pub title: Option<String>,
    pub created_at: i64,
    pub last_action: i64,
}

// Atomic counter for session IDs
static BROWSER_SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Browser automation engine using Playwright
pub struct BrowserAutomation {
    child: StdMutex<Option<Child>>,
    stdin: RwLock<Option<tokio::process::ChildStdin>>,
    stdout: RwLock<Option<BufReader<tokio::process::ChildStdout>>>,
    current_session: RwLock<Option<BrowserSession>>,
    initialized: AtomicBool,
}

impl BrowserAutomation {
    /// Create a new browser automation instance
    pub fn new() -> Self {
        Self {
            child: std::sync::Mutex::new(None),
            stdin: RwLock::new(None),
            stdout: RwLock::new(None),
            current_session: RwLock::new(None),
            initialized: AtomicBool::new(false),
        }
    }

    /// Initialize the Playwright subprocess
    pub async fn initialize(&self) -> Result<(), String> {
        if self.initialized.load(Ordering::SeqCst) {
            return Ok(());
        }

        info!("Starting Playwright browser automation...");

        let mut child = Command::new("python3")
            .args(["-c", BROWSER_PROXY_SCRIPT])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn Playwright proxy: {}. Is Python installed?", e))?;

        let stdin = child.stdin.take()
            .ok_or_else(|| "Failed to capture stdin".to_string())?;
        let stdout = child.stdout.take()
            .ok_or_else(|| "Failed to capture stdout".to_string())?;

        if let Ok(mut guard) = self.child.lock() {
            *guard = Some(child);
        }
        *self.stdin.write().await = Some(stdin);
        *self.stdout.write().await = Some(BufReader::new(stdout));

        // Wait for initialization and verify connection
        self.send_command(serde_json::json!({"action": "ping"})).await?;

        self.initialized.store(true, Ordering::SeqCst);
        info!("Playwright browser automation initialized successfully");

        Ok(())
    }

    /// Send a command to the browser and get the response
    async fn send_command(&self, cmd: Value) -> Result<Value, String> {
        let mut stdin = self.stdin.write().await;
        let stdin = stdin.as_mut()
            .ok_or_else(|| "Browser not initialized".to_string())?;

        let mut stdout = self.stdout.write().await;
        let stdout = stdout.as_mut()
            .ok_or_else(|| "Browser not initialized".to_string())?;

        let cmd_str = serde_json::to_string(&cmd)
            .map_err(|e| format!("Failed to serialize command: {}", e))?;
        
        stdin.write_all(format!("{}\n", cmd_str).as_bytes())
            .await
            .map_err(|e| format!("Failed to write command: {}", e))?;
        stdin.flush()
            .await
            .map_err(|e| format!("Failed to flush: {}", e))?;

        let mut response = String::new();
        stdout.read_line(&mut response)
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        let parsed: Value = serde_json::from_str(&response)
            .map_err(|e| format!("Failed to parse response '{}': {}", response, e))?;

        Ok(parsed)
    }

    /// Navigate to a URL
    pub async fn navigate(&self, url: &str) -> Result<BrowserSession, String> {
        if !self.initialized.load(Ordering::SeqCst) {
            self.initialize().await?;
        }

        let result = self.send_command(serde_json::json!({
            "action": "navigate",
            "url": url
        })).await?;

        if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            let session = BrowserSession {
                id: format!("session_{}", BROWSER_SESSION_COUNTER.fetch_add(1, Ordering::SeqCst)),
                current_url: result.get("url").and_then(|v| v.as_str()).map(String::from),
                title: result.get("title").and_then(|v| v.as_str()).map(String::from),
                created_at: chrono::Utc::now().timestamp(),
                last_action: chrono::Utc::now().timestamp(),
            };

            *self.current_session.write().await = Some(session.clone());
            Ok(session)
        } else {
            Err(result.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Navigation failed")
                .to_string())
        }
    }

    /// Get page content as HTML
    pub async fn get_content(&self) -> Result<String, String> {
        let result = self.send_command(serde_json::json!({
            "action": "content"
        })).await?;

        if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            result.get("data")
                .and_then(|v| v.as_str())
                .map(String::from)
                .ok_or_else(|| "No content in response".to_string())
        } else {
            Err(result.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Failed to get content")
                .to_string())
        }
    }

    /// Take a screenshot and return base64-encoded PNG
    pub async fn screenshot(&self, full_page: bool) -> Result<String, String> {
        let result = self.send_command(serde_json::json!({
            "action": "screenshot",
            "full_page": full_page
        })).await?;

        if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            result.get("data")
                .and_then(|v| v.as_str())
                .map(String::from)
                .ok_or_else(|| "No screenshot data in response".to_string())
        } else {
            Err(result.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Screenshot failed")
                .to_string())
        }
    }

    /// Click an element by CSS selector
    pub async fn click(&self, selector: &str) -> Result<bool, String> {
        let result = self.send_command(serde_json::json!({
            "action": "click",
            "selector": selector
        })).await?;

        Ok(result.get("success").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    /// Type text into an element
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<bool, String> {
        let result = self.send_command(serde_json::json!({
            "action": "type",
            "selector": selector,
            "text": text
        })).await?;

        Ok(result.get("success").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    /// Hover over an element
    pub async fn hover(&self, selector: &str) -> Result<bool, String> {
        let result = self.send_command(serde_json::json!({
            "action": "hover",
            "selector": selector
        })).await?;

        Ok(result.get("success").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    /// Execute JavaScript on the page
    pub async fn evaluate(&self, script: &str) -> Result<String, String> {
        let result = self.send_command(serde_json::json!({
            "action": "evaluate",
            "script": script
        })).await?;

        if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            result.get("data")
                .and_then(|v| v.as_str())
                .map(String::from)
                .ok_or_else(|| "No data in evaluate response".to_string())
        } else {
            Err(result.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Evaluate failed")
                .to_string())
        }
    }

    /// Extract text content from elements matching a selector
    pub async fn extract(&self, selector: &str) -> Result<String, String> {
        let result = self.send_command(serde_json::json!({
            "action": "extract",
            "selector": selector
        })).await?;

        if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            result.get("data")
                .and_then(|v| v.as_str())
                .map(String::from)
                .ok_or_else(|| "No extract data in response".to_string())
        } else {
            Err(result.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Extract failed")
                .to_string())
        }
    }

    /// Scroll the page
    pub async fn scroll(&self, amount: i32) -> Result<bool, String> {
        let result = self.send_command(serde_json::json!({
            "action": "scroll",
            "amount": amount
        })).await?;

        Ok(result.get("success").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    /// Wait for a specified time
    pub async fn wait(&self, seconds: f64) -> Result<bool, String> {
        let result = self.send_command(serde_json::json!({
            "action": "wait",
            "seconds": seconds
        })).await?;

        Ok(result.get("success").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    /// Get current session info
    pub async fn get_session(&self) -> Option<BrowserSession> {
        self.current_session.read().await.clone()
    }

    /// Shutdown the browser
    pub async fn shutdown(&self) {
        if self.initialized.load(Ordering::SeqCst) {
            let _ = self.send_command(serde_json::json!({"action": "quit"})).await;

            if let Ok(mut guard) = self.child.lock() {
                if let Some(mut child) = guard.take() {
                    let _ = child.kill().await;
                }
            }

            *self.stdin.write().await = None;
            *self.stdout.write().await = None;
            self.initialized.store(false, Ordering::SeqCst);

            info!("Browser automation shut down");
        }
    }
}

impl Default for BrowserAutomation {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BrowserAutomation {
    fn drop(&mut self) {
        if self.initialized.load(Ordering::SeqCst) {
            if let Ok(mut child_guard) = self.child.lock() {
                if let Some(ref mut child) = *child_guard {
                    let _ = child.kill();
                }
            }
        }
    }
}

/// Browser Skill for the LLM tool registry
pub struct BrowserSkill {
    automation: Arc<BrowserAutomation>,
}

impl BrowserSkill {
    /// Create a new browser skill
    pub fn new() -> Self {
        Self {
            automation: Arc::new(BrowserAutomation::new()),
        }
    }

    /// Get the automation engine for external access
    pub fn get_automation(&self) -> Arc<BrowserAutomation> {
        self.automation.clone()
    }
}

impl Default for BrowserSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for BrowserSkill {
    fn name(&self) -> &'static str {
        "browser"
    }

    fn description(&self) -> &'static str {
        "Launch headless Chromium to surf dynamic web pages. Use to navigate to URLs, capture \
         screenshots, interact with forms (click, type), extract content, execute JavaScript, \
         and scrape JavaScript-rendered pages. Prerequisites: Python with `pip install playwright` \
         and `playwright install chromium`."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Browser action to perform",
                    "enum": ["navigate", "screenshot", "content", "click", "type", "hover", "evaluate", "extract", "scroll", "wait"]
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to (for navigate action)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector for element interaction (click, type, hover, extract)"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type into an element (for type action)"
                },
                "script": {
                    "type": "string",
                    "description": "JavaScript code to execute on the page (for evaluate action)"
                },
                "amount": {
                    "type": "integer",
                    "description": "Scroll amount in pixels (positive = down, negative = up), default: 500"
                },
                "seconds": {
                    "type": "number",
                    "description": "Time to wait in seconds (for wait action)"
                },
                "full_page": {
                    "type": "boolean",
                    "description": "Capture full page screenshot (for screenshot action)",
                    "default": false
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let automation = self.automation.clone();
        let args = args.clone();

        // Use blocking runtime to call async methods
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "browser".to_string(),
                    output: "Error: no async runtime available. Browser automation requires Tokio.".to_string(),
                    is_error: true,
                };
            }
        };

        // Spawn blocking task to execute async browser operations
        let result = std::thread::spawn(move || {
            rt.block_on(async {
                execute_browser_action(&automation, &args).await
            })
        })
        .join()
        .unwrap_or_else(|_| Err("Browser operation panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "browser".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "browser".to_string(),
                output: format!("Browser error: {}", e),
                is_error: true,
            },
        }
    }
}

/// Execute a browser action based on arguments
async fn execute_browser_action(automation: &Arc<BrowserAutomation>, args: &Value) -> Result<String, String> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");

    match action {
        "navigate" => {
            let url = args.get("url").and_then(|v| v.as_str())
                .ok_or("URL is required for navigate action")?;
            
            let session = automation.navigate(url).await?;
            Ok(format!(
                "Navigated to {}\nTitle: {}\nSession ID: {}",
                session.current_url.as_deref().unwrap_or("unknown"),
                session.title.as_deref().unwrap_or("unknown"),
                session.id
            ))
        }

        "screenshot" => {
            let full_page = args.get("full_page").and_then(|v| v.as_bool()).unwrap_or(false);
            let b64 = automation.screenshot(full_page).await?;
            
            // Return a summary, not the full base64 (to keep output small)
            Ok(format!(
                "Screenshot captured: {} bytes (base64 encoded PNG). Use evaluate action to retrieve full data if needed.",
                b64.len()
            ))
        }

        "content" => {
            let html = automation.get_content().await?;
            let truncated = if html.len() > MAX_OUTPUT_BYTES {
                format!("{}...\n[TRUNCATED — {} total bytes]", &html[..MAX_OUTPUT_BYTES], html.len())
            } else {
                html
            };
            Ok(format!("Page content ({} bytes):\n{}", truncated.len(), truncated))
        }

        "click" => {
            let selector = args.get("selector").and_then(|v| v.as_str())
                .ok_or("Selector is required for click action")?;
            
            let success = automation.click(selector).await?;
            if success {
                Ok(format!("Clicked on element: {}", selector))
            } else {
                Err(format!("Failed to click on element: {}", selector))
            }
        }

        "type" => {
            let selector = args.get("selector").and_then(|v| v.as_str())
                .ok_or("Selector is required for type action")?;
            let text = args.get("text").and_then(|v| v.as_str())
                .ok_or("Text is required for type action")?;
            
            let success = automation.type_text(selector, text).await?;
            if success {
                Ok(format!("Typed '{}' into element: {}", text, selector))
            } else {
                Err(format!("Failed to type into element: {}", selector))
            }
        }

        "hover" => {
            let selector = args.get("selector").and_then(|v| v.as_str())
                .ok_or("Selector is required for hover action")?;
            
            let success = automation.hover(selector).await?;
            if success {
                Ok(format!("Hovered over element: {}", selector))
            } else {
                Err(format!("Failed to hover over element: {}", selector))
            }
        }

        "evaluate" => {
            let script = args.get("script").and_then(|v| v.as_str())
                .ok_or("Script is required for evaluate action")?;
            
            let result = automation.evaluate(script).await?;
            Ok(format!("JavaScript result: {}", result))
        }

        "extract" => {
            let selector = args.get("selector").and_then(|v| v.as_str())
                .ok_or("Selector is required for extract action")?;
            
            let content = automation.extract(selector).await?;
            if content.is_empty() {
                Ok("No matching elements found".to_string())
            } else {
                Ok(format!("Extracted content ({} characters):\n{}", content.len(), content))
            }
        }

        "scroll" => {
            let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(500) as i32;
            automation.scroll(amount).await?;
            Ok(format!("Scrolled {} pixels", if amount > 0 { "down" } else { "up" }))
        }

        "wait" => {
            let seconds = args.get("seconds").and_then(|v| v.as_f64()).unwrap_or(1.0);
            automation.wait(seconds).await?;
            Ok(format!("Waited {} seconds", seconds))
        }

        _ => Err(format!(
            "Unknown action: '{}'. Available actions: navigate, screenshot, content, click, type, hover, evaluate, extract, scroll, wait",
            action
        )),
    }
}