use crate::db::ChatMessage;
use crate::inference::InferenceEngine;
use crate::skill::{Skill, SkillOutput};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HallucinationMetrics {
    pub claims_made: usize,
    pub claims_verified: usize,
    pub claims_unverified: usize,
    pub hallucinations_blocked: usize,
}

impl HallucinationMetrics {
    pub fn record_claim(&mut self) {
        self.claims_made += 1;
    }

    pub fn record_verified(&mut self) {
        self.claims_verified += 1;
    }

    pub fn record_unverified(&mut self) {
        self.claims_unverified += 1;
    }

    pub fn record_blocked(&mut self) {
        self.hallucinations_blocked += 1;
    }

    pub fn summary(&self) -> String {
        format!(
            "Hallucination Shield: {} claims, {} verified, {} unverified, {} blocked ({}% grounded)",
            self.claims_made,
            self.claims_verified,
            self.claims_unverified,
            self.hallucinations_blocked,
            if self.claims_made > 0 {
                self.claims_verified * 100 / self.claims_made
            } else {
                100
            }
        )
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn dashboard_line(&self) -> String {
        format!(
            "  🔍 Facts: {} | ✅ Verified: {} | ⚠ Unverified: {} | 🚫 Blocked: {}",
            self.claims_made, self.claims_verified, self.claims_unverified, self.hallucinations_blocked
        )
    }
}

pub struct HallucinationDetectionSkill {
    metrics: Arc<std::sync::Mutex<HallucinationMetrics>>,
}

impl HallucinationDetectionSkill {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            metrics: Arc::new(std::sync::Mutex::new(HallucinationMetrics::default())),
        })
    }

    pub fn get_metrics(&self) -> HallucinationMetrics {
        self.metrics.lock().unwrap().clone()
    }

    pub fn reset_metrics(&self) {
        self.metrics.lock().unwrap().reset();
    }

    pub async fn verify_response(
        metrics: Arc<std::sync::Mutex<HallucinationMetrics>>,
        engine: &dyn InferenceEngine,
        response: &str,
        temperature: f32,
    ) -> Result<String, String> {
        let claims = extract_factual_claims(response);

        if claims.is_empty() {
            return Ok(response.to_string());
        }

        for claim in &claims {
            if let Ok(m) = metrics.lock() {
                let mut m = m;
                m.record_claim();
            }
        }

        let num_claims = claims.len();
        if num_claims < 3 {
            return Ok(response.to_string());
        }

        let claim_list = claims
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}. {}", i + 1, c))
            .collect::<Vec<_>>()
            .join("\n");

        let verify_prompt = format!(
            r#"You are a fact-checker. Review these claims from a previous response:

{}
        
For each claim, respond with YES (can be verified based on retrieved context) or NO (cannot verify).
        
Previous context available in the conversation provides source documents. 
Only mark claims as verified if there's direct supporting evidence.
        
Respond in this format:
```
1. VERIFIED: [explanation]
2. UNVERIFIED: [explanation]
...
```

Be strict. If unsure, mark UNVERIFIED."#,
            claim_list
        );

        let verify_msgs = vec![ChatMessage {
            role: "user".to_string(),
            content: verify_prompt,
        }];

        let verify_response = engine
            .infer(verify_msgs, None, 0.1, 1024)
            .await
            .unwrap_or_else(|_| "VERIFICATION_FAILED".to_string());

        let mut verified_count = 0;
        let mut unverified_count = 0;

        for line in verify_response.lines() {
            let upper = line.to_uppercase();
            if upper.contains("VERIFIED") {
                verified_count += 1;
            } else if upper.contains("UNVERIFIED") || upper.contains("CANNOT") {
                unverified_count += 1;
            }
        }

        if let Ok(m) = metrics.lock() {
            let mut m = m;
            for _ in 0..verified_count {
                m.record_verified();
            }
            for _ in 0..unverified_count {
                m.record_unverified();
                m.record_blocked();
            }
        }

        let verif_percent = if num_claims > 0 {
            (verified_count * 100) / num_claims
        } else {
            100
        };

        if verif_percent < 50 {
            let corrected = format!(
                "{}\n\n[NOTE: {}% of factual claims in this response could not be verified. Please treat with caution.]",
                response, 100 - verif_percent
            );
            Ok(corrected)
        } else {
            let annotated = format!(
                "{}",
                response
            );
            Ok(annotated)
        }
    }
}

fn extract_factual_claims(response: &str) -> Vec<String> {
    let mut claims = Vec::new();

    for line in response.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("```") || line.starts_with('-') && line.len() < 10 {
            continue;
        }

        let has_number = line.chars().any(|c| c.is_ascii_digit());
        let has_proper_name = line.chars().any(|c| c.is_uppercase())
            && line.chars().filter(|c| c.is_uppercase()).count() > 1;

        let has_tech_term = ["function", "module", "crate", "struct", "enum", "impl", "trait", "API", "endpoint"]
            .iter()
            .any(|t| line.contains(t));

        if (has_number || has_proper_name || has_tech_term) && line.len() > 30 && line.len() < 300 {
            claims.push(line.to_string());
        }

        if claims.len() >= 10 {
            break;
        }
    }

    claims
}

pub async fn quick_fact_check(
    response: &str,
    engine: &dyn InferenceEngine,
    existing_count: usize,
) -> (usize, usize, String) {
    let claims = extract_factual_claims(response);
    let total = existing_count + claims.len();

    if claims.is_empty() {
        return (0, 0, response.to_string());
    }

    let check_count = claims.len().min(5);
    let subset: Vec<_> = claims.iter().take(check_count).collect();

    let check_prompt = format!(
        r#"Quick fact-check these {} claims. Mark each as TRUE or UNCERTAIN.
Respond ONLY with "TRUE: N" and "UNCERTAIN: M" on separate lines.

Claims:
{}
"#,
        check_count,
        subset.iter().enumerate().map(|(i, c)| format!("{}. {}", i + 1, c)).collect::<Vec<_>>().join("\n")
    );

    let check_msgs = vec![ChatMessage {
        role: "user".to_string(),
        content: check_prompt,
    }];

    let check_result = engine
        .infer(check_msgs, None, 0.0, 256)
        .await
        .unwrap_or_default();

    let true_count = check_result.to_uppercase().matches("TRUE").count();
    let uncertain_count = check_count.saturating_sub(true_count);

    let annotated = if uncertain_count > 0 {
        format!("{}\n\n---\nFact-check: {} of {} sampled claims verified.", response, true_count, check_count)
    } else {
        response.to_string()
    };

    (true_count, uncertain_count, annotated)
}