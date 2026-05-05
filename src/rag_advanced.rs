// ============================================================================
// ULTRACLAW — rag_advanced.rs
// ============================================================================
// Advanced RAG features: HyDE, Cross-Encoder Reranking, and Context Fusion.
//
// This module extends the basic RAG engine with:
// - HyDE (Hypothetical Document Embeddings): generate hypothetical answers
//   and embed them for better retrieval alignment
// - Cross-encoder reranking: re-score retrieved results with a more
//   expensive but accurate cross-encoder model
// - Multi-query search: expand queries to retrieve from different angles
// - Fusion methods: combine results from multiple retrieval strategies
//   (RRF = Reciprocal Rank Fusion, DRR = Distribution-based Rank Fusion)
//
// ARCHITECTURE:
// - HyDESearch: wraps VectorStore, generates hypothetical documents for
//   better query-document alignment
// - Reranker: client for the cross-encoder reranking server
// - AdvancedRAG: orchestrates all advanced retrieval methods
//
// INTEGRATION:
// - Uses VectorStore for initial retrieval
// - Uses EmbeddingService for embedding generation
// - Optionally uses a cross-encoder server for reranking
// - Can be integrated with the LLM for HyDE answer generation
// ============================================================================

use crate::embedding_service::EmbeddingService;
use crate::rag_engine::{VectorStore, SearchResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

const DEFAULT_RRF_K: f32 = 60.0;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HyDEConfig {
    pub enabled: bool,
    pub num_generations: usize,
    pub embed_generations: bool,
}

impl Default for HyDEConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            num_generations: 3,
            embed_generations: true,
        }
    }
}

/// HyDE (Hypothetical Document Embeddings) search
///
/// HyDE improves retrieval by:
/// 1. Generating hypothetical answer documents from the query
/// 2. Embedding both the query AND the hypothetical answers
/// 3. Using the hypothetical embeddings to find similar real documents
///
/// This helps when queries are semantically different from answers but
/// share underlying intent patterns.
pub struct HyDESearch {
    vector_store: Arc<VectorStore>,
    embedding_service: Arc<EmbeddingService>,
    config: HyDEConfig,
}

impl HyDESearch {
    /// Create a new HyDE search instance
    pub fn new(
        vector_store: Arc<VectorStore>,
        embedding_service: Arc<EmbeddingService>,
        config: Option<HyDEConfig>,
    ) -> Self {
        Self {
            vector_store,
            embedding_service,
            config: config.unwrap_or_default(),
        }
    }

    /// Perform HyDE search
    ///
    /// When LLM is available, generates hypothetical answers and uses them
    /// for retrieval. Falls back to standard retrieval when LLM is not
    /// configured or generation fails.
    pub async fn search(
        &self,
        query: &str,
        top_k: usize,
        generate_fn: Option<Box<dyn Fn(&str, usize) -> Pin<Box<dyn Future<Output = Result<Vec<String>, String>> + Send>> + Send + Sync>>,
    ) -> Vec<SearchResult> {
        if !self.config.enabled || generate_fn.is_none() {
            debug!("HyDE disabled or no generator, falling back to standard search");
            return self.vector_store.search(query, top_k, None).await;
        }

        let generate_fn = generate_fn.unwrap();
        match generate_fn(query, self.config.num_generations).await {
            Ok(hypothetical_docs) if !hypothetical_docs.is_empty() => {
                info!(query = %query, num_hypothetical = hypothetical_docs.len(), "HyDE: generated hypothetical answers");
                self.search_with_hypothetical(query, &hypothetical_docs, top_k).await
            }
            Ok(_) => {
                warn!("HyDE: no hypothetical documents generated, using standard search");
                self.vector_store.search(query, top_k, None).await
            }
            Err(e) => {
                warn!(error = %e, "HyDE generation failed, using standard search");
                self.vector_store.search(query, top_k, None).await
            }
        }
    }

    /// Search using hypothetical document embeddings
    async fn search_with_hypothetical(
        &self,
        query: &str,
        hypothetical_docs: &[String],
        top_k: usize,
    ) -> Vec<SearchResult> {
        let mut all_hypothetical_embeddings = Vec::new();

        for doc in hypothetical_docs {
            match self.embedding_service.embed(doc).await {
                Ok(emb) => all_hypothetical_embeddings.push(emb),
                Err(e) => warn!(error = %e, "Failed to embed hypothetical document"),
            }
        }

        if all_hypothetical_embeddings.is_empty() {
            return self.vector_store.search(query, top_k, None).await;
        }

        let query_embedding = match self.embedding_service.embed(query).await {
            Ok(emb) => emb,
            Err(_) => return self.vector_store.search(query, top_k, None).await,
        };

        let mut combined_scores: HashMap<String, (f32, SearchResult)> = HashMap::new();

        for hyp_emb in &all_hypothetical_embeddings {
            let results = self.vector_store.search_by_embedding(hyp_emb, top_k * 2, None).await;
            for result in results {
                let entry = combined_scores.entry(result.id.clone()).or_insert((0.0, result.clone()));
                entry.0 += result.score;

                if result.score > entry.1.score {
                    entry.1 = result;
                }
            }
        }

        let query_results = self.vector_store.search_by_embedding(&query_embedding, top_k, None).await;
        for result in query_results {
            let entry = combined_scores.entry(result.id.clone()).or_insert((0.0, result.clone()));
            entry.0 += result.score * 0.5;

            if result.score > entry.1.score {
                entry.1 = result;
            }
        }

        let mut sorted: Vec<_> = combined_scores.into_values().collect();
        sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(top_k);
        sorted.into_iter().map(|(_, r)| r).collect()
    }

    /// Simple search without HyDE (for testing or fallback)
    pub async fn simple_search(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        self.vector_store.search(query, top_k, None).await
    }
}

impl Default for HyDESearch {
    fn default() -> Self {
        Self::new(
            Arc::new(VectorStore::default()),
            EmbeddingService::new(None, None),
            None,
        )
    }
}

/// Cross-encoder reranker client
///
/// Connects to an external reranking server (typically running
/// scripts/rerank_server.py) to re-score retrieved documents using
/// a cross-encoder model. Cross-encoders consider both query and document
/// together, providing more accurate relevance scores than bi-encoders.
pub struct Reranker {
    rerank_url: Arc<RwLock<Option<String>>>,
    timeout_secs: Arc<RwLock<u64>>,
}

impl Reranker {
    /// Create a new reranker client
    pub fn new() -> Self {
        Self {
            rerank_url: Arc::new(RwLock::new(None)),
            timeout_secs: Arc::new(RwLock::new(30)),
        }
    }

    /// Create with custom configuration
    pub fn with_config(url: Option<&str>, timeout_secs: u64) -> Self {
        let reranker = Self::new();
        if let Some(u) = url {
            reranker.set_url(u);
        }
        reranker.set_timeout(timeout_secs);
        reranker
    }

    /// Set the reranking server URL (async)
    pub async fn set_url_async(&self, url: &str) {
        let url_clean = url.trim().trim_end_matches('/').to_string();
        let mut guard = self.rerank_url.write().await;
        *guard = Some(url_clean.clone());
        info!(url = %url_clean, "Reranker URL configured");
    }

    /// Set the reranking server URL (sync wrapper for skill use)
    pub fn set_url(&self, url: &str) {
        let url_clean = url.trim().trim_end_matches('/').to_string();
        let rerank_url = self.rerank_url.clone();
        let url_clone = url_clean.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let mut guard = rerank_url.write().await;
                *guard = Some(url_clone.clone());
            });
        });

        info!(url = %url_clean, "Reranker URL configured");
    }

    /// Set request timeout in seconds
    pub fn set_timeout(&self, secs: u64) {
        let timeout_secs = self.timeout_secs.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let mut guard = timeout_secs.write().await;
                *guard = secs;
            });
        });
    }

    /// Set request timeout in seconds (async)
    pub async fn set_timeout_async(&self, secs: u64) {
        let mut guard = self.timeout_secs.write().await;
        *guard = secs;
    }

    /// Check if reranker is configured
    pub async fn is_configured(&self) -> bool {
        let guard = self.rerank_url.read().await;
        guard.is_some()
    }

    /// Rerank results using cross-encoder
    ///
    /// Takes a list of retrieved documents and re-scores them based on
    /// their relevance to the query. Returns documents sorted by the
    /// new cross-encoder scores.
    pub async fn rerank(&self, query: &str, candidates: Vec<SearchResult>) -> Vec<SearchResult> {
        let url = {
            let guard = self.rerank_url.read().await;
            guard.clone()
        };

        let url = match url {
            Some(u) => u,
            None => {
                debug!("Reranker not configured, returning candidates as-is");
                return candidates;
            }
        };

        if candidates.is_empty() {
            return candidates;
        }

        let documents: Vec<&str> = candidates.iter().map(|r| r.text.as_str()).collect();

        debug!(query = %query, num_docs = documents.len(), "Reranking documents");

        let timeout = *self.timeout_secs.read().await;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let rerank_url = format!("{}/rerank", url);

        let response = match client
            .post(&rerank_url)
            .json(&serde_json::json!({
                "query": query,
                "documents": documents
            }))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                warn!(error = %e, "Reranking request failed");
                return candidates;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            warn!(status = %status, "Reranking returned non-success status");
            return candidates;
        }

        let scores: Vec<f32> = match response.json().await {
            Ok(serde_json::Value::Object(map)) => {
                map.get("scores")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect()
                    })
                    .unwrap_or_else(|| candidates.iter().map(|r| r.score).collect())
            }
            Ok(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect(),
            _ => {
                warn!("Invalid rerank response format");
                return candidates;
            }
        };

        if scores.len() != candidates.len() {
            warn!(expected = candidates.len(), got = scores.len(), "Score count mismatch");
            return candidates;
        }

        let mut results: Vec<SearchResult> = candidates
            .into_iter()
            .zip(scores.into_iter())
            .map(|(mut r, score)| {
                r.score = score;
                r
            })
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        debug!(results = results.len(), "Reranking complete");
        results
    }

    /// Health check for the reranking server
    pub async fn health_check(&self) -> Result<bool, String> {
        let url = {
            let guard = self.rerank_url.read().await;
            guard.clone()
        };

        let url = match url {
            Some(u) => u,
            None => return Err("Reranker not configured".to_string()),
        };

        let health_url = format!("{}/health", url);
        let client = reqwest::Client::new();

        match client.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => Ok(true),
            Ok(resp) => Err(format!("Health check failed: {}", resp.status())),
            Err(e) => Err(format!("Health check error: {}", e)),
        }
    }
}

impl Default for Reranker {
    fn default() -> Self {
        Self::new()
    }
}

/// Fusion method for combining retrieval results
#[derive(Debug, Clone, Copy)]
pub enum FusionMethod {
    ReciprocalRankFusion,
    DistributionBasedRankFusion,
    ScoreAverage,
}

impl Default for FusionMethod {
    fn default() -> Self {
        Self::ReciprocalRankFusion
    }
}

impl std::fmt::Display for FusionMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FusionMethod::ReciprocalRankFusion => write!(f, "rrf"),
            FusionMethod::DistributionBasedRankFusion => write!(f, "drr"),
            FusionMethod::ScoreAverage => write!(f, "average"),
        }
    }
}

impl std::str::FromStr for FusionMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "rrf" => Ok(FusionMethod::ReciprocalRankFusion),
            "drr" | "distribution" => Ok(FusionMethod::DistributionBasedRankFusion),
            "avg" | "average" => Ok(FusionMethod::ScoreAverage),
            _ => Err(format!("Unknown fusion method: {}", s)),
        }
    }
}

/// Advanced RAG combining HyDE, reranking, and context fusion
pub struct AdvancedRAG {
    vector_store: Arc<VectorStore>,
    embedding_service: Arc<EmbeddingService>,
    hyde: Arc<HyDESearch>,
    reranker: Arc<Reranker>,
    default_top_k: usize,
    default_fusion: FusionMethod,
}

impl AdvancedRAG {
    /// Create a new advanced RAG instance
    pub fn new(vector_store: Arc<VectorStore>) -> Self {
        let embedding_service = EmbeddingService::new(None, None);
        Self {
            hyde: Arc::new(HyDESearch::new(
                vector_store.clone(),
                embedding_service.clone(),
                None,
            )),
            reranker: Arc::new(Reranker::new()),
            vector_store,
            embedding_service,
            default_top_k: 10,
            default_fusion: FusionMethod::ReciprocalRankFusion,
        }
    }

    /// Create with custom configuration
    pub fn with_config(
        vector_store: Arc<VectorStore>,
        embedding_service: Arc<EmbeddingService>,
        hyde_config: HyDEConfig,
        rerank_url: Option<&str>,
    ) -> Self {
        let hyde = Arc::new(HyDESearch::new(
            vector_store.clone(),
            embedding_service.clone(),
            Some(hyde_config),
        ));
        let reranker = Arc::new(Reranker::with_config(rerank_url, 30));

        Self {
            vector_store,
            embedding_service,
            hyde,
            reranker,
            default_top_k: 10,
            default_fusion: FusionMethod::ReciprocalRankFusion,
        }
    }

    /// Set reranking server URL
    pub fn set_reranker_url(&self, url: &str) {
        self.reranker.set_url(url);
    }

    /// Set default top_k for searches
    pub fn set_default_top_k(&mut self, top_k: usize) {
        self.default_top_k = top_k;
    }

    /// Set default fusion method
    pub fn set_default_fusion(&mut self, method: FusionMethod) {
        self.default_fusion = method;
    }

    /// Get the underlying vector store
    pub fn vector_store(&self) -> Arc<VectorStore> {
        self.vector_store.clone()
    }

    /// Multi-query retrieval: expand query and merge results
    ///
    /// Generates multiple query variations and collects results from each,
    /// deduplicating by document ID. This helps capture different aspects
    /// of the user's information need.
    pub async fn multi_query_search(
        &self,
        query: &str,
        num_queries: usize,
        top_k: usize,
    ) -> Vec<SearchResult> {
        let queries = self.expand_query(query, num_queries).await;

        debug!(num_queries = queries.len(), "Multi-query search with expansions");

        let mut all_results: Vec<(SearchResult, usize)> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for (i, q) in queries.iter().enumerate() {
            let results = self.vector_store.search(q, top_k, None).await;
            for r in results {
                if seen.insert(r.id.clone()) {
                    all_results.push((r, i));
                }
            }
        }

        all_results.sort_by(|a, b| b.0.score.partial_cmp(&a.0.score).unwrap_or(std::cmp::Ordering::Equal));
        all_results.into_iter().map(|(r, _)| r).collect()
    }

    /// Query expansion using simple transformations
    ///
    /// Generates query variations based on common transformations:
    /// - Original query
    /// - What/how question form
    /// - Information-seeking form
    ///
    /// For more sophisticated expansion, integrate with LLM.
    async fn expand_query(&self, query: &str, num_queries: usize) -> Vec<String> {
        let mut queries = vec![query.to_string()];

        let words: Vec<&str> = query.split_whitespace().collect();
        if !words.is_empty() {
            queries.push(format!("What is {}?", words[0]));
            queries.push(format!("Information about {}", query));
        }

        if num_queries > 3 && query.len() > 10 {
            queries.push(format!("Explain {}", query));
            queries.push(format!("How to {}", query));
        }

        queries.truncate(num_queries);
        queries
    }

    /// Context fusion: combine retrieved contexts using specified method
    pub async fn fusion_search(
        &self,
        query: &str,
        top_k: usize,
        fusion_method: FusionMethod,
    ) -> Vec<SearchResult> {
        match fusion_method {
            FusionMethod::ReciprocalRankFusion => self.reciprocal_rank_fusion(query, top_k).await,
            FusionMethod::DistributionBasedRankFusion => {
                self.distribution_based_rank_fusion(query, top_k).await
            }
            FusionMethod::ScoreAverage => self.score_average_fusion(query, top_k).await,
        }
    }

    /// Reciprocal Rank Fusion (RRF)
    ///
    /// Combines rankings from multiple queries using the formula:
    /// RRF_score(d) = Σ(1 / (k + rank(d)))
    ///
    /// where k is a constant (typically 60) and rank(d) is the position
    /// of document d in each ranking.
    async fn reciprocal_rank_fusion(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        let k = DEFAULT_RRF_K;
        let queries = self.expand_query(query, 3).await;

        let mut fused_scores: HashMap<String, (f32, SearchResult)> = HashMap::new();

        for q in &queries {
            let results = self.vector_store.search(q, top_k * 2, Some(0.1)).await;
            for (rank, result) in results.iter().enumerate() {
                let rrf_score = 1.0 / (k + (rank + 1) as f32);
                let entry = fused_scores.entry(result.id.clone()).or_insert((0.0, result.clone()));
                entry.0 += rrf_score;
                if result.score > entry.1.score {
                    entry.1 = result.clone();
                }
            }
        }

        let mut sorted: Vec<_> = fused_scores.into_values().collect();
        sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(top_k);
        sorted.into_iter().map(|(_, r)| r).collect()
    }

    /// Distribution-based Rank Fusion (DRR)
    ///
    /// Normalizes scores within each ranking and averages across rankings,
    /// providing more robust fusion when score distributions vary.
    async fn distribution_based_rank_fusion(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        let queries = self.expand_query(query, 3).await;

        let mut all_scores: HashMap<String, Vec<f32>> = HashMap::new();
        let mut all_results: HashMap<String, SearchResult> = HashMap::new();

        for q in &queries {
            let results = self.vector_store.search(q, top_k * 2, Some(0.1)).await;

            if let Some(first) = results.first() {
                let max_score = first.score.max(1e-8);

                for result in results {
                    all_scores.entry(result.id.clone()).or_default().push(result.score / max_score);
                    if !all_results.contains_key(&result.id) {
                        all_results.insert(result.id.clone(), result);
                    }
                }
            }
        }

        let mut fused: Vec<(String, f32)> = all_scores
            .into_iter()
            .map(|(id, scores)| {
                let avg = scores.iter().sum::<f32>() / scores.len() as f32;
                (id, avg)
            })
            .collect();

        fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut results = Vec::new();
        for (id, score) in fused.into_iter().take(top_k) {
            if let Some(mut r) = all_results.get(&id).cloned() {
                r.score = score;
                results.push(r);
            }
        }
        results
    }

    /// Simple score averaging fusion
    async fn score_average_fusion(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        let queries = self.expand_query(query, 3).await;

        let mut score_sums: HashMap<String, (f32, SearchResult)> = HashMap::new();

        for q in &queries {
            let results = self.vector_store.search(q, top_k * 2, Some(0.1)).await;
            for result in results {
                let entry = score_sums.entry(result.id.clone()).or_insert((0.0, result.clone()));
                entry.0 += result.score;
                if result.score > entry.1.score {
                    entry.1 = result;
                }
            }
        }

        let count = queries.len() as f32;
        let mut averaged: Vec<_> = score_sums
            .into_iter()
            .map(|(id, (score_sum, mut r))| {
                r.score = score_sum / count;
                (id, r)
            })
            .collect();

        averaged.sort_by(|a, b| b.1.score.partial_cmp(&a.1.score).unwrap_or(std::cmp::Ordering::Equal));
        averaged.truncate(top_k);
        averaged.into_iter().map(|(_, r)| r).collect()
    }

    /// Complete advanced search with HyDE, multi-query, fusion, and reranking
    ///
    /// This is the main entry point for advanced retrieval, combining all
    /// optimization techniques in a single call.
    pub async fn advanced_search(
        &self,
        query: &str,
        top_k: usize,
        use_hyde: bool,
        use_fusion: bool,
        use_rerank: bool,
        generate_fn: Option<Box<dyn Fn(&str, usize) -> Pin<Box<dyn Future<Output = Result<Vec<String>, String>> + Send>> + Send + Sync>>,
    ) -> Vec<SearchResult> {
        let results = if use_fusion {
            self.fusion_search(query, top_k * 2, self.default_fusion).await
        } else if use_hyde {
            self.hyde.search(query, top_k * 2, generate_fn).await
        } else {
            self.multi_query_search(query, 3, top_k * 2).await
        };

        if use_rerank {
            self.reranker.rerank(query, results).await
        } else {
            results
        }
    }

    /// Search with all advanced features enabled
    pub async fn search(&self, query: &str, top_k: usize) -> Vec<SearchResult> {
        self.advanced_search(query, top_k, true, true, true, None).await
    }
}

impl Default for AdvancedRAG {
    fn default() -> Self {
        Self::new(Arc::new(VectorStore::default()))
    }
}

// Add the missing import for Pin and Future
use std::future::Future;
use std::pin::Pin;

// ============================================================================
// Skill Implementation
// ============================================================================

use crate::skill::{Skill, SkillOutput};

pub struct AdvancedRAGSkill {
    rag: Arc<RwLock<AdvancedRAG>>,
    vector_store: Arc<VectorStore>,
}

impl AdvancedRAGSkill {
    /// Create a new advanced RAG skill
    pub fn new() -> Self {
        let vector_store = Arc::new(VectorStore::default());
        let rag = AdvancedRAG::new(vector_store.clone());

        Self {
            rag: Arc::new(RwLock::new(rag)),
            vector_store,
        }
    }

    /// Create with custom vector store
    pub fn with_store(vector_store: Arc<VectorStore>) -> Self {
        let rag = AdvancedRAG::new(vector_store.clone());

        Self {
            rag: Arc::new(RwLock::new(rag)),
            vector_store,
        }
    }

    /// Configure the reranking server URL
    pub async fn set_reranker_url(&self, url: &str) {
        let rag = self.rag.read().await;
        rag.set_reranker_url(url);
    }

    /// Get the underlying vector store for integration
    pub fn get_vector_store(&self) -> Arc<VectorStore> {
        self.vector_store.clone()
    }
}

impl Default for AdvancedRAGSkill {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for AdvancedRAGSkill {
    fn name(&self) -> &'static str {
        "advanced_rag"
    }

    fn description(&self) -> &'static str {
        "Advanced RAG with HyDE (Hypothetical Document Embeddings), cross-encoder \
         reranking, multi-query search, and fusion methods (RRF, DRR). Use this \
         for higher-quality retrieval when standard RAG is insufficient. Configure \
         reranker with config_reranker action. Supported fusion: rrf (Reciprocal \
         Rank Fusion), drr (Distribution-based), average."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Advanced RAG action",
                    "enum": ["search", "multi_query", "fusion", "hyde", "config_reranker", "rerank", "stats"]
                },
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "top_k": {
                    "type": "integer",
                    "description": "Number of results to return",
                    "default": 10
                },
                "use_hyde": {
                    "type": "boolean",
                    "description": "Enable HyDE (Hypothetical Document Embeddings)",
                    "default": true
                },
                "use_fusion": {
                    "type": "boolean",
                    "description": "Enable fusion search (multi-query with rank fusion)",
                    "default": true
                },
                "use_rerank": {
                    "type": "boolean",
                    "description": "Enable cross-encoder reranking",
                    "default": true
                },
                "fusion_method": {
                    "type": "string",
                    "description": "Fusion method",
                    "enum": ["rrf", "drr", "average"],
                    "default": "rrf"
                },
                "num_queries": {
                    "type": "integer",
                    "description": "Number of query variations for multi-query search",
                    "default": 3
                },
                "reranker_url": {
                    "type": "string",
                    "description": "URL for the cross-encoder reranking server"
                },
                "documents": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Documents to rerank (for rerank action)"
                }
            },
            "required": ["action"]
        })
    }

    fn execute_sync(&self, args: &serde_json::Value) -> SkillOutput {
        use tokio::runtime::Handle;

        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("search").to_string();
        let rag = self.rag.clone();
        let args_json = serde_json::to_string(args).unwrap_or_default();

        let rt = match Handle::try_current() {
            Ok(rt) => rt,
            Err(_) => {
                return SkillOutput {
                    name: "advanced_rag".to_string(),
                    output: "Error: no async runtime available".to_string(),
                    is_error: true,
                };
            }
        };

        let result = std::thread::spawn(move || {
            rt.block_on(async {
                let args_owned: serde_json::Value = serde_json::from_str(&args_json).unwrap_or_default();
                execute_advanced_rag_action(&rag, &action, &args_owned).await
            })
        })
        .join()
        .unwrap_or_else(|_| Err("Advanced RAG operation panicked".to_string()));

        match result {
            Ok(output) => SkillOutput {
                name: "advanced_rag".to_string(),
                output,
                is_error: false,
            },
            Err(e) => SkillOutput {
                name: "advanced_rag".to_string(),
                output: format!("Advanced RAG error: {}", e),
                is_error: true,
            },
        }
    }
}

async fn execute_advanced_rag_action(
    rag: &Arc<RwLock<AdvancedRAG>>,
    action: &str,
    args: &serde_json::Value,
) -> Result<String, String> {
    match action {
        "search" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("Query required")?;
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let use_hyde = args.get("use_hyde").and_then(|v| v.as_bool()).unwrap_or(true);
            let use_fusion = args.get("use_fusion").and_then(|v| v.as_bool()).unwrap_or(true);
            let use_rerank = args.get("use_rerank").and_then(|v| v.as_bool()).unwrap_or(true);

            let rag_guard = rag.read().await;
            let results = rag_guard
                .advanced_search(query, top_k, use_hyde, use_fusion, use_rerank, None)
                .await;
            drop(rag_guard);

            Ok(format_search_results(query, &results))
        }

        "multi_query" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("Query required")?;
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let num_queries = args.get("num_queries").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

            let rag_guard = rag.read().await;
            let results = rag_guard.multi_query_search(query, num_queries, top_k).await;
            drop(rag_guard);

            Ok(format_search_results(query, &results))
        }

        "fusion" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("Query required")?;
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let method_str = args
                .get("fusion_method")
                .and_then(|v| v.as_str())
                .unwrap_or("rrf");
            let fusion_method: FusionMethod = method_str.parse().unwrap_or(FusionMethod::ReciprocalRankFusion);

            let rag_guard = rag.read().await;
            let results = rag_guard.fusion_search(query, top_k, fusion_method).await;
            drop(rag_guard);

            Ok(format_search_results(query, &results))
        }

        "hyde" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("Query required")?;
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let rag_guard = rag.read().await;
            let results = rag_guard.hyde.search(query, top_k, None).await;
            drop(rag_guard);

            Ok(format_search_results(query, &results))
        }

        "config_reranker" => {
            let url = args.get("reranker_url").and_then(|v| v.as_str()).ok_or("reranker_url required")?;

            let rag_guard = rag.read().await;
            rag_guard.set_reranker_url(url);
            drop(rag_guard);

            Ok(format!("Reranker URL configured: {}", url))
        }

        "rerank" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("Query required")?;
            let docs_array = args.get("documents").and_then(|v| v.as_array());

            let docs: Vec<String> = match docs_array {
                Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                None => return Err("documents array required".to_string()),
            };

            if docs.is_empty() {
                return Ok("No documents to rerank".to_string());
            }

            let candidates: Vec<SearchResult> = docs
                .iter()
                .enumerate()
                .map(|(i, text)| SearchResult {
                    id: format!("doc_{}", i),
                    text: text.clone(),
                    score: 0.5,
                    doc_id: format!("doc_{}", i),
                    chunk_index: 0,
                    metadata: crate::rag_engine::ChunkMetadata::default(),
                    highlight: None,
                })
                .collect();

            let rag_guard = rag.read().await;
            let reranker = rag_guard.reranker.clone();
            drop(rag_guard);

            let results = reranker.rerank(query, candidates).await;

            Ok(format_search_results(query, &results))
        }

        "stats" => {
            let rag_guard = rag.read().await;
            let store_stats = rag_guard.vector_store.stats().await;
            let rerank_configured = rag_guard.reranker.is_configured().await;

            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "vector_store": {
                    "total_chunks": store_stats.total_chunks,
                    "total_documents": store_stats.total_documents,
                    "capacity": store_stats.capacity,
                    "embedding_dim": store_stats.embedding_dim
                },
                "reranker_configured": rerank_configured,
                "default_top_k": rag_guard.default_top_k,
                "default_fusion": rag_guard.default_fusion.to_string()
            })).unwrap_or_else(|_| "{}".to_string()))
        }

        _ => Err(format!(
            "Unknown action: '{}'. Available: search, multi_query, fusion, hyde, config_reranker, rerank, stats",
            action
        )),
    }
}

fn format_search_results(query: &str, results: &[SearchResult]) -> String {
    if results.is_empty() {
        return serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "results": [],
            "count": 0,
            "message": "No results found"
        })).unwrap_or_else(|_| "{}".to_string());
    }

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

    serde_json::to_string_pretty(&serde_json::json!({
        "query": query,
        "results": result_json,
        "count": results.len()
    })).unwrap_or_else(|_| "{}".to_string())
}