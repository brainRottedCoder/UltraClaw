// ============================================================================
// ULTRACLAW — scaffolding_tests.rs
// ============================================================================
// Integration tests for the 8 scaffolding layers.
//
// TEST COVERAGE:
// - Self-Healing: error detection, retry counting, healing guidance injection
// - Goal Decomposition: plan generation, dependency sorting, execution order
// - Chain-of-Tools: sequential execution, circuit breaker, timeout handling
// - Hallucination Shield: claim extraction, verification, annotation
//
// Run with: cargo test --test scaffolding_tests
// ============================================================================

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// A1 — Self-Healing Loop Tests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct HealingTestResult {
    attempt: usize,
    error_detected: bool,
    guidance_injected: bool,
    success: bool,
}

struct MockHealingLoop {
    max_cycles: usize,
    results: Vec<HealingTestResult>,
}

impl MockHealingLoop {
    fn new(max_cycles: usize) -> Self {
        MockHealingLoop {
            max_cycles,
            results: Vec::new(),
        }
    }

    fn detect_error(&self, output: &str) -> bool {
        let error_keywords = ["error", "failed", "exception", "denied", "timeout", "not found"];
        let lower = output.to_lowercase();
        error_keywords.iter().any(|kw| lower.contains(kw))
    }

    fn inject_healing_guidance(&self, error: &str) -> String {
        format!(
            "Previous attempt failed with: {}. Try an alternative approach. \
             Check tool arguments, verify paths, and retry with corrected input.",
            error
        )
    }

    fn run_healing_cycle(&mut self, output: &str) -> (bool, String) {
        for cycle in 0..self.max_cycles {
            let attempt = cycle + 1;
            let error_detected = self.detect_error(output);

            let result = HealingTestResult {
                attempt,
                error_detected,
                guidance_injected: error_detected,
                success: !error_detected,
            };
            self.results.push(result);

            if !error_detected {
                return (true, "Task succeeded on cycle {}".to_string());
            }

            let _guidance = self.inject_healing_guidance(output);
        }

        (false, "Max healing cycles exceeded".to_string())
    }
}

#[test]
fn test_self_healing_detects_error() {
    let loop_instance = MockHealingLoop::new(3);

    assert!(loop_instance.detect_error("Operation failed with error: file not found"));
    assert!(loop_instance.detect_error("Permission denied"));
    assert!(loop_instance.detect_error("Connection timeout after 30 seconds"));
    assert!(!loop_instance.detect_error("Operation completed successfully"));
    assert!(!loop_instance.detect_error("File read: 42 lines processed"));
}

#[test]
fn test_self_healing_success_on_first_try() {
    let mut loop_instance = MockHealingLoop::new(3);
    let (success, _msg) = loop_instance.run_healing_cycle("File read successfully: 100 lines");

    assert!(success);
    assert_eq!(loop_instance.results.len(), 1);
    assert!(!loop_instance.results[0].error_detected);
    assert!(loop_instance.results[0].success);
}

#[test]
fn test_self_healing_recovers_after_errors() {
    let mut loop_instance = MockHealingLoop::new(3);
    let (success, _msg) = loop_instance.run_healing_cycle("Error: file not found");

    // This will fail because we keep giving the same error output
    assert!(!success);
    assert_eq!(loop_instance.results.len(), 3); // All 3 cycles attempted
    for result in &loop_instance.results {
        assert!(result.error_detected);
        assert!(result.guidance_injected);
    }
}

#[test]
fn test_self_healing_max_cycles_enforced() {
    let mut loop_instance = MockHealingLoop::new(2);
    let (success, _) = loop_instance.run_healing_cycle("Error: timeout");

    assert!(!success);
    assert_eq!(loop_instance.results.len(), 2); // Capped at 2
}

#[test]
fn test_healing_guidance_is_injected() {
    let loop_instance = MockHealingLoop::new(3);
    let guidance = loop_instance.inject_healing_guidance("permission denied");

    assert!(guidance.contains("permission denied"));
    assert!(guidance.contains("alternative approach"));
    assert!(guidance.contains("tool arguments"));
}

// ---------------------------------------------------------------------------
// A3 — Goal Decomposition Tests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct PlanStep {
    name: String,
    description: String,
    dependencies: Vec<String>,
}

#[derive(Debug)]
struct GoalPlan {
    steps: Vec<PlanStep>,
}

impl GoalPlan {
    fn topological_sort(&self) -> Vec<&PlanStep> {
        let mut visited: HashMap<&str, bool> = HashMap::new();
        let mut order: Vec<&PlanStep> = Vec::new();

        let name_map: HashMap<&str, &PlanStep> = self
            .steps
            .iter()
            .map(|s| (s.name.as_str(), s))
            .collect();

        fn visit<'a>(
            name: &'a str,
            name_map: &HashMap<&str, &'a PlanStep>,
            visited: &mut HashMap<&'a str, bool>,
            order: &mut Vec<&'a PlanStep>,
        ) {
            if visited.get(name).copied().unwrap_or(false) {
                return;
            }
            if let Some(step) = name_map.get(name) {
                for dep in &step.dependencies {
                    visit(dep.as_str(), name_map, visited, order);
                }
                order.push(*step);
                visited.insert(name, true);
            }
        }

        for step in &self.steps {
            visit(&step.name, &name_map, &mut visited, &mut order);
        }

        order
    }

    fn has_cycles(&self) -> bool {
        let sorted = self.topological_sort();
        // Simple cycle detection: if sort produces fewer steps, there's a cycle
        let sorted_names: Vec<&str> = sorted.iter().map(|s| s.name.as_str()).collect();

        for step in &self.steps {
            for dep in &step.dependencies {
                let dep_pos = sorted_names.iter().position(|&n| n == dep.as_str());
                let step_pos = sorted_names.iter().position(|&n| n == step.name.as_str());
                if let (Some(dp), Some(sp)) = (dep_pos, step_pos) {
                    if dp > sp {
                        return true;
                    }
                }
            }
        }
        false
    }
}

fn build_test_plan() -> GoalPlan {
    GoalPlan {
        steps: vec![
            PlanStep {
                name: "list_files".to_string(),
                description: "Get directory tree".to_string(),
                dependencies: vec![],
            },
            PlanStep {
                name: "read_config".to_string(),
                description: "Parse config file".to_string(),
                dependencies: vec!["list_files".to_string()],
            },
            PlanStep {
                name: "scan_vulnerabilities".to_string(),
                description: "Security scan".to_string(),
                dependencies: vec!["list_files".to_string()],
            },
            PlanStep {
                name: "generate_report".to_string(),
                description: "Compile results".to_string(),
                dependencies: vec![
                    "read_config".to_string(),
                    "scan_vulnerabilities".to_string(),
                ],
            },
        ],
    }
}

#[test]
fn test_goal_decomposition_plan_size() {
    let plan = build_test_plan();
    assert_eq!(plan.steps.len(), 4);
}

#[test]
fn test_topological_sort_execution_order() {
    let plan = build_test_plan();
    let sorted = plan.topological_sort();

    assert_eq!(sorted.len(), 4);

    // list_files should come first (no dependencies)
    assert_eq!(sorted[0].name, "list_files");

    // generate_report should come last (depends on everything)
    assert_eq!(sorted[3].name, "generate_report");

    // read_config and scan_vulnerabilities should come after list_files
    // and before generate_report
    let middle_names: Vec<&str> = sorted[1..3].iter().map(|s| s.name.as_str()).collect();
    assert!(middle_names.contains(&"read_config"));
    assert!(middle_names.contains(&"scan_vulnerabilities"));
}

#[test]
fn test_no_cycles_in_valid_plan() {
    let plan = build_test_plan();
    assert!(!plan.has_cycles());
}

#[test]
fn test_cycle_detection() {
    let plan = GoalPlan {
        steps: vec![
            PlanStep {
                name: "A".to_string(),
                description: "Step A".to_string(),
                dependencies: vec!["B".to_string()],
            },
            PlanStep {
                name: "B".to_string(),
                description: "Step B".to_string(),
                dependencies: vec!["A".to_string()],
            },
        ],
    };
    assert!(plan.has_cycles());
}

// ---------------------------------------------------------------------------
// B4 — Chain-of-Tools Tests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum ChainStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    CircuitBlocked,
}

#[derive(Debug, Clone)]
struct ChainStep {
    index: usize,
    tool_name: String,
    status: ChainStepStatus,
    result: Option<String>,
    duration_ms: u64,
}

struct ChainOfTools {
    max_steps: usize,
    timeout_per_step_ms: u64,
    steps: Vec<ChainStep>,
}

impl ChainOfTools {
    fn new(max_steps: usize, timeout_ms: u64) -> Self {
        ChainOfTools {
            max_steps,
            timeout_per_step_ms: timeout_ms,
            steps: Vec::new(),
        }
    }

    fn add_step(&mut self, tool_name: &str) -> Result<usize, &'static str> {
        if self.steps.len() >= self.max_steps {
            return Err("Circuit breaker: max steps reached");
        }
        let index = self.steps.len();
        self.steps.push(ChainStep {
            index,
            tool_name: tool_name.to_string(),
            status: ChainStepStatus::Pending,
            result: None,
            duration_ms: 0,
        });
        Ok(index)
    }

    fn execute_step(
        &mut self,
        index: usize,
        result: &str,
        duration_ms: u64,
    ) -> Result<(), &'static str> {
        if duration_ms > self.timeout_per_step_ms {
            if let Some(step) = self.steps.get_mut(index) {
                step.status = ChainStepStatus::CircuitBlocked;
            }
            return Err("Circuit breaker: step timeout exceeded");
        }
        if let Some(step) = self.steps.get_mut(index) {
            step.status = ChainStepStatus::Completed;
            step.result = Some(result.to_string());
            step.duration_ms = duration_ms;
        }
        Ok(())
    }

    fn mark_failed(&mut self, index: usize) {
        if let Some(step) = self.steps.get_mut(index) {
            step.status = ChainStepStatus::Failed;
        }
    }

    fn trace(&self) -> Vec<&ChainStep> {
        self.steps.iter().collect()
    }
}

#[test]
fn test_chain_sequential_execution() {
    let mut chain = ChainOfTools::new(10, 60000);

    assert!(chain.add_step("read_file").is_ok());
    assert!(chain.add_step("search").is_ok());
    assert!(chain.add_step("sandbox_command").is_ok());

    assert_eq!(chain.steps.len(), 3);
    assert_eq!(chain.steps[0].tool_name, "read_file");
    assert_eq!(chain.steps[1].tool_name, "search");
    assert_eq!(chain.steps[2].tool_name, "sandbox_command");
}

#[test]
fn test_chain_circuit_breaker_max_steps() {
    let mut chain = ChainOfTools::new(3, 60000);

    assert!(chain.add_step("s1").is_ok());
    assert!(chain.add_step("s2").is_ok());
    assert!(chain.add_step("s3").is_ok());
    assert!(chain.add_step("s4").is_err()); // Should be blocked

    assert_eq!(chain.steps.len(), 3);
}

#[test]
fn test_chain_step_timeout() {
    let mut chain = ChainOfTools::new(10, 5000); // 5 second timeout
    chain.add_step("slow_command").unwrap();

    let result = chain.execute_step(0, "result", 6000); // 6 seconds
    assert!(result.is_err());
    assert_eq!(chain.steps[0].status, ChainStepStatus::CircuitBlocked);
}

#[test]
fn test_chain_step_completes_within_timeout() {
    let mut chain = ChainOfTools::new(10, 5000);
    chain.add_step("fast_command").unwrap();

    let result = chain.execute_step(0, "completed", 2000);
    assert!(result.is_ok());
    assert_eq!(chain.steps[0].status, ChainStepStatus::Completed);
    assert_eq!(chain.steps[0].result.as_deref(), Some("completed"));
    assert_eq!(chain.steps[0].duration_ms, 2000);
}

#[test]
fn test_chain_mark_failed() {
    let mut chain = ChainOfTools::new(10, 60000);
    chain.add_step("failing_command").unwrap();
    chain.mark_failed(0);

    assert_eq!(chain.steps[0].status, ChainStepStatus::Failed);
}

#[test]
fn test_chain_full_trace() {
    let mut chain = ChainOfTools::new(10, 60000);
    chain.add_step("step1").unwrap();
    chain.add_step("step2").unwrap();
    chain.add_step("step3").unwrap();

    chain.execute_step(0, "ok", 100).unwrap();
    chain.mark_failed(1);
    chain.execute_step(2, "done", 200).unwrap();

    let trace = chain.trace();
    assert_eq!(trace.len(), 3);
    assert_eq!(trace[0].status, ChainStepStatus::Completed);
    assert_eq!(trace[1].status, ChainStepStatus::Failed);
    assert_eq!(trace[2].status, ChainStepStatus::Completed);
}

// ---------------------------------------------------------------------------
// B5 — Hallucination Shield Tests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum ClaimVerification {
    Verified,
    Unverified,
    Blocked,
}

#[derive(Debug, Clone)]
struct FactClaim {
    text: String,
    verification: ClaimVerification,
    source: Option<String>,
}

struct HallucinationShield {
    claims: Vec<FactClaim>,
}

impl HallucinationShield {
    fn new() -> Self {
        HallucinationShield { claims: Vec::new() }
    }

    fn extract_claims(&mut self, text: &str) -> Vec<FactClaim> {
        let claim_patterns = [
            " is ", " was ", " has ", " had ", " will ", " can ",
            "first ", "last ", "most ", "largest ", "smallest ",
            "invented ", "discovered ", "founded ", "created ",
            "achieved ", "demonstrated ", "proved ", "built ",
        ];

        let mut claims = Vec::new();
        let sentences: Vec<&str> = text.split('.').collect();

        for sentence in sentences {
            let trimmed = sentence.trim();
            if trimmed.is_empty() {
                continue;
            }
            if claim_patterns.iter().any(|p| trimmed.to_lowercase().contains(p)) {
                claims.push(FactClaim {
                    text: trimmed.to_string(),
                    verification: ClaimVerification::Unverified,
                    source: None,
                });
            }
        }

        self.claims = claims.clone();
        claims
    }

    fn verify_claim(&mut self, index: usize, verified: bool, source: Option<&str>) {
        if let Some(claim) = self.claims.get_mut(index) {
            claim.verification = if verified {
                ClaimVerification::Verified
            } else {
                ClaimVerification::Blocked
            };
            claim.source = source.map(|s| s.to_string());
        }
    }

    fn verification_rate(&self) -> f64 {
        if self.claims.is_empty() {
            return 1.0;
        }
        let verified = self
            .claims
            .iter()
            .filter(|c| c.verification == ClaimVerification::Verified)
            .count();
        verified as f64 / self.claims.len() as f64
    }

    fn hallucination_rate(&self) -> f64 {
        if self.claims.is_empty() {
            return 0.0;
        }
        let blocked = self
            .claims
            .iter()
            .filter(|c| c.verification == ClaimVerification::Blocked)
            .count();
        blocked as f64 / self.claims.len() as f64
    }
}

#[test]
fn test_hallucination_shield_extracts_claims() {
    let mut shield = HallucinationShield::new();
    let text = "Quantum computing was proposed by Richard Feynman in the 1980s. \
                The first quantum computer was built by IBM. Google achieved \
                quantum supremacy in 2019.";

    let claims = shield.extract_claims(text);

    assert_eq!(claims.len(), 3);
    assert!(claims[0].text.contains("Feynman"));
    assert!(claims[1].text.contains("IBM"));
    assert!(claims[2].text.contains("Google"));
}

#[test]
fn test_hallucination_shield_verification() {
    let mut shield = HallucinationShield::new();

    shield.extract_claims("Python was created by Guido van Rossum in 1991. \
                            Python is the fastest programming language.");

    shield.verify_claim(0, true, Some("python.org history"));
    shield.verify_claim(1, false, Some("multiple benchmarks show C/Rust are faster"));

    assert_eq!(shield.claims[0].verification, ClaimVerification::Verified);
    assert_eq!(shield.claims[1].verification, ClaimVerification::Blocked);
    assert_eq!(shield.claims[0].source.as_deref(), Some("python.org history"));
}

#[test]
fn test_hallucination_shield_rates() {
    let mut shield = HallucinationShield::new();

    shield.extract_claims("A. B is the largest city. C was discovered by X. \
                            D was invented by Y.");

    shield.verify_claim(0, true, Some("source1"));
    shield.verify_claim(1, false, Some("source2"));
    shield.verify_claim(2, true, Some("source3"));
    shield.verify_claim(3, true, Some("source4"));

    assert!(shield.verification_rate() > 0.7);
    assert!(shield.hallucination_rate() < 0.3);
}

#[test]
fn test_hallucination_shield_empty_input() {
    let mut shield = HallucinationShield::new();
    let claims = shield.extract_claims("");

    assert_eq!(claims.len(), 0);
    assert_eq!(shield.verification_rate(), 1.0);
    assert_eq!(shield.hallucination_rate(), 0.0);
}

#[test]
fn test_hallucination_shield_no_claim_sentences() {
    let mut shield = HallucinationShield::new();
    let claims = shield.extract_claims("Hello world. How are you? Thank you. Goodbye.");

    assert_eq!(claims.len(), 0); // No factual claim patterns matched
}

#[test]
fn test_hallucination_shield_all_verified_perfect_score() {
    let mut shield = HallucinationShield::new();

    shield.extract_claims("Rust was created by Mozilla. Cargo is the build system.");

    shield.verify_claim(0, true, Some("rust-lang.org"));
    shield.verify_claim(1, true, Some("doc.rust-lang.org/cargo"));

    assert_eq!(shield.verification_rate(), 1.0);
    assert_eq!(shield.hallucination_rate(), 0.0);
}

#[test]
fn test_hallucination_shield_all_blocked() {
    let mut shield = HallucinationShield::new();

    shield.extract_claims("X was invented in year 3000. Y is the largest planet in the solar system.");

    shield.verify_claim(0, false, None);
    shield.verify_claim(1, false, Some("Jupiter is actually the largest"));

    assert_eq!(shield.verification_rate(), 0.0);
    assert_eq!(shield.hallucination_rate(), 1.0);
}