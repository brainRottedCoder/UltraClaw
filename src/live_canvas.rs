pub struct LiveCanvasProtocol {}

impl Default for LiveCanvasProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveCanvasProtocol {
    pub fn new() -> Self {
        Self {}
    }

    pub fn render_ui(&self, component_name: &str) -> String {
        format!("Rendering Agent-Driven Canvas Component: {}", component_name)
    }
}
