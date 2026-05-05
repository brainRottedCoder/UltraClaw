use crate::skill::{Skill, SkillOutput};
use serde_json::Value;

/// Web Search Skill using DuckDuckGo Lite.
/// No API keys required, making it robust and free.
pub struct SearchSkill;

impl Skill for SearchSkill {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> &'static str {
        "Search the web for real-time information. Useful for answering questions about recent events or facts."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                }
            },
            "required": ["query"]
        })
    }

    fn execute_sync(&self, args: &Value) -> SkillOutput {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        
        // Use blocking reqwest client to hit DuckDuckGo HTML version
        let url = format!("https://html.duckduckgo.com/html/?q={}", urlencoding::encode(query));
        
        let client = reqwest::blocking::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());

        match client.get(&url).send() {
            Ok(resp) => {
                let html: String = resp.text().unwrap_or_default();
                // Extremely simple scraping: look for snippets
                let mut results = String::new();
                let doc = html.split("a class=\"result__snippet");
                for (i, part) in doc.skip(1).enumerate() {
                    if i >= 3 { break; } // Top 3 results
                    if let Some(start) = part.find('>') {
                        if let Some(end) = part.find("</a>") {
                            let raw_text = &part[start + 1..end];
                            // Strip inner html tags
                            let clean_text = raw_text.replace("<b>", "").replace("</b>", "").replace("&#39;", "'").replace("&quot;", "\"");
                            results.push_str(&format!("Result {}:\n{}\n\n", i + 1, clean_text));
                        }
                    }
                }

                if results.is_empty() {
                    results = "No results found or parsing failed.".to_string();
                }

                SkillOutput {
                    name: self.name().to_string(),
                    output: results,
                    is_error: false,
                }
            }
            Err(e) => {
                SkillOutput {
                    name: self.name().to_string(),
                    output: format!("Search request failed: {}", e),
                    is_error: true,
                }
            }
        }
    }
}
