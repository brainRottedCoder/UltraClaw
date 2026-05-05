// ============================================================================
// ULTRACLAW — rag_engine.rs
// ============================================================================
// RAG (Retrieval-Augmented Generation) Engine with in-memory vector store.
//
// This module provides a complete RAG pipeline:
// - In-memory vector store with cosine similarity search
// - Document ingestion with automatic embedding generation
// - Hybrid retrieval (BM25 keyword + vector semantic)
// - Configurable chunking and overlap
//
// ARCHITECTURE:
// - VectorStore: in-memory store with cosine similarity
//   Uses a flat index with O(N*d) search where N=documents, d=embedding_dim
//   For production scale, replace with LanceDB or Qdrant
// - Chunked indexing: documents are split into overlapping chunks
// - Metadata support: each chunk carries source, page, position metadata
//
// MEMORY:
// - Each document chunk stores: id (16B) + text (variable) + embedding (4*384=1536B) + metadata
// - With 384-dim embeddings, 10K chunks ≈ ~20MB RAM
// - Bounded index size prevents unbounded growth
//
// INTEGRATION:
// - Uses EmbeddingService for vector generation
// - Uses DocumentParserService for chunking
// - Returns results for context injection into LLM prompts
// ============================================================================

use crate::embedding_service::EmbeddingService;
use crate::document_parser::{DocumentParserService, TextChunk};
use crate::skill::{Skill, SkillOutput};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

const MAX_STORE_DOCUMENTS: usize = 50_000;
const EMBEDDING_DIM: usize = 384;

/// A single embedded chunk stored in the vector index
#[derive(Debug, Clone)]
struct IndexedChunk {
    id: String,
    text: String,
    embedding: Vec<f32>,
    doc_id: String,
    chunk_index: usize,
    metadata: ChunkMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChunkMetadata {
    pub source: Option<String>,
    pub page: Option<u32>,
    pub position: Option<usize>,
    pub file_type: Option<String>,
}

/// Search result with relevance score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub text: String,
    pub score: f32,
    pub doc_id: String,
    pub chunk_index: usize,
    pub metadata: ChunkMetadata,
    pub highlight: Option<String>,
}

impl SearchResult {
    pub fn new(chunk: &IndexedChunk, score: f32) -> Self {
        Self {
            id: chunk.id.clone(),
            text: chunk.text.clone(),
            score,
            doc_id: chunk.doc_id.clone(),
            chunk_index: chunk.chunk_index,
            metadata: chunk.metadata.clone(),
            highlight: Self::generate_highlight(&chunk.text, 200),
        }
    }

    fn generate_highlight(text: &str, max_len: usize) -> Option<String> {
        let trimmed = text.trim();
        if trimmed.len() <= max_len {
            Some(trimmed.to_string())
        } else {
            let truncated = &trimmed[..max_len];
            let last_space = truncated.rfind(' ').unwrap_or(max_len);
            Some(format!("{}...", &truncated[..last_space]))
        }
    }
}

/// In-memory vector store with cosine similarity
pub struct VectorStore {
    chunks: RwLock<HashMap<String, IndexedChunk>>,
    doc_ids: RwLock<HashMap<String, Vec<String>>>,
    embedding_service: Arc<EmbeddingService>,
    max_documents: usize,
}

impl VectorStore {
    pub fn new(embedding_service: Arc<EmbeddingService>, max_documents: Option<usize>) -> Arc<Self> {
        Arc::new(Self {
            chunks: RwLock::new(HashMap::new()),
            doc_ids: RwLock::new(HashMap::new()),
            embedding_service,
            max_documents: max_documents.unwrap_or(MAX_STORE_DOCUMENTS),
        })
    }

    /// Add a document from a file path (parse + chunk + embed)
    pub async fn add_document(
        &self,
        path: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Result<DocumentAddResult, String> {
        let doc_id = Uuid::new_v4().to_string();

        // Parse the document
        let parser = Arc::new(DocumentParserService::new());
        let result = parser.parse(path).await?;
        let file_type = result.file_type.clone();

        // Collect all text
        let all_text = if let Some(pages) = result.pages {
            pages.iter().map(|p| p.text.clone()).collect::<Vec<_>>().join("\n")
        } else {
            result.text.unwrap_or_default()
        };

        if all_text.trim().is_empty() {
            return Err("Document contains no extractable text".to_string());
        }

        // Chunk the text
        let chunks = parser.chunk_text(&all_text, chunk_size, overlap).await?;

        // Add chunks to index
        let mut chunk_ids = Vec::new();
        for (idx, chunk) in chunks.iter().enumerate() {
            let chunk_id = self.add_chunk_internal(
                &doc_id,
                idx,
                &chunk.text,
                file_type.clone(),
                Some(path),
            ).await?;

            chunk_ids.push(chunk_id);
        }

        let total_chunks = chunk_ids.len();

        {
            let mut doc_ids = self.doc_ids.write().await;
            doc_ids.insert(doc_id.clone(), chunk_ids);
        }

        info!(doc_id = %doc_id, chunks = total_chunks, path = %path, "Document indexed");

        Ok(DocumentAddResult {
            doc_id,
            chunks_added: total_chunks,
            num_chars: all_text.len() as u64,
        })
    }

    /// Add a raw text document
    pub async fn add_text(
        &self,
        text: &str,
        doc_id: Option<&str>,
        chunk_size: usize,
        overlap: usize,
        source: Option<&str>,
    ) -> Result<String, String> {
        let parser = Arc::new(DocumentParserService::new());
        let chunks = parser.chunk_text(text, chunk_size, overlap).await?;

        let doc_id = doc_id.map(|s| s.to_string()).unwrap_or_else(|| Uuid::new_v4().to_string());

        let mut chunk_ids = Vec::new();
        for (idx, chunk) in chunks.iter().enumerate() {
            let chunk_id = self.add_chunk_internal(&doc_id, idx, &chunk.text, "text".to_string(), source)
                .await?;
            chunk_ids.push(chunk_id);
        }

        {
            let mut doc_ids = self.doc_ids.write().await;
            doc_ids.insert(doc_id.clone(), chunk_ids);
        }

        Ok(doc_id)
    }

    /// Internal: add a single chunk to the index
    async fn add_chunk_internal(
        &self,
        doc_id: &str,
        chunk_index: usize,
        text: &str,
        file_type: String,
        source: Option<&str>,
    ) -> Result<String, String> {
        // Generate embedding
        let embedding = self.embedding_service.embed(text).await?;

        let chunk_id = Uuid::new_v4().to_string();
        let chunk = IndexedChunk {
            id: chunk_id.clone(),
            text: text.to_string(),
            embedding,
            doc_id: doc_id.to_string(),
            chunk_index,
            metadata: ChunkMetadata {
                source: source.map(|s| s.to_string()),
                page: None,
                position: None,
                file_type: Some(file_type),
            },
        };

        let mut chunks = self.chunks.write().await;

        // Enforce max size limit
        if chunks.len() >= self.max_documents {
            warn!("Vector store at capacity ({} chunks), rejecting new insert", chunks.len());
            return Err("Vector store at maximum capacity".to_string());
        }

        chunks.insert(chunk_id.clone(), chunk);
        Ok(chunk_id)
    }

    /// Search by semantic similarity
    pub async fn search(&self, query: &str, top_k: usize, min_score: Option<f32>) -> Vec<SearchResult> {
        let query_embedding = match self.embedding_service.embed(query).await {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "Failed to embed query");
                return vec![];
            }
        };

        let chunks = self.chunks.read().await;
        let min_score = min_score.unwrap_or(0.0);

        // Compute cosine similarity for all chunks
        let mut scored: Vec<(String, f32)> = chunks
            .iter()
            .map(|(id, chunk)| {
                let score = cosine_similarity(&query_embedding, &chunk.embedding);
                (id.clone(), score)
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Take top_k and filter by min_score
        scored
            .into_iter()
            .take(top_k * 2) // Take extra for deduplication
            .filter(|(_, score)| *score >= min_score)
            .filter_map(|(id, score)| {
                chunks.get(&id).map(|chunk| SearchResult::new(chunk, score))
            })
            .collect()
    }

    /// Search by pre-computed embedding vector
    pub async fn search_by_embedding(&self, embedding: &[f32], top_k: usize, min_score: Option<f32>) -> Vec<SearchResult> {
        let chunks = self.chunks.read().await;
        let min_score = min_score.unwrap_or(0.0);

        let mut scored: Vec<(String, f32)> = chunks
            .iter()
            .map(|(id, chunk)| {
                let score = cosine_similarity(embedding, &chunk.embedding);
                (id.clone(), score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(top_k * 2)
            .filter(|(_, score)| *score >= min_score)
            .filter_map(|(id, score)| {
                chunks.get(&id).map(|chunk| SearchResult::new(chunk, score))
            })
            .collect()
    }

    /// Hybrid search: semantic + keyword (BM25-lite)
    pub async fn hybrid_search(
        &self,
        query: &str,
        top_k: usize,
        semantic_weight: f32,
    ) -> Vec<SearchResult> {
        let semantic_results = self.search(query, top_k * 2, Some(0.1)).await;
        let keyword_results = self.keyword_search(query, top_k * 2);

        // Merge and deduplicate
        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<SearchResult> = Vec::new();

        // Add semantic results (higher weight)
        for mut r in semantic_results {
            if seen.insert(r.id.clone()) {
                r.score = r.score * semantic_weight;
                merged.push(r);
            }
        }

        // Add keyword results with lower weight
        for mut r in keyword_results {
            if seen.insert(r.id.clone()) {
                r.score = r.score * (1.0 - semantic_weight);
                merged.push(r);
            }
        }

        // Sort by combined score
        merged.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        merged.truncate(top_k);
        merged
    }

    /// Simple keyword search (BM25-lite)
    pub fn keyword_search(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        let query_terms: std::collections::HashSet<String> = query
            .split_whitespace()
            .map(|s| s.to_lowercase())
            .collect();

        if query_terms.is_empty() {
            return vec![];
        }

        let chunks = self.chunks.blocking_read();
        let mut results: Vec<SearchResult> = Vec::new();

        for chunk in chunks.values() {
            let text_lower = chunk.text.to_lowercase();
            let term_count = query_terms.iter().filter(|t| text_lower.contains(t.as_str())).count();
            if term_count > 0 {
                let score = (term_count as f32) / (query_terms.len() as f32);
                results.push(SearchResult::new(chunk, score));
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(top_k);
        results
    }

    /// Remove a document and all its chunks
    pub async fn remove_document(&self, doc_id: &str) -> Result<usize, String> {
        let chunk_ids = {
            let doc_ids = self.doc_ids.read().await;
            doc_ids.get(doc_id).cloned()
        };

        let chunk_ids = chunk_ids.ok_or_else(|| format!("Document {} not found", doc_id))?;

        {
            let mut chunks = self.chunks.write().await;
            for chunk_id in &chunk_ids {
                chunks.remove(chunk_id);
            }
        }

        {
            let mut doc_ids = self.doc_ids.write().await;
            doc_ids.remove(doc_id);
        }

        info!(doc_id = %doc_id, removed = chunk_ids.len(), "Document removed from index");
        Ok(chunk_ids.len())
    }

    /// Get all documents
    pub async fn list_documents(&self) -> Vec<DocumentInfo> {
        let chunks = self.chunks.read().await;
        let doc_ids = self.doc_ids.read().await;

        doc_ids
            .iter()
            .map(|(doc_id, chunk_ids)| {
                let total_chars: usize = chunk_ids
                    .iter()
                    .filter_map(|cid| chunks.get(cid))
                    .map(|c| c.text.len())
                    .sum();

                DocumentInfo {
                    doc_id: doc_id.clone(),
                    num_chunks: chunk_ids.len(),
                    total_chars,
                }
            })
            .collect()
    }

    /// Get store statistics
    pub async fn stats(&self) -> StoreStats {
        let chunks = self.chunks.read().await;
        let doc_ids = self.doc_ids.read().await;

        StoreStats {
            total_chunks: chunks.len(),
            total_documents: doc_ids.len(),
            capacity: self.max_documents,
            embedding_dim: EMBEDDING_DIM,
        }
    }

    /// Clear all documents
    pub async fn clear(&self) {
        let mut chunks = self.chunks.write().await;
        let mut doc_ids = self.doc_ids.write().await;
        chunks.clear();
        doc_ids.clear();
        info!("Vector store cleared");
    }
}

impl Default for VectorStore {
    fn default() -> Self {
        Self {
            chunks: RwLock::new(HashMap::new()),
            doc_ids: RwLock::new(HashMap::new()),
            embedding_service: EmbeddingService::new(None, None),
            max_documents: MAX_STORE_DOCUMENTS,
        }
    }
}

/// Compute cosine similarity between two vectors
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = (norm_a.sqrt() * norm_b.sqrt()).max(1e-8);
    dot / denom
}

/// Result of adding a document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentAddResult {
    pub doc_id: String,
    pub chunks_added: usize,
    pub num_chars: u64,
}

/// Document info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentInfo {
    pub doc_id: String,
    pub num_chunks: usize,
    pub total_chars: usize,
}

/// Store statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreStats {
    pub total_chunks: usize,
    pub total_documents: usize,
    pub capacity: usize,
    pub embedding_dim: usize,
}

// ============================================================================
// Skill Implementation
// ============================================================================

pub struct RAGSkill {
    vector_store: Arc<VectorStore>,
}

impl RAGSkill {
    pub fn new() -> Self {
        let embedding_service = EmbeddingService::new(None, None);
        let vector_store = VectorStore::new(embedding_service, None);
        Self { vector_store }
    }

    pub fn get_store(&self) -> Arc<VectorStore> {
        self.vector_store.clone()
    }
}

impl Default for RAGSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for RAGSkill {
    fn name(&self) -> &'static str {
        "rag"
    }

    fn description(&self) -> &'static str {
        "Retrieval-Augmented Generation: ingest documents into a vector store, \
         search by semantic similarity, and retrieve relevant context. \
         Supports semantic search, keyword search, and hybrid retrieval. \
         Use before LLM inference to inject relevant context from stored documents."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "RAG action",
                    "enum": ["ingest", "ingest_text", "search", "hybrid", "remove", "list", "stats", "clear"]
                },
                "path": {
                    "type": "string",
                    "description": "File path to ingest (for ingest action)"
                },
                "text": {
                    "type": "string",
                    "description": "Text content to ingest (for ingest_text action)"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (for search/hybrid actions)"
                },
                "top_k": {
                    "type": "integer",
                    "description": "Number of results to return",
                    "default": 5
                },
                "chunk_size": {
                    "type": "integer",
                    "description": "Chunk size in characters",
                    "default": 1000
                },
                "overlap": {
                    "type": "integer",
                    "description": "Chunk overlap",
                    "default": 200
                },
                "doc_id": {
                    "type": "string",
                    "description": "Document ID (for remove action)"
                },
                "semantic_weight": {
                    "type": "number",
                    "description": "Weight for semantic vs keyword search (0.0-1.0)",
                    "default": 0.7
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, args: &serde_json::Value) -> SkillOutput {
        use tokio::runtime::Handle;

        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("search").to_string();
        let store = self.vector_store.clone();
        let args_json = serde_json::to_string(args).unwrap_or_default();

        let rt = match Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "rag".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let args_owned: serde_json::Value = serde_json::from_str(&args_json).unwrap_or_default();
                execute_rag_action(&store, &action, &args_owned).await
            })
        })
        .join()
        .unwrap_or_else(|_| Err("RAG operation panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "rag".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "rag".to_string(),
                output: format!("RAG error: {}", e),
                is_error: true,
            },
        }
    }
}

async fn execute_rag_action(
    store: &Arc<VectorStore>,
    action: &str,
    args: &serde_json::Value,
) -> Result<String, String> {
    match action {
        "ingest" => {
            let path = args.get("path").and_then(|v| v.as_str()).ok_or("Path is required for ingest")?;
            let chunk_size = args.get("chunk_size").and_then(|v| v.as_u64()).unwrap_or(1000) as usize;
            let overlap = args.get("overlap").and_then(|v| v.as_u64()).unwrap_or(200) as usize;

            let result = store.add_document(path, chunk_size, overlap).await?;
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "ingested",
                "doc_id": result.doc_id,
                "chunks_added": result.chunks_added,
                "num_chars": result.num_chars
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "ingest_text" => {
            let text = args.get("text").and_then(|v| v.as_str()).ok_or("Text is required for ingest_text")?;
            let source = args.get("source").and_then(|v| v.as_str());
            let chunk_size = args.get("chunk_size").and_then(|v| v.as_u64()).unwrap_or(1000) as usize;
            let overlap = args.get("overlap").and_then(|v| v.as_u64()).unwrap_or(200) as usize;

            let doc_id = store.add_text(text, None, chunk_size, overlap, source).await?;
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "ingested",
                "doc_id": doc_id,
                "text_length": text.len()
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "search" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("Query is required for search")?;
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

            let results = store.search(query, top_k, None).await;
            let result_json: Vec<serde_json::Value> = results.iter().map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "text": r.text,
                    "score": r.score,
                    "doc_id": r.doc_id,
                    "highlight": r.highlight,
                    "metadata": r.metadata
                })
            }).collect();

            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "results": result_json,
                "count": results.len(),
                "query": query
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "hybrid" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("Query is required for hybrid")?;
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
            let weight = args.get("semantic_weight").and_then(|v| v.as_f64()).unwrap_or(0.7) as f32;

            let results = store.hybrid_search(query, top_k, weight).await;
            let result_json: Vec<serde_json::Value> = results.iter().map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "text": r.text,
                    "score": r.score,
                    "doc_id": r.doc_id,
                    "highlight": r.highlight,
                    "metadata": r.metadata
                })
            }).collect();

            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "results": result_json,
                "count": results.len(),
                "query": query
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "remove" => {
            let doc_id = args.get("doc_id").and_then(|v| v.as_str()).ok_or("doc_id is required for remove")?;
            let removed = store.remove_document(doc_id).await?;
            Ok(format!("Removed {} chunks for document {}", removed, doc_id))
        }

        "list" => {
            let docs = store.list_documents().await;
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "documents": docs,
                "count": docs.len()
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "stats" => {
            let stats = store.stats().await;
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "total_chunks": stats.total_chunks,
                "total_documents": stats.total_documents,
                "capacity": stats.capacity,
                "embedding_dim": stats.embedding_dim
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        "clear" => {
            store.clear().await;
            Ok("Vector store cleared".to_string())
        }

        _ => Err(format!(
            "Unknown action: '{}'. Available: ingest, ingest_text, search, hybrid, remove, list, stats, clear",
            action
        )),
    }
}