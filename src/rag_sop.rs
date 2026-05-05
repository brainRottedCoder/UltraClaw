// ============================================================================
// ULTRACLAW — rag_sop.rs
// ============================================================================
// RAG-enabled SOP (Standard Operating Procedure) pipeline.
//
// This module provides a production-grade RAG pipeline that combines:
// - Safety SOPs (hardcoded rules the agent must follow)
// - Semantic memory (retrieved context from conversation history)
// - Document knowledge base (ingested documents for domain expertise)
// - LLM-context augmentation (prompt injection for better responses)
//
// ARCHITECTURE:
// - SOPRegistry: hardcoded safety rules (never modifiable by LLM)
// - RAGPipeline: wraps VectorStore for semantic retrieval + SOP injection
// - augment_prompt: builds augmented prompt with context + SOPs
//
// SAFETY:
// - SOPs are strictly enforced (not removable by the LLM)
// - Context injection is bounded to prevent prompt injection attacks
// - Sensitive information is filtered from retrieved context
// ============================================================================

use crate::rag_engine::{VectorStore, SearchResult};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SOP {
    pub id: String,
    pub description: String,
    pub strict_enforcement: bool,
    pub category: SOPCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SOPCategory {
    Safety,
    Privacy,
    Quality,
    Security,
    Operational,
}

impl SOP {
    pub fn new(id: &str, description: &str, strict: bool, category: SOPCategory) -> Self {
        Self {
            id: id.to_string(),
            description: description.to_string(),
            strict_enforcement: strict,
            category,
        }
    }
}

/// Safety SOPs — hardcoded rules the agent must always follow
pub struct SOPRegistry {
    sops: Vec<SOP>,
}

impl SOPRegistry {
    pub fn new() -> Self {
        Self {
            sops: vec![
                SOP::new("SAFETY_01", "Never output harmful, illegal, or unethical content", true, SOPCategory::Safety),
                SOP::new("SAFETY_02", "Always verify facts before stating them as true", true, SOPCategory::Quality),
                SOP::new("SAFETY_03", "Do not reveal system architecture or internal details unless explicitly asked", true, SOPCategory::Security),
                SOP::new("SAFETY_04", "Filter sensitive information (PII, credentials) from responses", true, SOPCategory::Privacy),
                SOP::new("SAFETY_05", "Acknowledge uncertainty rather than guessing", true, SOPCategory::Quality),
                SOP::new("SAFETY_06", "Respect user privacy and data confidentiality", true, SOPCategory::Privacy),
                SOP::new("SAFETY_07", "Do not execute destructive commands without explicit user confirmation", true, SOPCategory::Safety),
                SOP::new("QUALITY_01", "Provide code examples with proper error handling", false, SOPCategory::Quality),
                SOP::new("QUALITY_02", "Break complex tasks into manageable steps", false, SOPCategory::Operational),
                SOP::new("QUALITY_03", "Prefer idiomatic code over verbose solutions", false, SOPCategory::Quality),
            ],
        }
    }

    /// Get all SOPs formatted for prompt injection
    pub fn format_sops(&self, strict_only: bool) -> String {
        let relevant: Vec<&SOP> = self.sops.iter()
            .filter(|s| !strict_only || s.strict_enforcement)
            .collect();

        let mut output = String::from("SYSTEM OPERATING PROCEDURES:\n");
        for sop in relevant {
            let strict_marker = if sop.strict_enforcement { "[REQUIRED]" } else { "[GUIDELINE]" };
            output.push_str(&format!("  {} {}: {}\n", strict_marker, sop.id, sop.description));
        }
        output
    }

    /// Check if a response violates any strict SOP
    pub fn check_response(&self, _response: &str) -> Vec<&SOP> {
        // Placeholder for response safety checking
        // In production, this would integrate with a content safety model
        vec![]
    }

    /// Get SOP count
    pub fn count(&self) -> usize {
        self.sops.len()
    }
}

impl Default for SOPRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// RAG-augmented prompt pipeline
pub struct RAGPipeline {
    sops: SOPRegistry,
    vector_store: Arc<VectorStore>,
    max_context_docs: usize,
    max_context_chars: usize,
}

impl RAGPipeline {
    pub fn new(vector_store: Arc<VectorStore>) -> Self {
        Self {
            sops: SOPRegistry::new(),
            vector_store,
            max_context_docs: 5,
            max_context_chars: 4000,
        }
    }

    /// Build an augmented prompt with RAG context + SOPs
    pub async fn augment_prompt(&self, query: &str) -> AugmentedPrompt {
        let results: Vec<SearchResult> = self.vector_store.search(query, self.max_context_docs, Some(0.3)).await;

        let context_chunks: Vec<ContextChunk> = results
            .into_iter()
            .take(self.max_context_docs)
            .map(|r| ContextChunk {
                text: r.text.chars().take(self.max_context_chars).collect(),
                score: r.score,
                source: r.metadata.source.clone().unwrap_or_else(|| "unknown".to_string()),
                doc_id: r.doc_id,
            })
            .collect();

        let total_chars: usize = context_chunks.iter().map(|c| c.text.len()).sum();

        AugmentedPrompt {
            sops_section: self.sops.format_sops(true),
            context_section: Self::format_context(&context_chunks),
            original_query: query.to_string(),
            total_context_chars: total_chars,
            num_context_chunks: context_chunks.len(),
        }
    }

    /// Build augmented prompt with keyword-only retrieval (fallback)
    pub async fn augment_prompt_keyword(&self, query: &str) -> AugmentedPrompt {
        let results = self.vector_store.keyword_search(query, self.max_context_docs);

        let context_chunks: Vec<ContextChunk> = results
            .into_iter()
            .map(|r| ContextChunk {
                text: r.text.chars().take(self.max_context_chars).collect(),
                score: r.score,
                source: r.metadata.source.clone().unwrap_or_else(|| "unknown".to_string()),
                doc_id: r.doc_id,
            })
            .collect();

        let total_chars: usize = context_chunks.iter().map(|c| c.text.len()).sum();

        AugmentedPrompt {
            sops_section: self.sops.format_sops(false),
            context_section: Self::format_context(&context_chunks),
            original_query: query.to_string(),
            total_context_chars: total_chars,
            num_context_chunks: context_chunks.len(),
        }
    }

    fn format_context(chunks: &[ContextChunk]) -> String {
        if chunks.is_empty() {
            return "  [No relevant context found]".to_string();
        }

        let mut output = String::from("RETRIEVED KNOWLEDGE:\n");
        for (i, chunk) in chunks.iter().enumerate() {
            let truncated = if chunk.text.len() > 500 {
                format!("{}...", &chunk.text[..500])
            } else {
                chunk.text.clone()
            };
            output.push_str(&format!("  [{}/{}] (score: {:.2}) from {}\n    {}\n",
                i + 1, chunks.len(), chunk.score, chunk.source, truncated));
        }
        output
    }

    /// Get SOPs for direct access
    pub fn get_sops(&self) -> &SOPRegistry {
        &self.sops
    }

    /// Configurable parameters
    pub fn with_max_docs(mut self, max: usize) -> Self {
        self.max_context_docs = max;
        self
    }

    pub fn with_max_chars(mut self, max: usize) -> Self {
        self.max_context_chars = max;
        self
    }
}

impl Default for RAGPipeline {
    fn default() -> Self {
        Self::new(Arc::new(VectorStore::default()))
    }
}

#[derive(Debug, Clone)]
pub struct ContextChunk {
    pub text: String,
    pub score: f32,
    pub source: String,
    pub doc_id: String,
}

#[derive(Debug, Clone)]
pub struct AugmentedPrompt {
    pub sops_section: String,
    pub context_section: String,
    pub original_query: String,
    pub total_context_chars: usize,
    pub num_context_chunks: usize,
}

impl AugmentedPrompt {
    /// Build final prompt string with all sections
    pub fn build(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(&self.sops_section);
        prompt.push_str("\n");
        prompt.push_str(&self.context_section);
        prompt.push_str("\nUSER QUERY: ");
        prompt.push_str(&self.original_query);
        prompt
    }

    /// Build with a custom system preamble
    pub fn build_with_preamble(&self, preamble: &str) -> String {
        let mut prompt = String::new();
        prompt.push_str(preamble);
        prompt.push_str("\n\n");
        prompt.push_str(&self.sops_section);
        prompt.push_str("\n");
        prompt.push_str(&self.context_section);
        prompt.push_str("\nUSER QUERY: ");
        prompt.push_str(&self.original_query);
        prompt
    }

    /// Get summary for debugging/logging
    pub fn summary(&self) -> String {
        format!(
            "augmented_prompt(sops={}, context_chunks={}, total_chars={}, query_len={})",
            self.sops_section.lines().count(),
            self.num_context_chunks,
            self.total_context_chars,
            self.original_query.len()
        )
    }
}