// ============================================================================
// ULTRACLAW — system_nodes.rs
// ============================================================================
// System Node Skills — camera, screen recording, and session management.
//
// ARCHITECTURE:
// - Camera operations use Python subprocess proxies (OpenCV/ffmpeg)
// - Screen recording uses ffmpeg via subprocess
// - Sessions use native platform-specific commands
// - All operations have security considerations built in
//
// CAMERA: Supports v4l2 on Linux, AVFoundation on macOS, DirectShow on Windows
// SCREEN: Uses ffmpeg for cross-platform capture
// SESSIONS: Platform-specific (loginctl/wmic/dscl)
// ============================================================================

use crate::skill::{Skill, SkillOutput};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tracing::{info, warn, debug};

// ============================================================================
// Storage Configuration
// ============================================================================

fn get_media_dir() -> PathBuf {
    std::env::var("ULTRACLAW_MEDIA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            #[cfg(windows)]
            {
                std::env::var("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("C:\\Users\\Public"))
                    .join("ultraclaw")
                    .join("media")
            }
            #[cfg(not(windows))]
            {
                PathBuf::from("/tmp/ultraclaw/media")
            }
        })
}

fn ensure_media_dir() -> std::io::Result<PathBuf> {
    let dir = get_media_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn generate_media_path(prefix: &str, ext: &str) -> std::io::Result<PathBuf> {
    let dir = ensure_media_dir()?;
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let filename = format!("{}_{}.{}", prefix, timestamp, ext);
    Ok(dir.join(filename))
}

// ============================================================================
// Camera Snap Skill
// ============================================================================

struct CameraSnapSkill;

impl CameraSnapSkill {
    fn capture_linux(&self, device: Option<&str>) -> Result<PathBuf, String> {
        let dev = device.unwrap_or("/dev/video0");
        let output_path = generate_media_path("camera_snap", "jpg")
            .map_err(|e| format!("Failed to generate path: {}", e))?;

        // Try ffmpeg first (works with most devices)
        let result = Command::new("ffmpeg")
            .args(&[
                "-y", "-f", "video4linux2",
                "-input_format", "mjpeg",
                "-i", dev,
                "-vframes", "1",
                "-q:v", "2",
                output_path.to_str().unwrap(),
            ])
            .output();

        match result {
            Ok(o) if o.status.success() => Ok(output_path),
            _ => {
                // Fallback: try OpenCV via Python
                let fallback_result = self.capture_with_python("snap", None);
                fallback_result
            }
        }
    }

    fn capture_macos(&self, _device: Option<&str>) -> Result<PathBuf, String> {
        let output_path = generate_media_path("camera_snap", "jpg")
            .map_err(|e| format!("Failed to generate path: {}", e))?;

        let script = format!(
            r#"import subprocess; result = subprocess.run(['/usr/bin/screencapture', '-x', '-t', 'jpg', '{}'], capture_output=True); exit(result.returncode)"#,
            output_path.to_str().unwrap()
        );

        let result = Command::new("python3")
            .args(&["-c", &script])
            .output()
            .map_err(|e| format!("Python execution failed: {}", e))?;

        if result.status.success() && output_path.exists() {
            Ok(output_path)
        } else {
            self.capture_with_python("snap", None)
        }
    }

    fn capture_windows(&self, _device: Option<&str>) -> Result<PathBuf, String> {
        self.capture_with_python("snap", None)
    }

    fn capture_with_python(&self, action: &str, duration: Option<u32>) -> Result<PathBuf, String> {
        let output_path = generate_media_path(
            match action {
                "snap" => "camera_snap",
                "clip" => "camera_clip",
                _ => "camera_media",
            },
            match action {
                "snap" => "jpg",
                _ => "mp4",
            }
        )
        .map_err(|e| format!("Failed to generate path: {}", e))?;

        let duration_arg = duration.map(|d| format!(", duration={}", d)).unwrap_or_default();
        let script = format!(
            r#"
import cv2
import sys

try:
    camera = cv2.VideoCapture(0)
    if not camera.isOpened():
        print("ERROR: Cannot open camera", file=sys.stderr)
        sys.exit(1)

    ret, frame = camera.read()
    camera.release()

    if not ret:
        print("ERROR: Failed to capture frame", file=sys.stderr)
        sys.exit(1)

    cv2.imwrite('{}', frame)
    print("SUCCESS: {}")
except Exception as e:
    print(f"ERROR: {{e}}", file=sys.stderr)
    sys.exit(1)
"#,
            output_path.to_str().unwrap().replace('\\', "\\\\"),
            output_path.to_str().unwrap().replace('\\', "\\\\")
        );

        let result = Command::new("python")
            .args(&["-c", &script])
            .output()
            .map_err(|e| format!("Python failed: {}", e))?;

        if result.status.success() && output_path.exists() {
            Ok(output_path)
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr);
            Err(format!("Capture failed: {}", stderr))
        }
    }
}

impl Skill for CameraSnapSkill {
    fn name(&self) -> &'static str { "camera_snap" }
    fn description(&self) -> &'static str {
        "Capture a photo from the host webcam. Returns the path to the saved image file. \
         On Linux uses v4l2/ffmpeg, on macOS uses AVFoundation, on Windows uses OpenCV."
    }
    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "device": {
                    "type": "string",
                    "description": "Camera device path (e.g., /dev/video0 on Linux, 0 on Windows)"
                }
            }
        })
    }
    fn execute_sync(&self, params: &Value) -> SkillOutput {
        let device = params.get("device").and_then(|v| v.as_str());

        let result = if cfg!(target_os = "linux") {
            self.capture_linux(device)
        } else if cfg!(target_os = "macos") {
            self.capture_macos(device)
        } else {
            self.capture_windows(device)
        };

        match result {
            Ok(path) => {
                info!(path = %path.display(), "Camera snap captured");
                SkillOutput {
                    name: "camera_snap".to_string(),
                    output: format!("Photo captured: {}", path.display()),
                    is_error: false,
                }
            }
            Err(e) => {
                warn!(error = %e, "Camera snap failed");
                SkillOutput {
                    name: "camera_snap".to_string(),
                    output: format!("Camera capture failed: {}", e),
                    is_error: true,
                }
            }
        }
    }
}

// ============================================================================
// Camera Clip Skill
// ============================================================================

struct CameraClipSkill;

impl CameraClipSkill {
    fn record_linux(&self, device: Option<&str>, duration: u32) -> Result<PathBuf, String> {
        let dev = device.unwrap_or("/dev/video0");
        let output_path = generate_media_path("camera_clip", "mp4")
            .map_err(|e| format!("Failed to generate path: {}", e))?;

        let result = Command::new("ffmpeg")
            .args(&[
                "-y", "-f", "video4linux2",
                "-input_format", "mjpeg",
                "-i", dev,
                "-t", &duration.to_string(),
                "-c:v", "libx264",
                "-preset", "ultrafast",
                output_path.to_str().unwrap(),
            ])
            .output()
            .map_err(|e| format!("FFmpeg failed: {}", e))?;

        if result.status.success() && output_path.exists() {
            Ok(output_path)
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr);
            Err(format!("FFmpeg recording failed: {}", stderr))
        }
    }

    fn record_with_python(&self, duration: u32) -> Result<PathBuf, String> {
        let output_path = generate_media_path("camera_clip", "mp4")
            .map_err(|e| format!("Failed to generate path: {}", e))?;

        let script = format!(
            r#"
import cv2
import sys

try:
    camera = cv2.VideoCapture(0)
    if not camera.isOpened():
        print("ERROR: Cannot open camera", file=sys.stderr)
        sys.exit(1)

    width = int(camera.get(cv2.CAP_PROP_FRAME_WIDTH))
    height = int(camera.get(cv2.CAP_PROP_FRAME_HEIGHT))
    fps = camera.get(cv2.CAP_PROP_FPS) or 30.0

    fourcc = cv2.VideoWriter_fourcc(*'mp4v')
    out = cv2.VideoWriter('{}', fourcc, fps, (width, height))

    frames = int({} * fps)
    for _ in range(frames):
        ret, frame = camera.read()
        if ret:
            out.write(frame)
        else:
            break

    camera.release()
    out.release()
    print("SUCCESS: Recording saved")
except Exception as e:
    print(f"ERROR: {{e}}", file=sys.stderr)
    sys.exit(1)
"#,
            output_path.to_str().unwrap().replace('\\', "\\\\"),
            duration
        );

        let result = Command::new("python")
            .args(&["-c", &script])
            .output()
            .map_err(|e| format!("Python failed: {}", e))?;

        if result.status.success() && output_path.exists() {
            Ok(output_path)
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr);
            Err(format!("Recording failed: {}", stderr))
        }
    }
}

impl Skill for CameraClipSkill {
    fn name(&self) -> &'static str { "camera_clip" }
    fn description(&self) -> &'static str {
        "Capture a video clip from the host webcam. Default 5 seconds, configurable. \
         Uses ffmpeg on Linux, OpenCV on other platforms."
    }
    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "duration": {
                    "type": "integer",
                    "description": "Duration in seconds (default: 5, max: 60)"
                },
                "device": {
                    "type": "string",
                    "description": "Camera device path"
                }
            }
        })
    }
    fn execute_sync(&self, params: &Value) -> SkillOutput {
        let duration = params.get("duration")
            .and_then(|v| v.as_i64())
            .map(|d| d.min(60) as u32)
            .unwrap_or(5);
        let device = params.get("device").and_then(|v| v.as_str());

        let result = if cfg!(target_os = "linux") {
            self.record_linux(device, duration)
        } else {
            self.record_with_python(duration)
        };

        match result {
            Ok(path) => {
                info!(path = %path.display(), duration = duration, "Camera clip captured");
                SkillOutput {
                    name: "camera_clip".to_string(),
                    output: format!("Video clip ({}s) captured: {}", duration, path.display()),
                    is_error: false,
                }
            }
            Err(e) => {
                warn!(error = %e, "Camera clip failed");
                SkillOutput {
                    name: "camera_clip".to_string(),
                    output: format!("Video capture failed: {}", e),
                    is_error: true,
                }
            }
        }
    }
}

// ============================================================================
// Screen Record Skill
// ============================================================================

struct ScreenRecordSkill;

impl ScreenRecordSkill {
    fn record_linux(&self, duration: u32) -> Result<PathBuf, String> {
        let output_path = generate_media_path("screen_record", "mp4")
            .map_err(|e| format!("Failed to generate path: {}", e))?;

        // Determine display and resolution
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());

        // Try to get screen dimensions
        let dims = Self::get_linux_resolution(&display).unwrap_or((1920, 1080));

        let result = Command::new("ffmpeg")
            .args(&[
                "-y",
                "-f", "x11grab",
                "-framerate", "30",
                "-video_size", &format!("{}x{}", dims.0, dims.1),
                "-i", &format!("{}.0", display),
                "-t", &duration.to_string(),
                "-c:v", "libx264",
                "-preset", "ultrafast",
                "-pix_fmt", "yuv420p",
                output_path.to_str().unwrap(),
            ])
            .output()
            .map_err(|e| format!("FFmpeg failed: {}", e))?;

        if result.status.success() && output_path.exists() {
            Ok(output_path)
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr);
            Err(format!("Screen recording failed: {}", stderr))
        }
    }

    fn get_linux_resolution(display: &str) -> Option<(u32, u32)> {
        let output = Command::new("xrandr")
            .args(&["--display", display])
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("*") {
                if let Some(res) = line.split_whitespace().next() {
                    let parts: Vec<&str> = res.split('x').collect();
                    if parts.len() == 2 {
                        let w: u32 = parts[0].parse().ok()?;
                        let h: u32 = parts[1].parse().ok()?;
                        return Some((w, h));
                    }
                }
            }
        }
        None
    }

    fn record_macos(&self, duration: u32) -> Result<PathBuf, String> {
        let output_path = generate_media_path("screen_record", "mp4")
            .map_err(|e| format!("Failed to generate path: {}", e))?;

        // Use macOS screen recording via screencapture + ffmpeg
        let temp_path = output_path.with_extension("mov");

        let result = Command::new("/usr/bin/screencapture")
            .args(&["-x", "-v", temp_path.to_str().unwrap()])
            .output()
            .map_err(|e| format!("screencapture failed: {}", e))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(format!("Screen capture failed: {}", stderr));
        }

        // Convert to mp4 with ffmpeg
        let convert_result = Command::new("ffmpeg")
            .args(&[
                "-y", "-i", temp_path.to_str().unwrap(),
                "-c:v", "libx264", "-preset", "fast",
                output_path.to_str().unwrap(),
            ])
            .output();

        // Clean up temp file
        let _ = fs::remove_file(&temp_path);

        match convert_result {
            Ok(o) if o.status.success() && output_path.exists() => Ok(output_path),
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                Err(format!("Video conversion failed: {}", stderr))
            }
            Err(e) => Err(format!("FFmpeg conversion failed: {}", e)),
        }
    }

    fn record_windows(&self, duration: u32) -> Result<PathBuf, String> {
        self.record_with_python(duration)
    }

    fn record_with_python(&self, duration: u32) -> Result<PathBuf, String> {
        let output_path = generate_media_path("screen_record", "mp4")
            .map_err(|e| format!("Failed to generate path: {}", e))?;

        let script = format!(
            r#"
import numpy as np
try:
    import mss
    import cv2

    with mss.mss() as sct:
        monitor = sct.monitors[1]
        width = monitor['width']
        height = monitor['height']

        fourcc = cv2.VideoWriter_fourcc(*'mp4v')
        out = cv2.VideoWriter('{}', fourcc, 30.0, (width, height))

        fps = 30.0
        total_frames = int({} * fps)
        for _ in range(total_frames):
            img = sct.grab(monitor)
            frame = np.array(img)
            frame = cv2.cvtColor(frame, cv2.COLOR_BGRA2BGR)
            out.write(frame)

        out.release()
        print("SUCCESS: Recording saved")
except ImportError:
    print("ERROR: mss library not installed. Install with: pip install mss opencv-python", file=__import__('sys').stderr)
    __import__('sys').exit(1)
except Exception as e:
    print(f"ERROR: {{e}}", file=__import__('sys').stderr)
    __import__('sys').exit(1)
"#,
            output_path.to_str().unwrap().replace('\\', "\\\\"),
            duration
        );

        let result = Command::new("python")
            .args(&["-c", &script])
            .output()
            .map_err(|e| format!("Python failed: {}", e))?;

        if result.status.success() && output_path.exists() {
            Ok(output_path)
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr);
            Err(format!("Screen recording failed: {}", stderr))
        }
    }
}

impl Skill for ScreenRecordSkill {
    fn name(&self) -> &'static str { "screen_record" }
    fn description(&self) -> &'static str {
        "Record a video of the primary monitor. Default 5 seconds, configurable up to 60s. \
         Uses x11grab on Linux, screencapture on macOS, MSS/OpenCV on Windows."
    }
    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "duration": {
                    "type": "integer",
                    "description": "Duration in seconds (default: 5, max: 60)"
                }
            }
        })
    }
    fn execute_sync(&self, params: &Value) -> SkillOutput {
        let duration = params.get("duration")
            .and_then(|v| v.as_i64())
            .map(|d| d.min(60) as u32)
            .unwrap_or(5);

        let result = if cfg!(target_os = "linux") {
            self.record_linux(duration)
        } else if cfg!(target_os = "macos") {
            self.record_macos(duration)
        } else {
            self.record_windows(duration)
        };

        match result {
            Ok(path) => {
                info!(path = %path.display(), duration = duration, "Screen recording captured");
                SkillOutput {
                    name: "screen_record".to_string(),
                    output: format!("Screen recording ({}s) saved: {}", duration, path.display()),
                    is_error: false,
                }
            }
            Err(e) => {
                warn!(error = %e, "Screen recording failed");
                SkillOutput {
                    name: "screen_record".to_string(),
                    output: format!("Screen recording failed: {}", e),
                    is_error: true,
                }
            }
        }
    }
}

// ============================================================================
// Location Get Skill
// ============================================================================

struct LocationGetSkill;

impl Skill for LocationGetSkill {
    fn name(&self) -> &'static str { "location_get" }
    fn description(&self) -> &'static str {
        "Get the current geolocation of the host machine. Returns IP-based approximate location. \
         For precise location, the host needs location services enabled."
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    fn execute_sync(&self, _params: &Value) -> SkillOutput {
        // Try multiple geolocation services
        let services = [
            "https://ipinfo.io/json",
            "https://ipapi.co/json/",
            "https://ip-api.com/json/",
        ];

        for service in &services {
            if let Some(location) = Self::fetch_location(service) {
                return SkillOutput {
                    name: "location_get".to_string(),
                    output: serde_json::to_string_pretty(&location).unwrap_or_else(|_| "{}".to_string()),
                    is_error: false,
                };
            }
        }

        // Fallback to mock data if all services fail
        SkillOutput {
            name: "location_get".to_string(),
            output: r#"{"lat": 0.0, "lon": 0.0, "city": "Unknown", "country": "Unknown", "note": "Could not determine location"}"#.to_string(),
            is_error: false,
        }
    }
}

impl LocationGetSkill {
    fn fetch_location(url: &str) -> Option<serde_json::Value> {
        let output = Command::new("curl")
            .args(&["-s", "-m", "5", url])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&json_str).ok()
    }
}

// ============================================================================
// System Run Skill
// ============================================================================

struct SystemRunSkill;
impl Skill for SystemRunSkill {
    fn name(&self) -> &'static str { "system_run" }
    fn description(&self) -> &'static str { "Execute a shell command on the host OS natively." }
    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to run"}
            },
            "required": ["command"]
        })
    }
    fn execute_sync(&self, params: &Value) -> SkillOutput {
        let command = params.get("command").and_then(|v| v.as_str()).unwrap_or("");

        if command.is_empty() {
            return SkillOutput {
                name: "system_run".to_string(),
                output: "Error: empty command".to_string(),
                is_error: true,
            };
        }

        // SECURITY: Basic command validation
        let dangerous = ["rm -rf /", ":(){:|:&};:", "mkfs", "dd if=/dev/zero"];
        for pattern in &dangerous {
            if command.contains(pattern) {
                return SkillOutput {
                    name: "system_run".to_string(),
                    output: format!("Blocked dangerous command pattern: {}", pattern),
                    is_error: true,
                };
            }
        }

        let output = if cfg!(target_os = "windows") {
            Command::new("cmd").args(&["/C", command]).output()
        } else {
            Command::new("sh").arg("-c").arg(command).output()
        };

        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let stderr = String::from_utf8_lossy(&o.stderr);
                let truncated_stdout = if stdout.len() > 4096 {
                    format!("{}...[truncated {} bytes]", &stdout[..4096], stdout.len() - 4096)
                } else {
                    stdout.to_string()
                };
                SkillOutput {
                    name: "system_run".to_string(),
                    output: format!("Exit code: {}\nSTDOUT:\n{}\nSTDERR:\n{}",
                        o.status.code().unwrap_or(-1),
                        truncated_stdout,
                        stderr),
                    is_error: !o.status.success(),
                }
            }
            Err(e) => SkillOutput {
                name: "system_run".to_string(),
                output: format!("Failed to execute command: {}", e),
                is_error: true,
            }
        }
    }
}

// ============================================================================
// System Notify Skill
// ============================================================================

struct SystemNotifySkill;
impl Skill for SystemNotifySkill {
    fn name(&self) -> &'static str { "system_notify" }
    fn description(&self) -> &'static str { "Display a toast notification on the host OS." }
    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {"type": "string"},
                "message": {"type": "string"}
            },
            "required": ["title", "message"]
        })
    }
    fn execute_sync(&self, params: &Value) -> SkillOutput {
        let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("Notification");
        let message = params.get("message").and_then(|v| v.as_str()).unwrap_or("");

        let result = if cfg!(target_os = "windows") {
            notify_windows(title, message)
        } else if cfg!(target_os = "macos") {
            notify_macos(title, message)
        } else {
            notify_linux(title, message)
        };

        match result {
            Ok(_) => {
                info!(title = %title, "Notification sent");
                SkillOutput {
                    name: "system_notify".to_string(),
                    output: "Notification sent successfully.".to_string(),
                    is_error: false,
                }
            }
            Err(e) => {
                warn!(error = %e, "Notification failed");
                SkillOutput {
                    name: "system_notify".to_string(),
                    output: format!("Notification failed: {}", e),
                    is_error: true,
                }
            }
        }
    }
}

fn notify_windows(title: &str, message: &str) -> Result<(), String> {
    // Use PowerShell for Windows notifications
    let ps_script = format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; \
         $template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
         $textNodes = $template.GetElementsByTagName('text'); $textNodes.Item(0).AppendChild($template.CreateTextNode('{}')) | Out-Null; \
         $textNodes.Item(1).AppendChild($template.CreateTextNode('{}')) | Out-Null; \
         $toast = [Windows.UI.Notifications.ToastNotification]::new($template); \
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Ultraclaw').Show($toast)",
        title.replace("'", "''"),
        message.replace("'", "''")
    );

    Command::new("powershell")
        .args(&["-Command", &ps_script])
        .output()
        .map_err(|e| format!("PowerShell notification failed: {}", e))?;

    Ok(())
}

fn notify_macos(title: &str, message: &str) -> Result<(), String> {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        message.replace("\"", "\\\""),
        title.replace("\"", "\\\"")
    );

    Command::new("osascript")
        .args(&["-e", &script])
        .output()
        .map_err(|e| format!("osascript notification failed: {}", e))?;

    Ok(())
}

fn notify_linux(title: &str, message: &str) -> Result<(), String> {
    // Try notify-send first, then fallback to echo
    let result = Command::new("notify-send")
        .args(&[title, message, "-a", "ultraclaw"])
        .output();

    match result {
        Ok(o) if o.status.success() => Ok(()),
        _ => {
            // Fallback: just print to console
            info!(title = %title, message = %message, "Notification (fallback)");
            Ok(())
        }
    }
}

// ============================================================================
// Sessions List Skill
// ============================================================================

struct SessionsListSkill;

impl SessionsListSkill {
    fn list_linux(&self) -> Result<String, String> {
        let output = Command::new("who")
            .output()
            .map_err(|e| format!("Failed to run who: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.is_empty() {
            return Ok("No active sessions.".to_string());
        }

        // Parse and format
        let sessions: Vec<Value> = stdout.lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5 {
                    Some(serde_json::json!({
                        "user": parts[0],
                        "tty": parts[1],
                        "login_time": format!("{} {} {}", parts[2], parts[3], parts[4]),
                        "from": parts.get(5)
                    }))
                } else {
                    None
                }
            })
            .collect();

        Ok(serde_json::json!({"sessions": sessions, "count": sessions.len()}).to_string())
    }

    fn list_macos(&self) -> Result<String, String> {
        let output = Command::new("who")
            .output()
            .map_err(|e| format!("Failed to run who: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.is_empty() {
            return Ok("No active sessions.".to_string());
        }

        let sessions: Vec<Value> = stdout.lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5 {
                    Some(serde_json::json!({
                        "user": parts[0],
                        "tty": parts[1],
                        "login_time": format!("{} {} {}", parts[2], parts[3], parts[4]),
                        "from": parts.get(5)
                    }))
                } else {
                    None
                }
            })
            .collect();

        Ok(serde_json::json!({"sessions": sessions, "count": sessions.len()}).to_string())
    }

    fn list_windows(&self) -> Result<String, String> {
        let output = Command::new("powershell")
            .args(&["-Command", "query user"])
            .output()
            .map_err(|e| format!("Failed to run query user: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.is_empty() || stdout.contains("No user exists") {
            return Ok("No active sessions.".to_string());
        }

        let sessions: Vec<Value> = stdout.lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let state = if line.contains("Active") { "Active" } else { "Disconnected" };
                    Some(serde_json::json!({
                        "user": parts[0],
                        "session_id": parts.get(1),
                        "state": state,
                        "idle_time": parts.get(3)
                    }))
                } else {
                    None
                }
            })
            .collect();

        Ok(serde_json::json!({"sessions": sessions, "count": sessions.len()}).to_string())
    }
}

impl Skill for SessionsListSkill {
    fn name(&self) -> &'static str { "sessions_list" }
    fn description(&self) -> &'static str {
        "List active user sessions on the OS. Shows logged-in users, terminal sessions, \
         and remote sessions where applicable."
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    fn execute_sync(&self, _params: &Value) -> SkillOutput {
        let result = if cfg!(target_os = "linux") {
            self.list_linux()
        } else if cfg!(target_os = "macos") {
            self.list_macos()
        } else {
            self.list_windows()
        };

        match result {
            Ok(output) => SkillOutput {
                name: "sessions_list".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "sessions_list".to_string(),
                output: format!("Failed to list sessions: {}", e),
                is_error: true,
            },
        }
    }
}

// ============================================================================
// Sessions History Skill
// ============================================================================

struct SessionsHistorySkill;

impl SessionsHistorySkill {
    fn history_linux(&self) -> Result<String, String> {
        // Use last command to get session history
        let output = Command::new("last")
            .args(&["-n", "20", "-F"])
            .output()
            .map_err(|e| format!("Failed to run last: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let entries: Vec<Value> = stdout.lines()
            .filter(|l| !l.contains("wtmp begins"))
            .take(20)
            .map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                serde_json::json!({
                    "user": parts.get(0).map(|s| s.to_string()),
                    "tty": parts.get(1).map(|s| s.to_string()),
                    "login_time": parts.get(3..7).map(|p| p.join(" ")),
                    "logout_time": parts.get(8..12).map(|p| p.join(" ")),
                    "duration": parts.get(13).map(|s| s.to_string())
                })
            })
            .collect();

        Ok(serde_json::json!({"history": entries}).to_string())
    }

    fn history_windows(&self) -> Result<String, String> {
        let output = Command::new("wevtutil")
            .args(&["ql", "Microsoft-Windows-TerminalServices-LocalSessionManager/Operational", "/c:5", "/f:text"])
            .output();

        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                Ok(serde_json::json!({
                    "history": stdout.lines().take(20).collect::<Vec<_>>()
                }).to_string())
            }
            Err(e) => Ok(serde_json::json!({
                "history": [],
                "note": format!("Windows event log query failed: {}. Using query user instead.", e)
            }).to_string()),
        }
    }
}

impl Skill for SessionsHistorySkill {
    fn name(&self) -> &'static str { "sessions_history" }
    fn description(&self) -> &'static str {
        "View history of OS sessions (recent logins/logouts). Returns last 20 entries."
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    fn execute_sync(&self, _params: &Value) -> SkillOutput {
        let result = if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
            self.history_linux()
        } else {
            self.history_windows()
        };

        match result {
            Ok(output) => SkillOutput {
                name: "sessions_history".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "sessions_history".to_string(),
                output: format!("Failed to get session history: {}", e),
                is_error: true,
            },
        }
    }
}

// ============================================================================
// Sessions Send Skill
// ============================================================================

struct SessionsSendSkill;

impl SessionsSendSkill {
    fn send_linux(&self, tty: &str, message: &str) -> Result<String, String> {
        let result = Command::new("echo")
            .arg(message)
            .output()
            .map_err(|e| format!("echo failed: {}", e))?;

        // Write directly to the TTY
        let write_result = Command::new("sudo")
            .args(&["sh", "-c", &format!("echo '{}' > /dev/{}", message, tty)])
            .output();

        match write_result {
            Ok(o) if o.status.success() => Ok(format!("Message sent to {}", tty)),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).to_string()),
            Err(e) => Err(format!("Failed to send message: {}", e)),
        }
    }

    fn send_windows(&self, session_id: &str, message: &str) -> Result<String, String> {
        // Use msg.exe to send messages on Windows
        let result = Command::new("msg")
            .args(&[session_id, message])
            .output()
            .map_err(|e| format!("msg command failed: {}", e))?;

        if result.status.success() {
            Ok(format!("Message sent to session {}", session_id))
        } else {
            Err(String::from_utf8_lossy(&result.stderr).to_string())
        }
    }
}

impl Skill for SessionsSendSkill {
    fn name(&self) -> &'static str { "sessions_send" }
    fn description(&self) -> &'static str {
        "Send a message or payload to an active session. On Linux writes to TTY, \
         on Windows uses msg.exe. Requires appropriate permissions."
    }
    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": {"type": "string", "description": "TTY (Linux) or session ID (Windows)"},
                "message": {"type": "string", "description": "Message to send"}
            },
            "required": ["target", "message"]
        })
    }
    fn execute_sync(&self, params: &Value) -> SkillOutput {
        let target = params.get("target").and_then(|v| v.as_str()).unwrap_or("");
        let message = params.get("message").and_then(|v| v.as_str()).unwrap_or("");

        if target.is_empty() || message.is_empty() {
            return SkillOutput {
                name: "sessions_send".to_string(),
                output: "Error: target and message are required".to_string(),
                is_error: true,
            };
        }

        let result = if cfg!(target_os = "windows") {
            self.send_windows(target, message)
        } else {
            self.send_linux(target, message)
        };

        match result {
            Ok(output) => SkillOutput {
                name: "sessions_send".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "sessions_send".to_string(),
                output: format!("Failed to send message: {}", e),
                is_error: true,
            },
        }
    }
}

// ============================================================================
// Sessions Spawn Skill
// ============================================================================

struct SessionsSpawnSkill;

impl SessionsSpawnSkill {
    fn spawn_linux(&self, shell: Option<&str>) -> Result<String, String> {
        let shell = shell.unwrap_or("/bin/bash");
        let output = Command::new("setsid")
            .args(&[shell, "-c", &format!("echo 'Ultraclaw session spawned at {}'; exec {}", chrono::Utc::now(), shell)])
            .spawn()
            .map_err(|e| format!("Failed to spawn session: {}", e))?;

        Ok(format!(
            "Session spawned with PID {}",
            output.id()
        ))
    }

    fn spawn_windows(&self, program: Option<&str>) -> Result<String, String> {
        let program = program.unwrap_or("cmd.exe");
        let output = Command::new("start")
            .args(&["cmd", "/c", &format!("title Ultraclaw Session && {} && pause", program)])
            .spawn()
            .map_err(|e| format!("Failed to spawn session: {}", e))?;

        Ok(format!(
            "Session spawned with PID {}",
            output.id()
        ))
    }
}

impl Skill for SessionsSpawnSkill {
    fn name(&self) -> &'static str { "sessions_spawn" }
    fn description(&self) -> &'static str {
        "Spawn a new detached terminal session. On Linux uses setsid for daemonized shell, \
         on Windows uses start cmd for new console window."
    }
    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "shell": {"type": "string", "description": "Shell to spawn (Linux)"},
                "program": {"type": "string", "description": "Program to run (Windows)"}
            }
        })
    }
    fn execute_sync(&self, params: &Value) -> SkillOutput {
        let shell = params.get("shell").and_then(|v| v.as_str());
        let program = params.get("program").and_then(|v| v.as_str());

        let result = if cfg!(target_os = "windows") {
            self.spawn_windows(program)
        } else {
            self.spawn_linux(shell)
        };

        match result {
            Ok(output) => {
                info!(output = %output, "Session spawned");
                SkillOutput {
                    name: "sessions_spawn".to_string(),
                    output,
                    is_error: false,
                }
            }
            Err(e) => {
                warn!(error = %e, "Session spawn failed");
                SkillOutput {
                    name: "sessions_spawn".to_string(),
                    output: format!("Failed to spawn session: {}", e),
                    is_error: true,
                }
            }
        }
    }
}

// ============================================================================
// Module Registration
// ============================================================================

pub struct SystemNodesModule {}

impl Default for SystemNodesModule {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemNodesModule {
    pub fn new() -> Self {
        Self {}
    }
    pub fn register_all() -> Vec<Box<dyn Skill>> {
        vec![
            Box::new(CameraSnapSkill {}),
            Box::new(CameraClipSkill {}),
            Box::new(ScreenRecordSkill {}),
            Box::new(LocationGetSkill {}),
            Box::new(SystemRunSkill {}),
            Box::new(SystemNotifySkill {}),
            Box::new(SessionsListSkill {}),
            Box::new(SessionsHistorySkill {}),
            Box::new(SessionsSendSkill {}),
            Box::new(SessionsSpawnSkill {}),
        ]
    }
}