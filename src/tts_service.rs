use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process;
use tokio::sync::RwLock;
use tracing::{info, warn};

const TTS_SERVER_SCRIPT: &str = r#"
import sys
import json
import base64
import io
try:
    from TTS.api import TTS
    tts = TTS(model_name="tts_models/multilingual/multi-dataset/xtts_v2", gpu=False)
    print("TTS_READY", flush=True)
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            cmd = json.loads(line)
            action = cmd.get("action", "")
            if action == "speak":
                text = cmd.get("text", "")
                language = cmd.get("language", "en")
                output_path = "/tmp/tts_output.wav"
                tts.tts_to_file(text=text, file_path=output_path, language=language)
                with open(output_path, "rb") as f:
                    audio_b64 = base64.b64encode(f.read()).decode()
                print(json.dumps({"status": "success", "audio": audio_b64}))
            elif action == "ping":
                print(json.dumps({"status": "ready"}))
            else:
                print(json.dumps({"status": "error", "message": f"Unknown action: {action}"}))
        except Exception as e:
            print(json.dumps({"status": "error", "message": str(e)}))
        sys.stdout.flush()
except ImportError:
    print("TTS_NOT_AVAILABLE: Coqui TTS not installed", flush=True)
    sys.exit(1)
"#;

pub struct TTSService {
    process: RwLock<Option<process::Child>>,
    stdin: RwLock<Option<tokio::process::ChildStdin>>,
    stdout: RwLock<Option<BufReader<tokio::process::ChildStdout>>>,
    available: RwLock<bool>,
}

impl TTSService {
    pub fn new() -> Self {
        Self {
            process: RwLock::new(None),
            stdin: RwLock::new(None),
            stdout: RwLock::new(None),
            available: RwLock::new(false),
        }
    }

    pub async fn start(&self) -> Result<(), String> {
        let mut child = process::Command::new("python3")
            .args(["-c", TTS_SERVER_SCRIPT])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn TTS server: {}", e))?;

        let stdin = child.stdin.take()
            .ok_or("Failed to capture TTS stdin")?;
        let stdout = child.stdout.take()
            .ok_or("Failed to capture TTS stdout")?;

        let mut stdout_reader = BufReader::new(stdout);
        let mut first_line = String::new();
        stdout_reader.read_line(&mut first_line).await
            .map_err(|e| format!("Failed to read TTS ready signal: {}", e))?;

        if first_line.trim() == "TTS_READY" {
            *self.process.write().await = Some(child);
            *self.stdin.write().await = Some(stdin);
            *self.stdout.write().await = Some(stdout_reader);
            *self.available.write().await = true;
            info!("TTS service started successfully");
            Ok(())
        } else if first_line.starts_with("TTS_NOT_AVAILABLE") {
            warn!("TTS not available: {}", first_line);
            *self.available.write().await = false;
            Ok(())
        } else {
            Err(format!("Unexpected TTS ready signal: {}", first_line))
        }
    }

    pub async fn speak(&self, text: &str, language: Option<&str>) -> Result<Vec<u8>, String> {
        let available = *self.available.read().await;
        if !available {
            return Err("TTS service not available. Install Coqui TTS: pip install TTS".to_string());
        }

        let mut stdin = self.stdin.write().await;
        let stdin = stdin.as_mut().ok_or("TTS stdin not available")?;
        let mut stdout = self.stdout.write().await;
        let stdout = stdout.as_mut().ok_or("TTS stdout not available")?;

        let cmd = serde_json::json!({
            "action": "speak",
            "text": text,
            "language": language.unwrap_or("en")
        });

        let cmd_str = serde_json::to_string(&cmd).map_err(|e| e.to_string())?;
        stdin.write_all(format!("{}\n", cmd_str).as_bytes()).await
            .map_err(|e| format!("Failed to send TTS command: {}", e))?;
        stdin.flush().await
            .map_err(|e| format!("Failed to flush TTS stdin: {}", e))?;

        let mut response = String::new();
        stdout.read_line(&mut response).await
            .map_err(|e| format!("Failed to read TTS response: {}", e))?;

        let resp: serde_json::Value = serde_json::from_str(&response)
            .map_err(|e| format!("Invalid TTS response: {}", e))?;

        if resp.get("status").and_then(|v| v.as_str()) == Some("success") {
            let audio_b64 = resp.get("audio").and_then(|v| v.as_str())
                .ok_or("No audio in TTS response")?;
            use base64::Engine;
            let audio_b64 = resp.get("audio").and_then(|v| v.as_str())
                .ok_or("No audio in TTS response")?;
            base64::engine::general_purpose::STANDARD.decode(audio_b64).map_err(|e| format!("Failed to decode audio: {}", e))
        } else {
            Err(resp.get("message").and_then(|v| v.as_str())
                .unwrap_or("Unknown TTS error")
                .to_string())
        }
    }

    pub async fn is_available(&self) -> bool {
        *self.available.read().await
    }

    pub async fn shutdown(&self) {
        if let Some(mut child) = self.process.write().await.take() {
            let _ = child.kill().await;
        }
        *self.stdin.write().await = None;
        *self.stdout.write().await = None;
        *self.available.write().await = false;
    }
}

impl Default for TTSService {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TTSService {
    fn drop(&mut self) {
        if let Ok(mut process) = self.process.try_write() {
            if let Some(ref mut child) = *process {
                let _ = child.kill();
            }
        }
    }
}