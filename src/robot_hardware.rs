use tokio::sync::RwLock;
use tracing::{info, warn};

pub trait RobotHardware: Send + Sync {
    fn drive(&self, speed: f32, direction: f32) -> Result<(), String>;
    fn speak(&self, text: &str) -> Result<(), String>;
    fn look(&self) -> Result<Vec<u8>, String>;
    fn emote(&self, expression: &str) -> Result<(), String>;
    fn sense(&self) -> Result<Vec<f32>, String>;
}

pub struct SimulatedHardware;

impl RobotHardware for SimulatedHardware {
    fn drive(&self, speed: f32, direction: f32) -> Result<(), String> {
        info!("[SIMULATED] Driving: speed={}, direction={}", speed, direction);
        Ok(())
    }

    fn speak(&self, text: &str) -> Result<(), String> {
        info!("[SIMULATED] Speaking: {}", text);
        Ok(())
    }

    fn look(&self) -> Result<Vec<u8>, String> {
        info!("[SIMULATED] Capturing camera frame");
        Ok(vec![0x80])
    }

    fn emote(&self, expression: &str) -> Result<(), String> {
        info!("[SIMULATED] Displaying expression: {}", expression);
        Ok(())
    }

    fn sense(&self) -> Result<Vec<f32>, String> {
        info!("[SIMULATED] LIDAR scan");
        Ok(vec![1.0, 1.5, 2.0, 1.2, 0.8])
    }
}

pub struct SerialHardware {
    port: String,
    connection: RwLock<Option<Box<dyn SerialPort>>>,
}

pub trait SerialPort: Send + Sync {
    fn write_bytes(&mut self, data: &[u8]) -> Result<(), String>;
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, String>;
}

impl SerialHardware {
    pub fn new(port: &str) -> Self {
        Self {
            port: port.to_string(),
            connection: RwLock::new(None),
        }
    }

    pub async fn connect(&self) -> Result<(), String> {
        warn!("Serial hardware not fully implemented - using simulated mode");
        Ok(())
    }
}

impl RobotHardware for SerialHardware {
    fn drive(&self, speed: f32, direction: f32) -> Result<(), String> {
        info!("[SERIAL] Would send: DRIVE:{},{}", speed, direction);
        Ok(())
    }

    fn speak(&self, text: &str) -> Result<(), String> {
        info!("[SERIAL] Would send: SPEAK:{}", text);
        Ok(())
    }

    fn look(&self) -> Result<Vec<u8>, String> {
        Ok(vec![])
    }

    fn emote(&self, expression: &str) -> Result<(), String> {
        info!("[SERIAL] Would send: EMOTE:{}", expression);
        Ok(())
    }

    fn sense(&self) -> Result<Vec<f32>, String> {
        Ok(vec![])
    }
}

pub struct HttpRobotHardware {
    base_url: String,
}

impl HttpRobotHardware {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
        }
    }
}

impl RobotHardware for HttpRobotHardware {
    fn drive(&self, speed: f32, direction: f32) -> Result<(), String> {
        let client = reqwest::blocking::Client::new();
        let url = format!("{}/drive?speed={}&direction={}", self.base_url, speed, direction);
        client.get(&url).send().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn speak(&self, text: &str) -> Result<(), String> {
        let client = reqwest::blocking::Client::new();
        let encoded_text = urlencoding::encode(text);
        let url = format!("{}/speak?text={}", self.base_url, encoded_text);
        client.get(&url).send().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn look(&self) -> Result<Vec<u8>, String> {
        let client = reqwest::blocking::Client::new();
        let response = client.get(&format!("{}/camera", self.base_url))
            .send()
            .map_err(|e| e.to_string())?;
        Ok(response.bytes().map_err(|e| e.to_string())?.to_vec())
    }

    fn emote(&self, expression: &str) -> Result<(), String> {
        let client = reqwest::blocking::Client::new();
        let encoded_expr = urlencoding::encode(expression);
        let url = format!("{}/emote?expression={}", self.base_url, encoded_expr);
        client.get(&url).send().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn sense(&self) -> Result<Vec<f32>, String> {
        let client = reqwest::blocking::Client::new();
        let response = client.get(&format!("{}/lidar", self.base_url))
            .send()
            .map_err(|e| e.to_string())?;
        let text = response.text().map_err(|e| e.to_string())?;
        let distances: Vec<f32> = serde_json::from_str(&text).unwrap_or_default();
        Ok(distances)
    }
}