pub struct SwabbleMacClient {}

impl Default for SwabbleMacClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SwabbleMacClient {
    pub fn new() -> Self {
        Self {}
    }

    pub fn bridge(&self) -> String {
        "Swabble Mac bridge active".into()
    }
}