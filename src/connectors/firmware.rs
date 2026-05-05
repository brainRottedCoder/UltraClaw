pub struct EmbeddedConnector {
    baud_rate: u32,
}

impl Default for EmbeddedConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddedConnector {
    pub fn new() -> Self {
        Self { baud_rate: 115200 }
    }

    pub fn poll_sensors(&self) -> Vec<f32> {
        vec![0.0, 0.0, 0.0]
    }
}