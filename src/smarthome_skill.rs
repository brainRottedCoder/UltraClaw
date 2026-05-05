// ============================================================================
// ULTRACLAW — smarthome_skill.rs
// ============================================================================
// Smart home device control via Home Assistant, Sonos, and Philips Hue.
//
// This module provides unified smart home control across multiple platforms:
// - Home Assistant: Control any entity (lights, switches, climate, media)
// - Philips Hue: Direct control of Hue lights and bridges
// - Sonos: Control Sonos speakers (play, pause, volume, music)
//
// ARCHITECTURE:
// - Home Assistant is the primary integration (REST API with long-lived tokens)
// - Hue and Sonos have direct HTTP/SoCo integrations as fallbacks
// - All calls are async using reqwest with proper error handling
//
// SECURITY:
// - No cloud dependencies for local Home Assistant instances
// - Token-based authentication (not username/password)
// - Network isolation via Home Assistant's own permissions
//
// CONFIGURATION:
// ULTRACLAW_HOMEASSISTANT_URL=http://homeassistant.local:8123
// ULTRACLAW_HOMEASSISTANT_TOKEN=your_long_lived_access_token
// ULTRACLAW_SONOS_IPS=192.168.1.100,192.168.1.101
// ULTRACLAW_HUE_BRIDGE_IP=192.168.1.50
// ULTRACLAW_HUE_API_KEY=your_hue_api_key
// ============================================================================

use crate::skill::{Skill, SkillOutput};
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, warn, error};

/// Smart home configuration
#[derive(Debug, Clone)]
pub struct SmartHomeConfig {
    pub homeassistant_url: String,
    pub homeassistant_token: String,
    pub sonos_ips: Vec<String>,
    pub hue_bridge_ip: Option<String>,
    pub hue_api_key: Option<String>,
}

impl Default for SmartHomeConfig {
    fn default() -> Self {
        Self {
            homeassistant_url: std::env::var("ULTRACLAW_HOMEASSISTANT_URL")
                .unwrap_or_else(|_| "http://homeassistant.local:8123".to_string()),
            homeassistant_token: std::env::var("ULTRACLAW_HOMEASSISTANT_TOKEN")
                .unwrap_or_default(),
            sonos_ips: std::env::var("ULTRACLAW_SONOS_IPS")
                .map(|s| s.split(',').map(|ip| ip.trim().to_string()).collect())
                .unwrap_or_default(),
            hue_bridge_ip: std::env::var("ULTRACLAW_HUE_BRIDGE_IP").ok(),
            hue_api_key: std::env::var("ULTRACLAW_HUE_API_KEY").ok(),
        }
    }
}

impl SmartHomeConfig {
    pub fn is_configured(&self) -> bool {
        !self.homeassistant_token.is_empty() ||
        !self.sonos_ips.is_empty() ||
        (self.hue_bridge_ip.is_some() && self.hue_api_key.is_some())
    }
}

/// Smart home device control skill
pub struct SmartHomeSkill {
    client: Client,
    config: SmartHomeConfig,
    ha_available: bool,
}

impl SmartHomeSkill {
    /// Create a new SmartHome skill with configuration from environment
    pub fn new() -> Self {
        let config = SmartHomeConfig::default();
        let ha_available = !config.homeassistant_token.is_empty();

        if !config.is_configured() {
            warn!("SmartHome skill initialized but no devices configured. Set Home Assistant token, Sonos IPs, or Hue credentials.");
        }

        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| Client::new()),
            config,
            ha_available,
        }
    }

    /// Create with explicit config
    pub fn with_config(config: SmartHomeConfig) -> Self {
        let ha_available = !config.homeassistant_token.is_empty();
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| Client::new()),
            config,
            ha_available,
        }
    }

    /// Check Home Assistant availability
    pub async fn check_ha_status(&self) -> bool {
        if self.config.homeassistant_token.is_empty() {
            return false;
        }

        let url = format!(
            "{}/api/",
            self.config.homeassistant_url.trim_end_matches('/')
        );

        match self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.homeassistant_token))
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    // -------------------------------------------------------------------------
    // Home Assistant Integration
    // -------------------------------------------------------------------------

    /// Get all Home Assistant states
    async fn ha_get_states(&self) -> Result<Vec<Value>, String> {
        let url = format!(
            "{}/api/states",
            self.config.homeassistant_url.trim_end_matches('/')
        );

        let response = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.homeassistant_token))
            .send()
            .await
            .map_err(|e| format!("Home Assistant request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(format!("Home Assistant error: {}", status));
        }

        response.json().await
            .map_err(|e| format!("Failed to parse Home Assistant response: {}", e))
    }

    /// Control a Home Assistant entity
    async fn ha_control_entity(&self, entity_id: &str, command: &str) -> Result<String, String> {
        let (service, data) = match command {
            "on" | "turn_on" => ("homeassistant.turn_on", serde_json::json!({ "entity_id": entity_id })),
            "off" | "turn_off" => ("homeassistant.turn_off", serde_json::json!({ "entity_id": entity_id })),
            "toggle" => ("homeassistant.toggle", serde_json::json!({ "entity_id": entity_id })),
            "open" => ("cover.open_cover", serde_json::json!({ "entity_id": entity_id })),
            "close" => ("cover.close_cover", serde_json::json!({ "entity_id": entity_id })),
            "stop" => ("cover.stop_cover", serde_json::json!({ "entity_id": entity_id })),
            _ => ("homeassistant.turn_on", serde_json::json!({ "entity_id": entity_id })),
        };

        let url = format!(
            "{}/api/services/{}",
            self.config.homeassistant_url.trim_end_matches('/'),
            service
        );

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.homeassistant_token))
            .json(&data)
            .send()
            .await
            .map_err(|e| format!("Home Assistant request failed: {}", e))?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Home Assistant error: {}", body));
        }

        Ok(format!("Home Assistant entity '{}' set to '{}'", entity_id, command))
    }

    /// Get specific entity state
    async fn ha_get_state(&self, entity_id: &str) -> Result<Value, String> {
        let url = format!(
            "{}/api/states/{}",
            self.config.homeassistant_url.trim_end_matches('/'),
            entity_id
        );

        let response = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.homeassistant_token))
            .send()
            .await
            .map_err(|e| format!("Home Assistant request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Entity '{}' not found or inaccessible", entity_id));
        }

        response.json().await
            .map_err(|e| format!("Failed to parse entity state: {}", e))
    }

    /// Call a Home Assistant service with parameters
    async fn ha_call_service(&self, domain: &str, service: &str, entity_id: &str, params: &Value) -> Result<String, String> {
        let mut data = serde_json::json!({ "entity_id": entity_id });
        if let Some(obj) = params.as_object() {
            for (k, v) in obj {
                if k != "entity_id" {
                    data[k] = v.clone();
                }
            }
        }

        let url = format!(
            "{}/api/services/{}/{}",
            self.config.homeassistant_url.trim_end_matches('/'),
            domain,
            service
        );

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.homeassistant_token))
            .json(&data)
            .send()
            .await
            .map_err(|e| format!("Home Assistant service call failed: {}", e))?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Home Assistant error: {}", body));
        }

        Ok(format!("Called {}.{} on {}", domain, service, entity_id))
    }

    // -------------------------------------------------------------------------
    // Sonos Integration
    // -------------------------------------------------------------------------

    /// Control a Sonos speaker via SoCo Python library
    async fn sonos_control(&self, speaker_ip: &str, action: &str, value: Option<&str>) -> Result<String, String> {
        let soco_script = match action {
            "play" => format!(r#"
from soco import SoCo
speaker = SoCo('{}')
speaker.play()
print('playing')
"#, speaker_ip),
            "pause" => format!(r#"
from soco import SoCo
speaker = SoCo('{}')
speaker.pause()
print('paused')
"#, speaker_ip),
            "stop" => format!(r#"
from soco import SoCo
speaker = SoCo('{}')
speaker.stop()
print('stopped')
"#, speaker_ip),
            "next" => format!(r#"
from soco import SoCo
speaker = SoCo('{}')
speaker.next()
print('next_track')
"#, speaker_ip),
            "previous" => format!(r#"
from soco import SoCo
speaker = SoCo('{}')
speaker.previous()
print('previous_track')
"#, speaker_ip),
            "volume" => {
                let vol = value.unwrap_or("50");
                format!(r#"
from soco import SoCo
speaker = SoCo('{}')
speaker.volume = int({})
print('volume_set')
"#, speaker_ip, vol)
            }
            "play_uri" => {
                let uri = value.unwrap_or("");
                format!(r#"
from soco import SoCo
speaker = SoCo('{}')
speaker.play_uri('{}')
print('playing_uri')
"#, speaker_ip, uri)
            }
            _ => return Err(format!("Unknown Sonos action: {}", action)),
        };

        let output = std::process::Command::new("python3")
            .args(["-c", &soco_script])
            .output()
            .map_err(|e| format!("Sonos control failed to execute: {}", e))?;

        if output.status.success() {
            let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(format!("Sonos {} @ {}: {}", action, speaker_ip, result))
        } else {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(format!("Sonos error: {}", err))
        }
    }

    /// List all Sonos speakers
    fn sonos_discover(&self) -> Vec<String> {
        let discover_script = r#"
from soco import.discover
speakers = list(discover())
for s in speakers:
    print(s.ip_address)
"#;

        match std::process::Command::new("python3")
            .args(["-c", discover_script])
            .output()
        {
            Ok(output) => {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            Err(_) => vec![],
        }
    }

    // -------------------------------------------------------------------------
    // Philips Hue Integration
    // -------------------------------------------------------------------------

    /// Control a Hue light
    async fn hue_control_light(&self, light_id: &str, command: &str, params: Option<&Value>) -> Result<String, String> {
        let (bridge_ip, api_key) = self.config.hue_bridge_ip
            .as_ref()
            .zip(self.config.hue_api_key.as_ref())
            .ok_or("Hue bridge not configured. Set ULTRACLAW_HUE_BRIDGE_IP and ULTRACLAW_HUE_API_KEY")?;

        let url = format!(
            "http://{}/api/{}/lights/{}",
            bridge_ip, api_key, light_id
        );

        let state = match command {
            "on" => serde_json::json!({ "on": true }),
            "off" => serde_json::json!({ "on": false }),
            "toggle" => serde_json::json!({ "on": true }), // Will read current state first
            "brightness" => {
                let brightness = params
                    .and_then(|p| p.get("brightness").and_then(|v| v.as_i64()))
                    .unwrap_or(254) as u8;
                serde_json::json!({ "on": true, "bri": brightness })
            }
            "color" => {
                let hue = params.and_then(|p| p.get("hue").and_then(|v| v.as_i64())).unwrap_or(0) as u16;
                let sat = params.and_then(|p| p.get("saturation").and_then(|v| v.as_i64())).unwrap_or(200) as u8;
                serde_json::json!({ "on": true, "hue": hue, "sat": sat })
            }
            "alert" => serde_json::json!({ "alert": "select" }),
            _ => serde_json::json!({ "on": true }),
        };

        let response = self.client
            .put(&url)
            .json(&serde_json::json!({ "state": state }))
            .send()
            .await
            .map_err(|e| format!("Hue request failed: {}", e))?;

        if response.status().is_success() {
            Ok(format!("Hue light {} set to '{}'", light_id, command))
        } else {
            Err(format!("Hue error: {}", response.text().await.unwrap_or_default()))
        }
    }

    /// Get Hue light state
    async fn hue_get_light(&self, light_id: &str) -> Result<Value, String> {
        let (bridge_ip, api_key) = self.config.hue_bridge_ip
            .as_ref()
            .zip(self.config.hue_api_key.as_ref())
            .ok_or("Hue bridge not configured")?;

        let url = format!(
            "http://{}/api/{}/lights/{}",
            bridge_ip, api_key, light_id
        );

        let response = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Hue request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Hue light {} not found", light_id));
        }

        response.json().await
            .map_err(|e| format!("Failed to parse Hue response: {}", e))
    }

    /// List all Hue lights
    async fn hue_list_lights(&self) -> Result<Value, String> {
        let (bridge_ip, api_key) = self.config.hue_bridge_ip
            .as_ref()
            .zip(self.config.hue_api_key.as_ref())
            .ok_or("Hue bridge not configured")?;

        let url = format!(
            "http://{}/api/{}/lights",
            bridge_ip, api_key
        );

        let response = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Hue request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Hue error: {}", response.text().await.unwrap_or_default()));
        }

        response.json().await
            .map_err(|e| format!("Failed to parse Hue response: {}", e))
    }
}

impl Default for SmartHomeSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for SmartHomeSkill {
    fn name(&self) -> &'static str {
        "smarthome"
    }

    fn description(&self) -> &'static str {
        "Control smart home devices via Home Assistant, Philips Hue, or Sonos. \
         Supports: lights (on/off/dim/color), media players (play/pause/volume), \
         switches, covers, climate, and more. Requires Home Assistant token or \
         direct device configurations."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The smart home action to perform",
                    "enum": ["control", "status", "list_devices", "discover"]
                },
                "platform": {
                    "type": "string",
                    "description": "Smart home platform to use",
                    "enum": ["homeassistant", "hue", "sonos"],
                    "default": "homeassistant"
                },
                "device": {
                    "type": "string",
                    "description": "Device identifier (entity_id for HA, light number for Hue, IP for Sonos)"
                },
                "command": {
                    "type": "string",
                    "description": "Command to execute",
                    "enum": ["on", "off", "toggle", "brightness", "color", "volume", "play", "pause", "stop", "next", "previous", "open", "close"]
                },
                "value": {
                    "type": "string",
                    "description": "Value for the command (e.g., volume level 0-100, color hex, URI)"
                },
                "params": {
                    "type": "object",
                    "description": "Additional parameters for the command (e.g., brightness, hue, saturation)"
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("control").to_string();
        let platform = args.get("platform").and_then(|v| v.as_str()).unwrap_or("homeassistant").to_string();
        let device = args.get("device").and_then(|v| v.as_str()).map(|s| s.to_string());
        let command = args.get("command").and_then(|v| v.as_str()).map(|s| s.to_string());
        let value = args.get("value").and_then(|v| v.as_str()).map(|s| s.to_string());
        let args_json = args.to_string();

        // Use blocking runtime for async operations
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "smarthome".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let config = self.config.clone();
        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let args_owned: Value = serde_json::from_str(&args_json).map_err(|e| e.to_string())?;
                let action = args_owned.get("action").and_then(|v| v.as_str()).unwrap_or("control");
                let platform = args_owned.get("platform").and_then(|v| v.as_str()).unwrap_or("homeassistant");
                let device = args_owned.get("device").and_then(|v| v.as_str());
                let command = args_owned.get("command").and_then(|v| v.as_str());
                let value = args_owned.get("value").and_then(|v| v.as_str());
                let params = args_owned.get("params");
                execute_smart_home_action(&config, action, platform, device, command, value, params).await
            })
        })
        .join()
        .unwrap_or_else(|_| Err("SmartHome operation panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "smarthome".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "smarthome".to_string(),
                output: format!("SmartHome error: {}", e),
                is_error: true,
            },
        }
    }
}

/// Execute smart home action based on arguments
async fn execute_smart_home_action(
    config: &SmartHomeConfig,
    action: &str,
    platform: &str,
    device: Option<&str>,
    command: Option<&str>,
    value: Option<&str>,
    params: Option<&Value>,
) -> Result<String, String> {
    match action {
        "control" => {
            let device = device.ok_or("Device identifier is required for control action")?;
            let command = command.ok_or("Command is required for control action")?;

            match platform {
                "homeassistant" => {
                    if config.homeassistant_token.is_empty() {
                        return Err("Home Assistant token not configured. Set ULTRACLAW_HOMEASSISTANT_TOKEN".to_string());
                    }

                    let skill = SmartHomeSkill::with_config(config.clone());
                    skill.ha_control_entity(device, command).await
                }

                "hue" => {
                    if config.hue_bridge_ip.is_none() || config.hue_api_key.is_none() {
                        return Err("Hue not configured. Set ULTRACLAW_HUE_BRIDGE_IP and ULTRACLAW_HUE_API_KEY".to_string());
                    }

                    let skill = SmartHomeSkill::with_config(config.clone());
                    skill.hue_control_light(device, command, params).await
                }

                "sonos" => {
                    if config.sonos_ips.is_empty() {
                        return Err("Sonos IPs not configured. Set ULTRACLAW_SONOS_IPS".to_string());
                    }

                    let target_ip = if config.sonos_ips.contains(&device.to_string()) {
                        device.to_string()
                    } else {
                        config.sonos_ips.first().cloned().unwrap_or_default()
                    };

                    let skill = SmartHomeSkill::with_config(config.clone());
                    skill.sonos_control(&target_ip, command, value).await
                }

                _ => Err(format!("Unknown platform: {}. Use homeassistant, hue, or sonos.", platform)),
            }
        }

        "status" => {
            let device = device.ok_or("Device identifier is required for status action")?;

            match platform {
                "homeassistant" => {
                    if config.homeassistant_token.is_empty() {
                        return Err("Home Assistant token not configured".to_string());
                    }

                    let skill = SmartHomeSkill::with_config(config.clone());
                    let state = skill.ha_get_state(device).await?;

                    let state_val = state.get("state").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let friendly_name = state.get("attributes")
                        .and_then(|a| a.get("friendly_name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(device);

                    Ok(format!("{} ({}) is {}", friendly_name, device, state_val))
                }

                "hue" => {
                    if config.hue_bridge_ip.is_none() {
                        return Err("Hue not configured".to_string());
                    }

                    let skill = SmartHomeSkill::with_config(config.clone());
                    let light = skill.hue_get_light(device).await?;

                    let on = light.get("state")
                        .and_then(|s| s.get("on"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let brightness = light.get("state")
                        .and_then(|s| s.get("bri"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let name = light.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(device);

                    let brightness_pct = (brightness as f64 / 254.0 * 100.0).round() as i64;
                    Ok(format!("{} is {} (brightness {}%)", name, if on { "on" } else { "off" }, brightness_pct))
                }

                _ => Err(format!("Status action not supported for platform: {}", platform)),
            }
        }

        "list_devices" => {
            match platform {
                "homeassistant" => {
                    if config.homeassistant_token.is_empty() {
                        return Err("Home Assistant token not configured".to_string());
                    }

                    let skill = SmartHomeSkill::with_config(config.clone());
                    let states = skill.ha_get_states().await?;

                    let mut devices: Vec<String> = Vec::new();
                    for state in states.iter().take(50) {
                        if let (Some(entity_id), Some(state_val)) = (
                            state.get("entity_id").and_then(|v| v.as_str()),
                            state.get("state").and_then(|v| v.as_str())
                        ) {
                            let friendly = state.get("attributes")
                                .and_then(|a| a.get("friendly_name"))
                                .and_then(|v| v.as_str())
                                .unwrap_or(entity_id);
                            devices.push(format!("{}: {}", entity_id, state_val));
                        }
                    }

                    if devices.is_empty() {
                        Ok("No devices found in Home Assistant".to_string())
                    } else {
                        Ok(format!("Home Assistant Devices ({}):\n{}", devices.len(), devices.join("\n")))
                    }
                }

                "hue" => {
                    if config.hue_bridge_ip.is_none() {
                        return Err("Hue not configured".to_string());
                    }

                    let skill = SmartHomeSkill::with_config(config.clone());
                    let lights = skill.hue_list_lights().await?;

                    let mut result = String::from("Hue Lights:\n");
                    if let Some(obj) = lights.as_object() {
                        for (id, light) in obj.iter().take(20) {
                            let name = light.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown");
                            let on = light.get("state").and_then(|s| s.get("on")).and_then(|v| v.as_bool()).unwrap_or(false);
                            result.push_str(&format!("  {}: {} ({})\n", id, name, if on { "on" } else { "off" }));
                        }
                    }
                    Ok(result)
                }

                "sonos" => {
                    if config.sonos_ips.is_empty() {
                        return Err("Sonos IPs not configured".to_string());
                    }

                    let mut result = String::from("Sonos Speakers:\n");
                    for ip in &config.sonos_ips {
                        result.push_str(&format!("  {}\n", ip));
                    }
                    Ok(result)
                }

                _ => Err(format!("Unknown platform: {}", platform)),
            }
        }

        "discover" => {
            match platform {
                "sonos" => {
                    let skill = SmartHomeSkill::with_config(config.clone());
                    let speakers = skill.sonos_discover();
                    if speakers.is_empty() {
                        Ok("No Sonos speakers discovered on the network".to_string())
                    } else {
                        Ok(format!("Discovered Sonos speakers: {}", speakers.join(", ")))
                    }
                }

                _ => Err(format!("Discover not supported for platform: {}", platform)),
            }
        }

        _ => Err(format!(
            "Unknown action: '{}'. Available: control, status, list_devices, discover",
            action
        )),
    }
}