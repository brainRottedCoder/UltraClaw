// ============================================================================
// ULTRACLAW — eval.rs
// ============================================================================
// Cost and performance metrics tracking for the GARAGE_INFERENCE hackathon.
//
// This module provides:
// - Real-time cost tracking (local vs cloud)
// - Latency and throughput metrics
// - Token counting and estimation
// - Hackathon submission metrics generation
//
// MEMORY OPTIMIZATION:
// - Metrics use atomic counters, not mutex-protected state.
// - Aggregations are done in place without copying data.
//
// ENERGY OPTIMIZATION:
// - Metrics are stored in memory only (no disk I/O).
// - Periodic summary generation instead of continuous logging.
// ============================================================================

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Cloud model pricing (per 1M tokens).
/// Used for cost comparison against local models.
#[derive(Debug, Clone, Copy)]
pub struct CloudPricing {
    pub input_cost_per_million: f64,
    pub output_cost_per_million: f64,
}

/// Standard cloud model pricing estimates.
impl CloudPricing {
    /// GPT-4o-mini pricing (fallback for demo).
    pub fn gpt_4o_mini() -> Self {
        Self {
            input_cost_per_million: 0.15,
            output_cost_per_million: 0.60,
        }
    }

    /// Gemini 2.0 Flash pricing.
    pub fn gemini_flash() -> Self {
        Self {
            input_cost_per_million: 0.10,
            output_cost_per_million: 0.40,
        }
    }

    /// Calculate cost for given token counts.
    pub fn calculate_cost(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        let input_cost = (input_tokens as f64 / 1_000_000.0) * self.input_cost_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * self.output_cost_per_million;
        input_cost + output_cost
    }
}

/// Session metrics for tracking cost and performance.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMetrics {
    pub total_inferences: u64,
    pub local_inferences: u64,
    pub cloud_inferences: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_latency_ms: u64,
    pub failed_inferences: u64,
    pub tool_calls_executed: u64,
    pub react_loops_completed: u64,
}

impl SessionMetrics {
    /// Create a new empty metrics tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate the estimated cloud cost for these tokens.
    pub fn estimated_cloud_cost(&self, pricing: CloudPricing) -> f64 {
        pricing.calculate_cost(self.total_input_tokens, self.total_output_tokens)
    }

    /// Calculate cost saved by using local model.
    pub fn local_savings(&self, pricing: CloudPricing) -> f64 {
        self.estimated_cloud_cost(pricing)
    }

    /// Calculate average latency per inference.
    pub fn avg_latency_ms(&self) -> f64 {
        if self.total_inferences == 0 {
            return 0.0;
        }
        self.total_latency_ms as f64 / self.total_inferences as f64
    }

    /// Calculate tokens per second throughput.
    pub fn tokens_per_second(&self) -> f64 {
        if self.total_latency_ms == 0 {
            return 0.0;
        }
        let total_tokens = self.total_input_tokens + self.total_output_tokens;
        let latency_secs = self.total_latency_ms as f64 / 1000.0;
        if latency_secs == 0.0 {
            return 0.0;
        }
        total_tokens as f64 / latency_secs
    }

    /// Generate a human-readable summary.
    pub fn summary(&self, pricing: CloudPricing) -> String {
        let local_cost = 0.0_f64; // Local Ollama = $0
        let cloud_cost = self.estimated_cloud_cost(pricing);

        format!(
            r#"## Session Metrics

### Cost Analysis
- **Local Inferences:** {} (${:.4})
- **Cloud Inferences:** {} (${:.4})
- **Total Inferences:** {}
- **Local Cost:** $0.00
- **Cloud Equivalent Cost:** ${:.4}
- **Savings:** ${:.4} ({}%)

### Performance
- **Total Input Tokens:** {}
- **Total Output Tokens:** {}
- **Total Tokens:** {}
- **Avg Latency:** {:.1}ms/inference
- **Throughput:** {:.1} tokens/sec

### Reliability
- **Failed Inferences:** {}
- **Tool Calls Executed:** {}
- **ReAct Loops Completed:** {}
"#,
            self.local_inferences,
            local_cost,
            self.cloud_inferences,
            cloud_cost,
            self.total_inferences,
            cloud_cost,
            local_cost,
            if cloud_cost > 0.0 {
                (local_cost / cloud_cost * 100.0) as i32
            } else {
                100
            },
            self.total_input_tokens,
            self.total_output_tokens,
            self.total_input_tokens + self.total_output_tokens,
            self.avg_latency_ms(),
            self.tokens_per_second(),
            self.failed_inferences,
            self.tool_calls_executed,
            self.react_loops_completed
        )
    }
}

/// Thread-safe metrics aggregator for use during inference.
pub struct MetricsAggregator {
    local_inferences: AtomicU64,
    cloud_inferences: AtomicU64,
    total_input_tokens: AtomicU64,
    total_output_tokens: AtomicU64,
    total_latency_ms: AtomicU64,
    failed_inferences: AtomicU64,
    tool_calls_executed: AtomicU64,
    react_loops_completed: AtomicU64,
}

impl Default for MetricsAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsAggregator {
    pub fn new() -> Self {
        Self {
            local_inferences: AtomicU64::new(0),
            cloud_inferences: AtomicU64::new(0),
            total_input_tokens: AtomicU64::new(0),
            total_output_tokens: AtomicU64::new(0),
            total_latency_ms: AtomicU64::new(0),
            failed_inferences: AtomicU64::new(0),
            tool_calls_executed: AtomicU64::new(0),
            react_loops_completed: AtomicU64::new(0),
        }
    }

    pub fn record_local_inference(&self, input_tokens: u64, output_tokens: u64, latency_ms: u64) {
        self.local_inferences.fetch_add(1, Ordering::Relaxed);
        self.total_input_tokens.fetch_add(input_tokens, Ordering::Relaxed);
        self.total_output_tokens.fetch_add(output_tokens, Ordering::Relaxed);
        self.total_latency_ms.fetch_add(latency_ms, Ordering::Relaxed);
    }

    pub fn record_cloud_inference(&self, input_tokens: u64, output_tokens: u64, latency_ms: u64) {
        self.cloud_inferences.fetch_add(1, Ordering::Relaxed);
        self.total_input_tokens.fetch_add(input_tokens, Ordering::Relaxed);
        self.total_output_tokens.fetch_add(output_tokens, Ordering::Relaxed);
        self.total_latency_ms.fetch_add(latency_ms, Ordering::Relaxed);
    }

    pub fn record_failed_inference(&self) {
        self.failed_inferences.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_tool_call(&self) {
        self.tool_calls_executed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_react_loop(&self) {
        self.react_loops_completed.fetch_add(1, Ordering::Relaxed);
    }

    /// Collect all metrics into a SessionMetrics struct.
    pub fn collect(&self) -> SessionMetrics {
        SessionMetrics {
            total_inferences: self.local_inferences.load(Ordering::Relaxed)
                + self.cloud_inferences.load(Ordering::Relaxed),
            local_inferences: self.local_inferences.load(Ordering::Relaxed),
            cloud_inferences: self.cloud_inferences.load(Ordering::Relaxed),
            total_input_tokens: self.total_input_tokens.load(Ordering::Relaxed),
            total_output_tokens: self.total_output_tokens.load(Ordering::Relaxed),
            total_latency_ms: self.total_latency_ms.load(Ordering::Relaxed),
            failed_inferences: self.failed_inferences.load(Ordering::Relaxed),
            tool_calls_executed: self.tool_calls_executed.load(Ordering::Relaxed),
            react_loops_completed: self.react_loops_completed.load(Ordering::Relaxed),
        }
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.local_inferences.store(0, Ordering::Relaxed);
        self.cloud_inferences.store(0, Ordering::Relaxed);
        self.total_input_tokens.store(0, Ordering::Relaxed);
        self.total_output_tokens.store(0, Ordering::Relaxed);
        self.total_latency_ms.store(0, Ordering::Relaxed);
        self.failed_inferences.store(0, Ordering::Relaxed);
        self.tool_calls_executed.store(0, Ordering::Relaxed);
        self.react_loops_completed.store(0, Ordering::Relaxed);
    }
}

/// Timing helper for measuring inference latency.
pub struct InferenceTimer {
    start: Instant,
}

impl InferenceTimer {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Hackathon submission metrics for GARAGE_INFERENCE.
/// This format matches what judges expect to see.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HackathonMetrics {
    pub model_name: String,
    pub model_tier: String,
    pub run_location: String,
    pub hardware_requirement: String,
    pub session: SessionMetrics,
}

impl HackathonMetrics {
    /// Create metrics for Phi-4-mini.
    pub fn for_phi4_mini(session: SessionMetrics) -> Self {
        Self {
            model_name: "phi4-mini:latest".to_string(),
            model_tier: "Tier 1 - Absolute Garage MAX".to_string(),
            run_location: "Local (Ollama)".to_string(),
            hardware_requirement: "CPU-only, no GPU required (~4GB RAM)".to_string(),
            session,
        }
    }

    /// Generate the complete hackathon metrics report.
    pub fn generate_report(&self) -> String {
        let pricing = CloudPricing::gpt_4o_mini();
        let local_cost = 0.0_f64;
        let cloud_cost = self.session.estimated_cloud_cost(pricing);

        format!(
            r#"# GARAGE_INFERENCE Hackathon Submission Metrics

## Model Declaration
- **Model:** {}
- **Tier:** {} ({})
- **Run Location:** {}
- **Hardware Required:** {}

## Cost Metrics
- **Local Cost:** ${:.4} (Ollama = free after model download)
- **Cloud Equivalent:** ${:.4} (GPT-4o-mini pricing)
- **Savings:** ${:.4} ({}%)

## Performance Metrics
- **Total Inferences:** {} (Local: {}, Cloud: {})
- **Total Tokens:** {} (Input: {}, Output: {})
- **Avg Latency:** {:.1}ms
- **Throughput:** {:.1} tokens/sec

## Tool Usage
- **Tool Calls Executed:** {}
- **ReAct Loops Completed:** {}

## Reliability
- **Failed Inferences:** {}
- **Success Rate:** {}%

---
Generated by Ultraclaw - GARAGE_INFERENCE Hackathon Submission
"#,
            self.model_name,
            self.model_tier.split('-').next().unwrap_or(&self.model_tier),
            self.model_tier,
            self.run_location,
            self.hardware_requirement,
            local_cost,
            cloud_cost,
            cloud_cost - local_cost,
            if cloud_cost > 0.0 {
                ((cloud_cost - local_cost) / cloud_cost * 100.0) as i32
            } else {
                100
            },
            self.session.total_inferences,
            self.session.local_inferences,
            self.session.cloud_inferences,
            self.session.total_input_tokens + self.session.total_output_tokens,
            self.session.total_input_tokens,
            self.session.total_output_tokens,
            self.session.avg_latency_ms(),
            self.session.tokens_per_second(),
            self.session.tool_calls_executed,
            self.session.react_loops_completed,
            self.session.failed_inferences,
            if self.session.total_inferences > 0 {
                (self.session.total_inferences - self.session.failed_inferences)
                    * 100
                    / self.session.total_inferences
            } else {
                0
            }
        )
    }
}

/// Benchmark result for comparing local vs cloud models.
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub task_name: String,
    pub local_latency_ms: u64,
    pub local_tokens: u64,
    pub local_cost: f64,
    pub cloud_latency_ms: u64,
    pub cloud_tokens: u64,
    pub cloud_cost: f64,
    pub quality_match_percent: u8,
    pub winner: String,
}

impl BenchmarkResult {
    pub fn new(task_name: &str) -> Self {
        Self {
            task_name: task_name.to_string(),
            local_latency_ms: 0,
            local_tokens: 0,
            local_cost: 0.0,
            cloud_latency_ms: 0,
            cloud_tokens: 0,
            cloud_cost: 0.0,
            quality_match_percent: 0,
            winner: "N/A".to_string(),
        }
    }

    pub fn summary(&self) -> String {
        format!(
            r#"### {} ({})

| Metric | Local (Gemma 4 E4B) | Cloud (GPT-4o-mini) |
|--------|---------------------|----------------------|
| Latency | {}ms | {}ms |
| Tokens | {} | {} |
| Cost | ${:.4} | ${:.4} |
| **Winner** | **{}** |
| Quality Match | {}% |

"#,
            self.task_name,
            if self.winner == "local" { "Local wins!" } else { "Cloud wins!" },
            self.local_latency_ms,
            self.cloud_latency_ms,
            self.local_tokens,
            self.cloud_tokens,
            self.local_cost,
            self.cloud_cost,
            self.winner,
            self.quality_match_percent
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_metrics() {
        let mut metrics = SessionMetrics::new();
        metrics.total_inferences = 7;
        metrics.local_inferences = 5;
        metrics.cloud_inferences = 2;
        metrics.total_input_tokens = 1000;
        metrics.total_output_tokens = 500;
        metrics.total_latency_ms = 5000;

        let pricing = CloudPricing::gpt_4o_mini();
        assert_eq!(metrics.estimated_cloud_cost(pricing), 0.00045); // Very small cost
        let avg = metrics.avg_latency_ms();
        assert!((avg - 714.2857142857143).abs() < 0.001); // 5000 / 7
    }

    #[test]
    fn test_metrics_aggregator() {
        let agg = MetricsAggregator::new();
        agg.record_local_inference(100, 50, 200);
        agg.record_local_inference(150, 75, 250);

        let collected = agg.collect();
        assert_eq!(collected.local_inferences, 2);
        assert_eq!(collected.total_input_tokens, 250);
    }
}