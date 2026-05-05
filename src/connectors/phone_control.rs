pub struct PhoneController {}

impl Default for PhoneController {
    fn default() -> Self {
        Self::new()
    }
}

impl PhoneController {
    pub fn new() -> Self {
        Self {}
    }

    pub fn dial(&self, number: &str) -> String {
        format!("Dialing: {}", number)
    }
}