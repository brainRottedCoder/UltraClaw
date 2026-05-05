pub struct AndroidAppClient {
    application_id: String,
}

impl Default for AndroidAppClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AndroidAppClient {
    pub fn new() -> Self {
        Self { application_id: "com.ultraclaw.android".into() }
    }
}