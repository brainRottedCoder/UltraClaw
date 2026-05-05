pub struct QuotaTracker {
    max_tokens: u64,
    tokens_used: u64,
}

impl Default for QuotaTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl QuotaTracker {
    pub fn new() -> Self {
        Self {
            max_tokens: 1_000_000,
            tokens_used: 0,
        }
    }

    pub fn consume(&mut self, amount: u64) -> Result<(), &'static str> {
        if self.tokens_used + amount > self.max_tokens {
            Err("Monthly quota exceeded! Upgrade to unlock unlimited usage.")
        } else {
            self.tokens_used += amount;
            Ok(())
        }
    }
}
