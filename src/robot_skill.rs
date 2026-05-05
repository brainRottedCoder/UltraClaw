use crate::skill::Skill;
use crate::robot_hardware::{RobotHardware, SimulatedHardware};
use std::sync::Arc;

pub struct RobotKit {
    hardware: Arc<dyn RobotHardware>,
    is_active: bool,
}

impl RobotKit {
    pub fn new() -> Self {
        Self {
            hardware: Arc::new(SimulatedHardware),
            is_active: false,
        }
    }

    pub fn with_hardware(hardware: Arc<dyn RobotHardware>) -> Self {
        Self {
            hardware,
            is_active: true,
        }
    }

    pub fn drive(&self, speed: f32, direction: f32) -> Result<String, String> {
        self.hardware.drive(speed, direction)?;
        Ok(format!("Robot driving at speed {} in direction {}", speed, direction))
    }

    pub fn speak(&self, text: &str) -> Result<String, String> {
        self.hardware.speak(text)?;
        Ok(format!("Robot speaking: {}", text))
    }

    pub fn look(&self) -> Result<String, String> {
        let frame = self.hardware.look()?;
        Ok(format!(
            "Camera frame captured: {} bytes",
            frame.len()
        ))
    }

    pub fn emote(&self, expression: &str) -> Result<String, String> {
        self.hardware.emote(expression)?;
        Ok(format!("Robot displaying expression: {}", expression))
    }

    pub fn sense(&self) -> Result<String, String> {
        let distances = self.hardware.sense()?;
        let summary: String = distances.iter()
            .map(|d| format!("{}m", (d * 10.0).round() / 10.0))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(format!("LIDAR scan: [{}]", summary))
    }
}

impl Default for RobotKit {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for RobotKit {
    fn name(&self) -> &'static str {
        "RobotKit"
    }

    fn description(&self) -> &'static str {
        "Direct hardware control for robots. Controls motors (drive), speaker (speak), \
         camera (look), LED matrix (emote), and LIDAR (sense). Works with Arduino, \
         Raspberry Pi, or WiFi-connected robots. Speed: -1.0 to 1.0, Direction: degrees."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Robot action to perform",
                    "enum": ["drive", "speak", "look", "emote", "sense", "status"]
                },
                "speed": {
                    "type": "number",
                    "description": "Motor speed (-1.0 to 1.0, negative = reverse)"
                },
                "direction": {
                    "type": "number",
                    "description": "Steering direction in degrees (0 = forward, 90 = right)"
                },
                "text": {
                    "type": "string",
                    "description": "Text for robot to speak (for speak action)"
                },
                "expression": {
                    "type": "string",
                    "description": "Expression to display on LED matrix",
                    "enum": ["happy", "sad", "angry", "surprised", "neutral", "sleepy"]
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, params: &serde_json::Value) -> crate::skill::SkillOutput {
        let action = params.get("action").and_then(|v| v.as_str()).unwrap_or("status");
        
        let result = match action {
            "drive" => {
                let speed = params.get("speed").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
                let direction = params.get("direction").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                self.drive(speed, direction)
            }
            "speak" => {
                let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("Hello!");
                self.speak(text)
            }
            "look" => self.look(),
            "emote" => {
                let expression = params.get("expression").and_then(|v| v.as_str()).unwrap_or("happy");
                self.emote(expression)
            }
            "sense" => self.sense(),
            "status" => Ok(format!(
                "Robot status: active={}, battery={}%", 
                self.is_active, 
                85
            )),
            _ => Err(format!("Unknown action: {}", action))
        };

        match result {
            Ok(output) => crate::skill::SkillOutput {
                name: self.name().into(),
                output,
                is_error: false
            },
            Err(e) => crate::skill::SkillOutput {
                name: self.name().into(),
                output: format!("Robot error: {}", e),
                is_error: true
            }
        }
    }
}