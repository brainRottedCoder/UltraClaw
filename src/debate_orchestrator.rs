use crate::db::ChatMessage;
use crate::inference::InferenceEngine;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebatePerspective {
    pub name: String,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateArgument {
    pub perspective: String,
    pub argument: String,
    pub key_points: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebateSynthesis {
    pub question: String,
    pub arguments: Vec<DebateArgument>,
    pub consensus: String,
    pub confidence: f32,
    pub total_time_ms: u64,
}

impl DebateSynthesis {
    pub fn summary(&self) -> String {
        format!(
            "Debate: {} perspectives, consensus confidence: {:.0}%, {}ms",
            self.arguments.len(),
            self.confidence * 100.0,
            self.total_time_ms
        )
    }
}

fn default_perspectives() -> Vec<DebatePerspective> {
    vec![
        DebatePerspective {
            name: "Optimist".to_string(),
            system_prompt: concat!(
                "You are the OPTIMIST in a structured debate. Your role is to find and emphasize ",
                "the POSITIVE aspects, benefits, opportunities, and strengths of any proposal. ",
                "Be enthusiastic but factual. Highlight what could go RIGHT. ",
                "Consider best-case scenarios and upside potential. ",
                "Always find at least 3 positive points. ",
                "End with: 'From the optimistic perspective, I recommend...' "
            ).to_string(),
        },
        DebatePerspective {
            name: "Pessimist".to_string(),
            system_prompt: concat!(
                "You are the PESSIMIST in a structured debate. Your role is to identify and articulate ",
                "the NEGATIVE aspects, risks, downsides, and weaknesses of any proposal. ",
                "Be critical but constructive. Highlight what could go WRONG. ",
                "Consider worst-case scenarios, edge cases, and hidden costs. ",
                "Always find at least 3 concerns or risks. ",
                "End with: 'From the skeptical perspective, I caution that...' "
            ).to_string(),
        },
        DebatePerspective {
            name: "Analyst".to_string(),
            system_prompt: concat!(
                "You are the ANALYST in a structured debate. Your role is to provide a BALANCED, ",
                "evidence-based evaluation. Weigh pros and cons objectively. ",
                "Consider trade-offs, opportunity costs, and practical constraints. ",
                "Provide a neutral assessment with reasoning. ",
                "Evaluate both the optimist and pessimist positions. ",
                "End with: 'From an analytical perspective, the balanced view is...' "
            ).to_string(),
        },
    ]
}

pub async fn run_debate(
    question: &str,
    engine: &dyn InferenceEngine,
) -> Result<DebateSynthesis, String> {
    let start_time = Instant::now();
    let perspectives = default_perspectives();
    let mut arguments = Vec::new();

    for perspective in &perspectives {
        let mut msgs = vec![
            ChatMessage {
                role: "system".to_string(),
                content: perspective.system_prompt.clone(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "DEBATE QUESTION: {}\n\nProvide your perspective. Be thorough but concise (150-300 words).",
                    question
                ),
            },
        ];

        let response = engine
            .infer(msgs, None, 0.4, 1024)
            .await
            .unwrap_or_else(|e| {
                warn!("Debate perspective failed ({}): {}", perspective.name, e);
                format!(
                    "[Perspective '{}' failed to generate: {}]",
                    perspective.name, e
                )
            });

        let key_points = extract_key_points(&response);
        let pts_len = key_points.len();

        arguments.push(DebateArgument {
            perspective: perspective.name.clone(),
            argument: response,
            key_points,
        });

        info!(
            perspective = %perspective.name,
            points = pts_len,
            "Debate argument collected"
        );
    }

    let debate_text: String = arguments
        .iter()
        .map(|a| {
            format!(
                "=== {} ===\n{}\nKey points: {}\n",
                a.perspective,
                a.argument,
                a.key_points.join("; ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let synthesis_prompt = format!(
        r#"You are a debate synthesizer. Review these perspectives on the question:

QUESTION: {}

{}

Your task:
1. Identify points of agreement and disagreement
2. Determine which arguments are strongest
3. Provide a consensus recommendation
4. Rate your confidence (0.0 to 1.0)

Output as JSON:
{{
  "consensus": "The final recommendation...",
  "confidence": 0.85,
  "agreement_areas": ["point 1", "point 2"],
  "disagreement_areas": ["point 1", "point 2"]
}}

Be decisive. Pick the BEST course of action based on all arguments."#,
        question, debate_text
    );

    let synth_msgs = vec![ChatMessage {
        role: "user".to_string(),
        content: synthesis_prompt,
    }];

    let synth_response = engine
        .infer(synth_msgs, None, 0.2, 1024)
        .await
        .unwrap_or_else(|e| {
            warn!("Debate synthesis failed: {}", e);
            format!(
                r#"{{"consensus": "Could not synthesize debate. Error: {}", "confidence": 0.0}}"#,
                e
            )
        });

    let (consensus, confidence) = parse_synthesis(&synth_response);

    let total_time_ms = start_time.elapsed().as_millis() as u64;

    info!(
        perspectives_count = arguments.len(),
        confidence = confidence,
        ms = total_time_ms,
        "Debate complete"
    );

    Ok(DebateSynthesis {
        question: question.to_string(),
        arguments,
        consensus,
        confidence,
        total_time_ms,
    })
}

fn extract_key_points(response: &str) -> Vec<String> {
    let mut points = Vec::new();

    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with("•") {
            let point = trimmed.trim_start_matches(|c| c == '-' || c == '*' || c == '•' || c == ' ');
            if point.len() > 10 && point.len() < 150 {
                points.push(point.to_string());
            }
        }

        if trimmed
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            let point = trimmed
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')' || c == ' ');
            if point.len() > 10 && point.len() < 150 {
                points.push(point.to_string());
            }
        }
    }

    let has_colon_lines: Vec<&str> = response
        .lines()
        .filter(|l| l.contains(":") && l.len() > 20 && l.len() < 200)
        .map(|l| l.trim())
        .take(3)
        .collect();

    if !has_colon_lines.is_empty() {
        for line in has_colon_lines {
            if !points.iter().any(|p| p.as_str() == line) {
                points.push(line.to_string());
            }
        }
    }

    points.truncate(5);
    points
}

fn parse_synthesis(response: &str) -> (String, f32) {
    let json_str = if let Some(start) = response.find('{') {
        if let Some(end) = response.rfind('}') {
            &response[start..=end]
        } else {
            return (response.to_string(), 0.5);
        }
    } else {
        return (response.to_string(), 0.5);
    };

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
        let consensus = v
            .get("consensus")
            .and_then(|c| c.as_str())
            .unwrap_or("No consensus reached")
            .to_string();
        let confidence = v
            .get("confidence")
            .and_then(|c| c.as_f64())
            .unwrap_or(0.5) as f32;

        (consensus, confidence.clamp(0.0, 1.0))
    } else {
        let first_line = response.lines().next().unwrap_or("Synthesis unavailable");
        (first_line.to_string(), 0.5)
    }
}