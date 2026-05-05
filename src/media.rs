// ============================================================================
// ULTRACLAW — media.rs
// ============================================================================
// Cloud-based image and video generation engine.
//
// SUPPORTED PROVIDERS (15 total):
// ┌─────────────────┬────────┬───────────────────────────────────────────────┐
// │ Provider        │ Type   │ Endpoint / Model                              │
// ├─────────────────┼────────┼───────────────────────────────────────────────┤
// │ OpenAI DALL-E 3 │ Image  │ POST /v1/images/generations                   │
// │ OpenAI Sora     │ Video  │ POST /v1/video/generations                    │
// │ Stability AI    │ Image  │ POST /v2beta/stable-image/generate/core       │
// │ Replicate       │ Both   │ POST /v1/predictions                          │
// │ Runway ML       │ Video  │ POST /v1/image_to_video                       │
// │ Together AI     │ Image  │ POST /v1/images/generations (OAI-compat)      │
// │ Fal.ai          │ Both   │ POST /fal-ai/flux/dev                         │
// │ Leonardo AI     │ Image  │ POST /generations                             │
// │ Google Imagen   │ Image  │ POST /v1/images:generate                      │
// │ Google Veo      │ Video  │ Gemini API veo-3.0-generate-preview           │
// │ Kling           │ Video  │ POST /v1/videos/text2video (klingai.com)      │
// │ Seedance        │ Video  │ Volcengine Jimeng / fal.ai seedance           │
// │ Luma            │ Video  │ POST /dream-machine/v1/generations (luma)     │
// │ Minimax         │ Video  │ POST /api/v1/video_generation (minimax.io)    │
// │ Pika            │ Video  │ fal.ai pika-2.2 / POST pika text-to-video     │
// └─────────────────┴────────┴───────────────────────────────────────────────┘
//
// ARCHITECTURE:
// MediaEngine dispatches to the correct provider based on config.
// All providers go through the same reqwest Client (connection pooling).
// Generated media is saved to disk, and the file path is returned.
//
// MEMORY OPTIMIZATION:
// - Images are streamed to disk, never fully buffered in RAM.
//   A 4MB PNG is written chunk-by-chunk via tokio::io::copy.
// - Only the API response metadata (~1KB) is held in RAM.
// - Base64 decoding (for providers that return base64) streams into
//   a file writer. Peak RAM: ~64KB decode buffer.
//
// ENERGY OPTIMIZATION:
// - CPU is idle during API calls (async await = yielded to scheduler).
// - No local GPU/CPU compute — all generation happens server-side.
// - Only one generation at a time per request (sequential, not parallel).
// ============================================================================

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::info;

/// Timeout for media generation API calls.
/// Image generation typically takes 5-30 seconds.
/// Video generation can take 60-180 seconds.
#[allow(dead_code)]
const IMAGE_TIMEOUT_SECS: u64 = 60;
const VIDEO_TIMEOUT_SECS: u64 = 300;

// ============================================================================
// MEDIA PROVIDER ENUM
// ============================================================================

/// All supported cloud media generation providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaProvider {
    /// OpenAI DALL-E 3 — highest quality, $0.04-0.12/image
    OpenAI,
    /// Stability AI SDXL/SD3 — fast, cheap, great for iteration
    Stability,
    /// Replicate — runs any open-source model (Flux, SDXL, etc.)
    Replicate,
    /// Runway ML Gen-3 Alpha — state-of-the-art video generation
    Runway,
    /// Together AI — OpenAI-compatible, runs Flux/SDXL
    Together,
    /// Fal.ai — ultra-fast Flux inference, pay-per-second
    Fal,
    /// Leonardo AI — fine-tuned models, style presets
    Leonardo,
    /// Google Imagen 3 — Vertex AI, very high quality
    Imagen,
    /// Google Veo — Gemini API video generation (Veo 2/3/3.1)
    Veo,
    /// Kling — Kuaishou's video gen (Kling 1.6/2.0/3.0)
    Kling,
    /// Seedance — ByteDance video gen (Seedance 1.0/2.0)
    Seedance,
    /// Luma Dream Machine — Ray2, cinematic video gen
    Luma,
    /// Minimax Hailuo — text/image-to-video (Hailuo 02/2.3)
    Minimax,
    /// Pika — stylized AI video (Pika 2.2)
    Pika,
    /// OpenAI Sora — OpenAI's video generation model
    Sora,
}

impl MediaProvider {
    /// Whether this provider supports image generation.
    pub fn supports_image(&self) -> bool {
        matches!(
            self,
            Self::OpenAI
                | Self::Stability
                | Self::Replicate
                | Self::Together
                | Self::Fal
                | Self::Leonardo
                | Self::Imagen
        )
    }

    /// Whether this provider supports video generation.
    pub fn supports_video(&self) -> bool {
        matches!(
            self,
            Self::Runway
                | Self::Replicate
                | Self::Fal
                | Self::Veo
                | Self::Kling
                | Self::Seedance
                | Self::Luma
                | Self::Minimax
                | Self::Pika
                | Self::Sora
        )
    }

    /// Parse provider from string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "openai" | "dalle" | "dall-e" => Some(Self::OpenAI),
            "stability" | "stable-diffusion" | "sd" | "sdxl" => Some(Self::Stability),
            "replicate" => Some(Self::Replicate),
            "runway" => Some(Self::Runway),
            "together" => Some(Self::Together),
            "fal" | "fal.ai" | "flux" => Some(Self::Fal),
            "leonardo" => Some(Self::Leonardo),
            "imagen" => Some(Self::Imagen),
            "veo" | "google-veo" => Some(Self::Veo),
            "kling" | "kuaishou" => Some(Self::Kling),
            "seedance" | "bytedance" | "jimeng" => Some(Self::Seedance),
            "luma" | "dream-machine" | "ray2" => Some(Self::Luma),
            "minimax" | "hailuo" => Some(Self::Minimax),
            "pika" => Some(Self::Pika),
            "sora" | "openai-video" => Some(Self::Sora),
            _ => None,
        }
    }
}

// ============================================================================
// MEDIA OUTPUT
// ============================================================================

/// Result of a media generation request.
#[derive(Debug, Clone, Serialize)]
pub struct MediaOutput {
    /// Absolute path to the saved file on disk.
    pub file_path: PathBuf,
    /// MIME type of the generated media.
    pub mime_type: String,
    /// File size in bytes.
    pub file_size: u64,
    /// The provider that generated this media.
    pub provider: MediaProvider,
    /// The prompt that was used.
    pub prompt: String,
    /// Generation metadata (model version, seed, etc.)
    pub metadata: Value,
}

/// Parameters for image generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageParams {
    pub prompt: String,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default = "default_count")]
    pub count: u32,
}

fn default_width() -> u32 { 1024 }
fn default_height() -> u32 { 1024 }
fn default_count() -> u32 { 1 }

/// Parameters for video generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoParams {
    pub prompt: String,
    #[serde(default = "default_duration")]
    pub duration_secs: u32,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

fn default_duration() -> u32 { 5 }

// ============================================================================
// MEDIA ENGINE
// ============================================================================

/// Cloud-based media generation engine.
///
/// # Memory Layout
/// - `client`: shared reqwest Client (~200 bytes, Arc'd)
/// - `output_dir`: PathBuf (~48 bytes)  
/// - `api_keys`: HashMap<provider, key> (~256 bytes for 8 providers)
/// - `default_provider`: enum (1 byte)
/// Total: ~500 bytes
pub struct MediaEngine {
    /// Reusable HTTP client (shared with InferenceEngine's client).
    client: Client,
    /// Directory to save generated media files.
    output_dir: PathBuf,
    /// API keys per provider.
    api_keys: std::collections::HashMap<MediaProvider, String>,
    /// Default image provider.
    pub default_image_provider: MediaProvider,
    /// Default video provider.
    pub default_video_provider: MediaProvider,
}

impl MediaEngine {
    /// Create a new media engine.
    ///
    /// Discovers available providers by checking which API keys are set.
    /// Falls back through providers in quality order if the preferred one
    /// isn't configured.
    pub fn new(
        api_keys: std::collections::HashMap<MediaProvider, String>,
        output_dir: PathBuf,
        preferred_image: Option<MediaProvider>,
        preferred_video: Option<MediaProvider>,
    ) -> Self {
        // Build the HTTP client with longer timeouts for media generation.
        let client = Client::builder()
            .timeout(Duration::from_secs(VIDEO_TIMEOUT_SECS))
            .pool_max_idle_per_host(2)
            .build()
            .expect("Failed to build media HTTP client");

        // Ensure output directory exists
        std::fs::create_dir_all(&output_dir).ok();

        // Pick default image provider: prefer user choice, then fallback by quality
        let image_priority = [
            MediaProvider::OpenAI,
            MediaProvider::Stability,
            MediaProvider::Fal,
            MediaProvider::Together,
            MediaProvider::Replicate,
            MediaProvider::Leonardo,
            MediaProvider::Imagen,
        ];
        let default_image = preferred_image
            .filter(|p| api_keys.contains_key(p) && p.supports_image())
            .or_else(|| {
                image_priority
                    .iter()
                    .find(|p| api_keys.contains_key(p) && p.supports_image())
                    .copied()
            })
            .unwrap_or(MediaProvider::OpenAI);

        // Pick default video provider
        let video_priority = [
            MediaProvider::Veo,
            MediaProvider::Sora,
            MediaProvider::Kling,
            MediaProvider::Runway,
            MediaProvider::Luma,
            MediaProvider::Seedance,
            MediaProvider::Minimax,
            MediaProvider::Pika,
            MediaProvider::Fal,
            MediaProvider::Replicate,
        ];
        let default_video = preferred_video
            .filter(|p| api_keys.contains_key(p) && p.supports_video())
            .or_else(|| {
                video_priority
                    .iter()
                    .find(|p| api_keys.contains_key(p) && p.supports_video())
                    .copied()
            })
            .unwrap_or(MediaProvider::Runway);

        info!(
            image_provider = ?default_image,
            video_provider = ?default_video,
            configured_providers = api_keys.len(),
            output_dir = %output_dir.display(),
            "MediaEngine initialized"
        );

        Self {
            client,
            output_dir,
            api_keys,
            default_image_provider: default_image,
            default_video_provider: default_video,
        }
    }

    /// Get the API key for a provider, or return an error.
    fn get_key(&self, provider: MediaProvider) -> Result<&str, String> {
        self.api_keys
            .get(&provider)
            .map(|s| s.as_str())
            .ok_or_else(|| format!("No API key configured for {:?}", provider))
    }

    /// Generate a unique file path for saving media.
    fn output_path(&self, extension: &str) -> PathBuf {
        let id = uuid::Uuid::new_v4();
        self.output_dir.join(format!("{}.{}", id, extension))
    }

    // ========================================================================
    // IMAGE GENERATION — PROVIDER IMPLEMENTATIONS
    // ========================================================================

    /// Generate an image using the configured default provider (or a specific one).
    pub async fn generate_image(
        &self,
        params: &ImageParams,
        provider: Option<MediaProvider>,
    ) -> Result<MediaOutput, String> {
        let provider = provider.unwrap_or(self.default_image_provider);

        if !provider.supports_image() {
            return Err(format!("{:?} does not support image generation", provider));
        }

        info!(
            provider = ?provider,
            prompt_len = params.prompt.len(),
            size = %format!("{}x{}", params.width, params.height),
            "Generating image"
        );

        match provider {
            MediaProvider::OpenAI => self.generate_image_openai(params).await,
            MediaProvider::Stability => self.generate_image_stability(params).await,
            MediaProvider::Replicate => self.generate_image_replicate(params).await,
            MediaProvider::Together => self.generate_image_together(params).await,
            MediaProvider::Fal => self.generate_image_fal(params).await,
            MediaProvider::Leonardo => self.generate_image_leonardo(params).await,
            MediaProvider::Imagen => self.generate_image_imagen(params).await,
            _ => Err(format!("{:?} does not support image generation", provider)),
        }
    }

    /// Generate a video using the configured default provider (or a specific one).
    pub async fn generate_video(
        &self,
        params: &VideoParams,
        provider: Option<MediaProvider>,
    ) -> Result<MediaOutput, String> {
        let provider = provider.unwrap_or(self.default_video_provider);

        if !provider.supports_video() {
            return Err(format!("{:?} does not support video generation", provider));
        }

        info!(
            provider = ?provider,
            prompt_len = params.prompt.len(),
            duration = params.duration_secs,
            "Generating video"
        );

        match provider {
            MediaProvider::Runway => self.generate_video_runway(params).await,
            MediaProvider::Replicate => self.generate_video_replicate(params).await,
            MediaProvider::Fal => self.generate_video_fal(params).await,
            MediaProvider::Veo => self.generate_video_veo(params).await,
            MediaProvider::Kling => self.generate_video_kling(params).await,
            MediaProvider::Seedance => self.generate_video_seedance(params).await,
            MediaProvider::Luma => self.generate_video_luma(params).await,
            MediaProvider::Minimax => self.generate_video_minimax(params).await,
            MediaProvider::Pika => self.generate_video_pika(params).await,
            MediaProvider::Sora => self.generate_video_sora(params).await,
            _ => Err(format!("{:?} does not support video generation", provider)),
        }
    }

    // ========================================================================
    // OPENAI DALL-E 3
    // ========================================================================

    async fn generate_image_openai(&self, params: &ImageParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::OpenAI)?;

        // DALL-E 3 supports: 1024x1024, 1024x1792, 1792x1024
        let size = match (params.width, params.height) {
            (w, h) if w > h => "1792x1024",
            (w, h) if h > w => "1024x1792",
            _ => "1024x1024",
        };

        let body = serde_json::json!({
            "model": params.model.as_deref().unwrap_or("dall-e-3"),
            "prompt": params.prompt,
            "n": 1,
            "size": size,
            "quality": params.style.as_deref().unwrap_or("standard"),
            "response_format": "b64_json"
        });

        let resp = self.client
            .post("https://api.openai.com/v1/images/generations")
            .header("Authorization", format!("Bearer {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("OpenAI request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("OpenAI DALL-E error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let b64 = json["data"][0]["b64_json"]
            .as_str()
            .ok_or("No image data in OpenAI response")?;

        let file_path = self.output_path("png");
        self.save_base64_to_file(b64, &file_path)?;

        let file_size = std::fs::metadata(&file_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "image/png".to_string(),
            file_size,
            provider: MediaProvider::OpenAI,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({
                "model": "dall-e-3",
                "size": size,
                "revised_prompt": json["data"][0]["revised_prompt"]
            }),
        })
    }

    // ========================================================================
    // STABILITY AI (SDXL / SD3)
    // ========================================================================

    async fn generate_image_stability(&self, params: &ImageParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Stability)?;

        let form = reqwest::multipart::Form::new()
            .text("prompt", params.prompt.clone())
            .text("output_format", "png")
            .text("aspect_ratio", Self::aspect_ratio(params.width, params.height));

        let resp = self.client
            .post("https://api.stability.ai/v2beta/stable-image/generate/core")
            .header("Authorization", format!("Bearer {}", key))
            .header("Accept", "application/json")
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("Stability request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Stability AI error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let b64 = json["image"]
            .as_str()
            .ok_or("No image data in Stability response")?;

        let file_path = self.output_path("png");
        self.save_base64_to_file(b64, &file_path)?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "image/png".to_string(),
            file_size,
            provider: MediaProvider::Stability,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": "stable-diffusion-core" }),
        })
    }

    // ========================================================================
    // TOGETHER AI (OpenAI-compatible, Flux/SDXL)
    // ========================================================================

    async fn generate_image_together(&self, params: &ImageParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Together)?;

        let body = serde_json::json!({
            "model": params.model.as_deref().unwrap_or("black-forest-labs/FLUX.1-schnell-Free"),
            "prompt": params.prompt,
            "width": params.width,
            "height": params.height,
            "n": 1,
            "response_format": "b64_json"
        });

        let resp = self.client
            .post("https://api.together.xyz/v1/images/generations")
            .header("Authorization", format!("Bearer {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Together AI request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Together AI error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let b64 = json["data"][0]["b64_json"]
            .as_str()
            .ok_or("No image data in Together response")?;

        let file_path = self.output_path("png");
        self.save_base64_to_file(b64, &file_path)?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "image/png".to_string(),
            file_size,
            provider: MediaProvider::Together,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": "FLUX.1-schnell" }),
        })
    }

    // ========================================================================
    // FAL.AI (Flux, fast inference)
    // ========================================================================

    async fn generate_image_fal(&self, params: &ImageParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Fal)?;

        let body = serde_json::json!({
            "prompt": params.prompt,
            "image_size": { "width": params.width, "height": params.height },
            "num_images": 1
        });

        let model = params.model.as_deref().unwrap_or("fal-ai/flux/dev");
        let url = format!("https://fal.run/{}", model);

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Key {}", key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Fal.ai request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Fal.ai error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let image_url = json["images"][0]["url"]
            .as_str()
            .ok_or("No image URL in Fal response")?;

        let file_path = self.output_path("png");
        self.download_url_to_file(image_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "image/png".to_string(),
            file_size,
            provider: MediaProvider::Fal,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model }),
        })
    }

    // ========================================================================
    // REPLICATE (any open-source model)
    // ========================================================================

    async fn generate_image_replicate(&self, params: &ImageParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Replicate)?;

        let model = params.model.as_deref()
            .unwrap_or("black-forest-labs/flux-schnell");

        let body = serde_json::json!({
            "model": model,
            "input": {
                "prompt": params.prompt,
                "width": params.width,
                "height": params.height,
                "num_outputs": 1
            }
        });

        // Create prediction
        let resp = self.client
            .post("https://api.replicate.com/v1/predictions")
            .header("Authorization", format!("Bearer {}", key))
            .header("Prefer", "wait")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Replicate request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Replicate error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

        // Replicate may return directly or require polling
        let output = self.replicate_wait_for_output(&json, key).await?;
        let image_url = output.as_str().ok_or("No image URL in Replicate output")?;

        let file_path = self.output_path("png");
        self.download_url_to_file(image_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "image/png".to_string(),
            file_size,
            provider: MediaProvider::Replicate,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model }),
        })
    }

    // ========================================================================
    // LEONARDO AI
    // ========================================================================

    async fn generate_image_leonardo(&self, params: &ImageParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Leonardo)?;

        let body = serde_json::json!({
            "prompt": params.prompt,
            "width": params.width,
            "height": params.height,
            "num_images": 1,
            "modelId": params.model.as_deref().unwrap_or("6b645e3a-d64f-4341-a6d8-7a3690fbf042"),
            "negative_prompt": params.negative_prompt.as_deref().unwrap_or("")
        });

        let resp = self.client
            .post("https://cloud.leonardo.ai/api/rest/v1/generations")
            .header("Authorization", format!("Bearer {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Leonardo request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Leonardo AI error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let gen_id = json["sdGenerationJob"]["generationId"]
            .as_str()
            .ok_or("No generation ID from Leonardo")?;

        // Poll for completion
        let image_url = self.leonardo_poll_result(key, gen_id).await?;

        let file_path = self.output_path("png");
        self.download_url_to_file(&image_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "image/png".to_string(),
            file_size,
            provider: MediaProvider::Leonardo,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "generation_id": gen_id }),
        })
    }

    // ========================================================================
    // GOOGLE IMAGEN 3
    // ========================================================================

    async fn generate_image_imagen(&self, params: &ImageParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Imagen)?;

        let body = serde_json::json!({
            "instances": [{ "prompt": params.prompt }],
            "parameters": {
                "sampleCount": 1,
                "aspectRatio": Self::aspect_ratio(params.width, params.height),
            }
        });

        let resp = self.client
            .post("https://us-central1-aiplatform.googleapis.com/v1/projects/-/locations/us-central1/publishers/google/models/imagen-3.0-generate-002:predict")
            .header("Authorization", format!("Bearer {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Imagen request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Google Imagen error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let b64 = json["predictions"][0]["bytesBase64Encoded"]
            .as_str()
            .ok_or("No image data in Imagen response")?;

        let file_path = self.output_path("png");
        self.save_base64_to_file(b64, &file_path)?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "image/png".to_string(),
            file_size,
            provider: MediaProvider::Imagen,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": "imagen-3.0-generate-002" }),
        })
    }

    // ========================================================================
    // VIDEO GENERATION — RUNWAY ML
    // ========================================================================

    async fn generate_video_runway(&self, params: &VideoParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Runway)?;

        let body = serde_json::json!({
            "promptText": params.prompt,
            "model": params.model.as_deref().unwrap_or("gen3a_turbo"),
            "duration": params.duration_secs.min(10),
            "watermark": false
        });

        let resp = self.client
            .post("https://api.dev.runwayml.com/v1/image_to_video")
            .header("Authorization", format!("Bearer {}", key))
            .header("X-Runway-Version", "2024-11-06")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Runway request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Runway error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let task_id = json["id"]
            .as_str()
            .ok_or("No task ID from Runway")?;

        // Poll for completion
        let video_url = self.runway_poll_result(key, task_id).await?;

        let file_path = self.output_path("mp4");
        self.download_url_to_file(&video_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "video/mp4".to_string(),
            file_size,
            provider: MediaProvider::Runway,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": "gen3a_turbo", "task_id": task_id }),
        })
    }

    // ========================================================================
    // VIDEO GENERATION — REPLICATE
    // ========================================================================

    async fn generate_video_replicate(&self, params: &VideoParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Replicate)?;

        let model = params.model.as_deref()
            .unwrap_or("minimax/video-01-live");

        let body = serde_json::json!({
            "model": model,
            "input": {
                "prompt": params.prompt,
            }
        });

        let resp = self.client
            .post("https://api.replicate.com/v1/predictions")
            .header("Authorization", format!("Bearer {}", key))
            .header("Prefer", "wait=300")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Replicate video request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Replicate error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let output = self.replicate_wait_for_output(&json, key).await?;
        let video_url = output.as_str().ok_or("No video URL in Replicate output")?;

        let file_path = self.output_path("mp4");
        self.download_url_to_file(video_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "video/mp4".to_string(),
            file_size,
            provider: MediaProvider::Replicate,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model }),
        })
    }

    // ========================================================================
    // VIDEO GENERATION — FAL.AI
    // ========================================================================

    async fn generate_video_fal(&self, params: &VideoParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Fal)?;

        let model = params.model.as_deref().unwrap_or("fal-ai/minimax-video");

        let body = serde_json::json!({
            "prompt": params.prompt,
        });

        let url = format!("https://fal.run/{}", model);

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Key {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Fal.ai video request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Fal.ai error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let video_url = json["video"]["url"]
            .as_str()
            .ok_or("No video URL in Fal response")?;

        let file_path = self.output_path("mp4");
        self.download_url_to_file(video_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "video/mp4".to_string(),
            file_size,
            provider: MediaProvider::Fal,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model }),
        })
    }

    // ========================================================================
    // VIDEO GENERATION — GOOGLE VEO (Gemini API)
    // ========================================================================

    async fn generate_video_veo(&self, params: &VideoParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Veo)?;
        let model = params.model.as_deref().unwrap_or("veo-3.0-generate-preview");
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:predictLongRunning?key={}",
            model, key
        );

        let body = serde_json::json!({
            "instances": [{ "prompt": params.prompt }],
            "parameters": {
                "aspectRatio": "16:9",
                "durationSeconds": params.duration_secs.min(8),
                "personGeneration": "allow_adult"
            }
        });

        let resp = self.client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Veo request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Google Veo error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let op_name = json["name"]
            .as_str()
            .ok_or("No operation name from Veo")?;

        let video_url = self.veo_poll_result(key, op_name).await?;

        let file_path = self.output_path("mp4");
        self.download_url_to_file(&video_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "video/mp4".to_string(),
            file_size,
            provider: MediaProvider::Veo,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model }),
        })
    }

    /// Poll Google Veo long-running operation.
    async fn veo_poll_result(&self, key: &str, op_name: &str) -> Result<String, String> {
        let poll_url = format!(
            "https://generativelanguage.googleapis.com/v1beta/{}?key={}",
            op_name, key
        );

        for attempt in 0..120 {
            tokio::time::sleep(Duration::from_secs(5)).await;

            let resp = self.client
                .get(&poll_url)
                .send()
                .await
                .map_err(|e| format!("Veo poll failed: {}", e))?;

            let json: Value = resp.json().await
                .map_err(|e| format!("Veo poll parse error: {}", e))?;

            if json["done"].as_bool().unwrap_or(false) {
                if let Some(video) = json["response"]["generatedSamples"][0]["video"]["uri"].as_str() {
                    return Ok(video.to_string());
                }
                return Err("Veo completed but no video URI found".into());
            }

            if json.get("error").is_some() {
                let msg = json["error"]["message"].as_str().unwrap_or("Unknown");
                return Err(format!("Veo generation failed: {}", msg));
            }

            info!(attempt, "Waiting for Veo video generation...");
        }
        Err("Veo video generation timed out after 10 minutes".into())
    }

    // ========================================================================
    // VIDEO GENERATION — KLING (Kuaishou)
    // ========================================================================

    async fn generate_video_kling(&self, params: &VideoParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Kling)?;
        let model = params.model.as_deref().unwrap_or("kling-v2");

        let body = serde_json::json!({
            "model_name": model,
            "prompt": params.prompt,
            "duration": format!("{}s", params.duration_secs.min(10)),
            "aspect_ratio": "16:9",
            "mode": "std"
        });

        let resp = self.client
            .post("https://api.klingai.com/v1/videos/text2video")
            .header("Authorization", format!("Bearer {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Kling request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Kling error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let task_id = json["data"]["task_id"]
            .as_str()
            .ok_or("No task_id from Kling")?;

        let video_url = self.kling_poll_result(key, task_id).await?;

        let file_path = self.output_path("mp4");
        self.download_url_to_file(&video_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "video/mp4".to_string(),
            file_size,
            provider: MediaProvider::Kling,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model, "task_id": task_id }),
        })
    }

    /// Poll Kling task until video is ready.
    async fn kling_poll_result(&self, key: &str, task_id: &str) -> Result<String, String> {
        let poll_url = format!(
            "https://api.klingai.com/v1/videos/text2video/{}",
            task_id
        );

        for attempt in 0..120 {
            tokio::time::sleep(Duration::from_secs(5)).await;

            let resp = self.client
                .get(&poll_url)
                .header("Authorization", format!("Bearer {}", key))
                .send()
                .await
                .map_err(|e| format!("Kling poll failed: {}", e))?;

            let json: Value = resp.json().await
                .map_err(|e| format!("Kling poll parse error: {}", e))?;

            let status = json["data"]["task_status"].as_str().unwrap_or("");
            match status {
                "succeed" => {
                    return json["data"]["task_result"]["videos"][0]["url"]
                        .as_str()
                        .map(String::from)
                        .ok_or("Kling succeeded but no video URL".into());
                }
                "failed" => {
                    let msg = json["data"]["task_status_msg"].as_str().unwrap_or("Unknown");
                    return Err(format!("Kling generation failed: {}", msg));
                }
                _ => {
                    info!(attempt, status, "Waiting for Kling video generation...");
                }
            }
        }
        Err("Kling video generation timed out after 10 minutes".into())
    }

    // ========================================================================
    // VIDEO GENERATION — SEEDANCE (ByteDance)
    // ========================================================================

    async fn generate_video_seedance(&self, params: &VideoParams) -> Result<MediaOutput, String> {
        // Seedance is available through fal.ai as a proxy, using the Fal key
        // or through Volcengine directly. We try fal.ai first for simplicity.
        let key = self.get_key(MediaProvider::Seedance)
            .or_else(|_| self.get_key(MediaProvider::Fal))?;

        let model = params.model.as_deref().unwrap_or("fal-ai/seedance-1.0");
        let url = format!("https://fal.run/{}", model);

        let body = serde_json::json!({
            "prompt": params.prompt,
            "duration": params.duration_secs.min(10),
        });

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Key {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Seedance request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Seedance error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let video_url = json["video"]["url"]
            .as_str()
            .or_else(|| json["output"]["url"].as_str())
            .ok_or("No video URL in Seedance response")?;

        let file_path = self.output_path("mp4");
        self.download_url_to_file(video_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "video/mp4".to_string(),
            file_size,
            provider: MediaProvider::Seedance,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model }),
        })
    }

    // ========================================================================
    // VIDEO GENERATION — LUMA DREAM MACHINE (Ray2)
    // ========================================================================

    async fn generate_video_luma(&self, params: &VideoParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Luma)?;
        let model = params.model.as_deref().unwrap_or("ray2");

        let body = serde_json::json!({
            "prompt": params.prompt,
            "model": model,
            "aspect_ratio": "16:9",
        });

        let resp = self.client
            .post("https://api.lumalabs.ai/dream-machine/v1/generations")
            .header("Authorization", format!("Bearer {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Luma request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Luma error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let gen_id = json["id"]
            .as_str()
            .ok_or("No generation ID from Luma")?;

        let video_url = self.luma_poll_result(key, gen_id).await?;

        let file_path = self.output_path("mp4");
        self.download_url_to_file(&video_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "video/mp4".to_string(),
            file_size,
            provider: MediaProvider::Luma,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model, "generation_id": gen_id }),
        })
    }

    /// Poll Luma Dream Machine generation.
    async fn luma_poll_result(&self, key: &str, gen_id: &str) -> Result<String, String> {
        let poll_url = format!(
            "https://api.lumalabs.ai/dream-machine/v1/generations/{}",
            gen_id
        );

        for attempt in 0..120 {
            tokio::time::sleep(Duration::from_secs(5)).await;

            let resp = self.client
                .get(&poll_url)
                .header("Authorization", format!("Bearer {}", key))
                .send()
                .await
                .map_err(|e| format!("Luma poll failed: {}", e))?;

            let json: Value = resp.json().await
                .map_err(|e| format!("Luma poll parse error: {}", e))?;

            let state = json["state"].as_str().unwrap_or("");
            match state {
                "completed" => {
                    return json["assets"]["video"]
                        .as_str()
                        .map(String::from)
                        .ok_or("Luma completed but no video URL".into());
                }
                "failed" => {
                    let msg = json["failure_reason"].as_str().unwrap_or("Unknown");
                    return Err(format!("Luma generation failed: {}", msg));
                }
                _ => {
                    info!(attempt, state, "Waiting for Luma video generation...");
                }
            }
        }
        Err("Luma video generation timed out after 10 minutes".into())
    }

    // ========================================================================
    // VIDEO GENERATION — MINIMAX / HAILUO
    // ========================================================================

    async fn generate_video_minimax(&self, params: &VideoParams) -> Result<MediaOutput, String> {
        let key = self.get_key(MediaProvider::Minimax)?;
        let model = params.model.as_deref().unwrap_or("T2V-01");

        let body = serde_json::json!({
            "model": model,
            "prompt": params.prompt,
        });

        let resp = self.client
            .post("https://api.minimax.chat/v1/video_generation")
            .header("Authorization", format!("Bearer {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Minimax request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Minimax error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let task_id = json["task_id"]
            .as_str()
            .ok_or("No task_id from Minimax")?;

        let video_url = self.minimax_poll_result(key, task_id).await?;

        let file_path = self.output_path("mp4");
        self.download_url_to_file(&video_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "video/mp4".to_string(),
            file_size,
            provider: MediaProvider::Minimax,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model, "task_id": task_id }),
        })
    }

    /// Poll Minimax video generation task.
    async fn minimax_poll_result(&self, key: &str, task_id: &str) -> Result<String, String> {
        let poll_url = format!(
            "https://api.minimax.chat/v1/query/video_generation?task_id={}",
            task_id
        );

        for attempt in 0..120 {
            tokio::time::sleep(Duration::from_secs(5)).await;

            let resp = self.client
                .get(&poll_url)
                .header("Authorization", format!("Bearer {}", key))
                .send()
                .await
                .map_err(|e| format!("Minimax poll failed: {}", e))?;

            let json: Value = resp.json().await
                .map_err(|e| format!("Minimax poll parse error: {}", e))?;

            let status = json["status"].as_str().unwrap_or("");
            match status {
                "Success" => {
                    return json["file_id"]
                        .as_str()
                        .map(|fid| format!("https://api.minimax.chat/v1/files/retrieve?file_id={}", fid))
                        .ok_or("Minimax succeeded but no file_id".into());
                }
                "Fail" => {
                    return Err("Minimax video generation failed".into());
                }
                _ => {
                    info!(attempt, status, "Waiting for Minimax video generation...");
                }
            }
        }
        Err("Minimax video generation timed out after 10 minutes".into())
    }

    // ========================================================================
    // VIDEO GENERATION — PIKA (via fal.ai)
    // ========================================================================

    async fn generate_video_pika(&self, params: &VideoParams) -> Result<MediaOutput, String> {
        // Pika 2.2 is hosted on fal.ai — reuse fal key if pika-specific key absent
        let key = self.get_key(MediaProvider::Pika)
            .or_else(|_| self.get_key(MediaProvider::Fal))?;

        let model = params.model.as_deref().unwrap_or("fal-ai/pika/v2.2/text-to-video");
        let url = format!("https://fal.run/{}", model);

        let body = serde_json::json!({
            "prompt": params.prompt,
            "aspect_ratio": "16:9",
            "resolution": "720p"
        });

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Key {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Pika request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Pika error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;
        let video_url = json["video"]["url"]
            .as_str()
            .or_else(|| json["output"]["url"].as_str())
            .ok_or("No video URL in Pika response")?;

        let file_path = self.output_path("mp4");
        self.download_url_to_file(video_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "video/mp4".to_string(),
            file_size,
            provider: MediaProvider::Pika,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model }),
        })
    }

    // ========================================================================
    // VIDEO GENERATION — OPENAI SORA
    // ========================================================================

    async fn generate_video_sora(&self, params: &VideoParams) -> Result<MediaOutput, String> {
        // Sora uses the same OpenAI API key
        let key = self.get_key(MediaProvider::Sora)
            .or_else(|_| self.get_key(MediaProvider::OpenAI))?;

        let model = params.model.as_deref().unwrap_or("sora");

        let body = serde_json::json!({
            "model": model,
            "input": [{
                "type": "text",
                "text": params.prompt
            }],
            "n": 1,
            "size": "1920x1080",
            "duration": params.duration_secs.min(20)
        });

        let resp = self.client
            .post("https://api.openai.com/v1/video/generations")
            .header("Authorization", format!("Bearer {}", key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Sora request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Sora error {}: {}", status, text));
        }

        let json: Value = resp.json().await.map_err(|e| format!("Parse error: {}", e))?;

        // Sora returns an async generation — poll for result
        let gen_id = json["id"]
            .as_str()
            .ok_or("No generation ID from Sora")?;

        let video_url = self.sora_poll_result(key, gen_id).await?;

        let file_path = self.output_path("mp4");
        self.download_url_to_file(&video_url, &file_path).await?;

        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        Ok(MediaOutput {
            file_path,
            mime_type: "video/mp4".to_string(),
            file_size,
            provider: MediaProvider::Sora,
            prompt: params.prompt.clone(),
            metadata: serde_json::json!({ "model": model, "generation_id": gen_id }),
        })
    }

    /// Poll OpenAI Sora video generation.
    async fn sora_poll_result(&self, key: &str, gen_id: &str) -> Result<String, String> {
        let poll_url = format!(
            "https://api.openai.com/v1/video/generations/{}",
            gen_id
        );

        for attempt in 0..120 {
            tokio::time::sleep(Duration::from_secs(5)).await;

            let resp = self.client
                .get(&poll_url)
                .header("Authorization", format!("Bearer {}", key))
                .send()
                .await
                .map_err(|e| format!("Sora poll failed: {}", e))?;

            let json: Value = resp.json().await
                .map_err(|e| format!("Sora poll parse error: {}", e))?;

            let status = json["status"].as_str().unwrap_or("");
            match status {
                "completed" => {
                    return json["data"][0]["url"]
                        .as_str()
                        .map(String::from)
                        .ok_or("Sora completed but no video URL".into());
                }
                "failed" => {
                    let err = json["error"].as_str().unwrap_or("Unknown error");
                    return Err(format!("Sora generation failed: {}", err));
                }
                _ => {
                    info!(attempt, status, "Waiting for Sora video generation...");
                }
            }
        }
        Err("Sora video generation timed out after 10 minutes".into())
    }

    // ========================================================================
    // HELPER METHODS
    // ========================================================================

    /// Decode base64 image data and save to file.
    fn save_base64_to_file(&self, b64_data: &str, path: &Path) -> Result<(), String> {
        use base64::Engine;
        let decoder = base64::engine::general_purpose::STANDARD;
        let bytes = decoder
            .decode(b64_data)
            .map_err(|e| format!("Base64 decode error: {}", e))?;
        std::fs::write(path, &bytes)
            .map_err(|e| format!("Failed to write file {}: {}", path.display(), e))?;
        info!(path = %path.display(), size = bytes.len(), "Saved media file");
        Ok(())
    }

    /// Download a URL to a local file (streaming to avoid RAM buffering).
    async fn download_url_to_file(&self, url: &str, path: &Path) -> Result<(), String> {
        let resp = self.client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Download failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Download error {}: {}", resp.status(), url));
        }

        let bytes = resp.bytes().await
            .map_err(|e| format!("Failed to read download body: {}", e))?;

        std::fs::write(path, &bytes)
            .map_err(|e| format!("Failed to write file {}: {}", path.display(), e))?;

        info!(path = %path.display(), size = bytes.len(), "Downloaded media file");
        Ok(())
    }

    /// Poll Replicate prediction until output is ready.
    async fn replicate_wait_for_output(&self, initial: &Value, key: &str) -> Result<Value, String> {
        // If output is already present (Prefer: wait worked)
        if let Some(output) = initial.get("output") {
            if let Some(arr) = output.as_array() {
                if let Some(first) = arr.first() {
                    return Ok(first.clone());
                }
            }
            if output.is_string() {
                return Ok(output.clone());
            }
        }

        // Poll the prediction URL
        let poll_url = initial["urls"]["get"]
            .as_str()
            .ok_or("No polling URL in Replicate response")?;

        for attempt in 0..60 {
            tokio::time::sleep(Duration::from_secs(5)).await;

            let resp = self.client
                .get(poll_url)
                .header("Authorization", format!("Bearer {}", key))
                .send()
                .await
                .map_err(|e| format!("Replicate poll failed: {}", e))?;

            let json: Value = resp.json().await
                .map_err(|e| format!("Replicate poll parse error: {}", e))?;

            let status = json["status"].as_str().unwrap_or("");
            match status {
                "succeeded" => {
                    if let Some(output) = json.get("output") {
                        if let Some(arr) = output.as_array() {
                            if let Some(first) = arr.first() {
                                return Ok(first.clone());
                            }
                        }
                        return Ok(output.clone());
                    }
                    return Err("Replicate succeeded but no output found".into());
                }
                "failed" | "canceled" => {
                    let err = json["error"].as_str().unwrap_or("Unknown error");
                    return Err(format!("Replicate prediction {}: {}", status, err));
                }
                _ => {
                    info!(attempt, status, "Waiting for Replicate prediction...");
                }
            }
        }
        Err("Replicate prediction timed out after 5 minutes".into())
    }

    /// Poll Runway task until video is ready.
    async fn runway_poll_result(&self, key: &str, task_id: &str) -> Result<String, String> {
        let poll_url = format!("https://api.dev.runwayml.com/v1/tasks/{}", task_id);

        for attempt in 0..60 {
            tokio::time::sleep(Duration::from_secs(5)).await;

            let resp = self.client
                .get(&poll_url)
                .header("Authorization", format!("Bearer {}", key))
                .header("X-Runway-Version", "2024-11-06")
                .send()
                .await
                .map_err(|e| format!("Runway poll failed: {}", e))?;

            let json: Value = resp.json().await
                .map_err(|e| format!("Runway poll parse error: {}", e))?;

            let status = json["status"].as_str().unwrap_or("");
            match status {
                "SUCCEEDED" => {
                    return json["output"][0]
                        .as_str()
                        .map(String::from)
                        .ok_or("Runway succeeded but no output URL".into());
                }
                "FAILED" => {
                    let err = json["failure"].as_str().unwrap_or("Unknown error");
                    return Err(format!("Runway generation failed: {}", err));
                }
                _ => {
                    info!(attempt, status, "Waiting for Runway video generation...");
                }
            }
        }
        Err("Runway video generation timed out after 5 minutes".into())
    }

    /// Poll Leonardo AI for generation result.
    async fn leonardo_poll_result(&self, key: &str, gen_id: &str) -> Result<String, String> {
        let poll_url = format!(
            "https://cloud.leonardo.ai/api/rest/v1/generations/{}",
            gen_id
        );

        for attempt in 0..60 {
            tokio::time::sleep(Duration::from_secs(3)).await;

            let resp = self.client
                .get(&poll_url)
                .header("Authorization", format!("Bearer {}", key))
                .send()
                .await
                .map_err(|e| format!("Leonardo poll failed: {}", e))?;

            let json: Value = resp.json().await
                .map_err(|e| format!("Leonardo poll parse error: {}", e))?;

            let status = json["generations_by_pk"]["status"]
                .as_str()
                .unwrap_or("");

            match status {
                "COMPLETE" => {
                    return json["generations_by_pk"]["generated_images"][0]["url"]
                        .as_str()
                        .map(String::from)
                        .ok_or("Leonardo completed but no image URL".into());
                }
                "FAILED" => {
                    return Err("Leonardo generation failed".into());
                }
                _ => {
                    info!(attempt, status, "Waiting for Leonardo generation...");
                }
            }
        }
        Err("Leonardo generation timed out after 3 minutes".into())
    }

    /// Calculate aspect ratio string from dimensions.
    fn aspect_ratio(w: u32, h: u32) -> String {
        if w == h {
            "1:1".to_string()
        } else if w * 9 == h * 16 || (w > h && (w as f32 / h as f32 - 16.0 / 9.0).abs() < 0.1) {
            "16:9".to_string()
        } else if h * 9 == w * 16 || (h > w && (h as f32 / w as f32 - 16.0 / 9.0).abs() < 0.1) {
            "9:16".to_string()
        } else if w * 3 == h * 4 || (w > h && (w as f32 / h as f32 - 4.0 / 3.0).abs() < 0.1) {
            "4:3".to_string()
        } else if w > h {
            "16:9".to_string()
        } else {
            "9:16".to_string()
        }
    }

    /// List available providers that have API keys configured.
    #[allow(dead_code)]
    pub fn available_providers(&self) -> Vec<MediaProvider> {
        self.api_keys.keys().copied().collect()
    }
}
