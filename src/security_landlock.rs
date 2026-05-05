use std::path::PathBuf;

pub struct LandlockSecurity {
    enabled: bool,
    allowed_paths: Vec<PathBuf>,
}

impl Default for LandlockSecurity {
    fn default() -> Self {
        Self::new()
    }
}

impl LandlockSecurity {
    pub fn new() -> Self {
        Self {
            enabled: false,
            allowed_paths: Vec::new(),
        }
    }

    pub fn allow_read(&mut self, path: PathBuf) {
        self.allowed_paths.push(path);
    }

    pub fn enforce(&mut self) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            // In a real implementation, we would bind to the linux landlock ABI here.
            // Restricting access to only allowed_paths.
            self.enabled = true;
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err("Landlock security is only supported on Linux".to_string())
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}
