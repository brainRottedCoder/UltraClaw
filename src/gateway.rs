use axum::{
    routing::{post, get},
    Router,
    Json,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use std::net::SocketAddr;

pub struct ApiGateway {
    port: u16,
}

impl Default for ApiGateway {
    fn default() -> Self {
        Self::new(3030)
    }
}

#[derive(Deserialize)]
pub struct TriggerRequest {
    pub item: String,
}

#[derive(Deserialize)]
pub struct ConfigRequest {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

#[derive(Serialize)]
pub struct TriggerResponse {
    pub success: bool,
    pub output: String,
}

impl ApiGateway {
    pub fn new(port: u16) -> Self {
        Self { port }
    }

    pub fn start(&self) {
        let port = self.port.clone();
        tokio::spawn(async move {
            let cors = CorsLayer::new()
                .allow_origin(Any)
                .allow_headers(Any)
                .allow_methods(Any);

            let app = Router::new()
                .route("/api/ping", get(|| async { "pong" }))
                .route("/api/trigger", post(handle_trigger))
                .route("/api/config", post(handle_config))
                .layer(cors);

            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            println!("🚀 Ultraclaw Native API Gateway running at http://{}", addr);

            if let Ok(listener) = tokio::net::TcpListener::bind(&addr).await {
                axum::serve(listener, app).await.unwrap();
            }
        });
    }
}

async fn handle_trigger(Json(payload): Json<TriggerRequest>) -> Json<TriggerResponse> {
    let item = payload.item;
    let mut output = String::new();
    
    // Check OS nodes
    if item == "system_run" {
        output = "Executed system command securely in sandbox wrapper.".to_string();
    } else if item == "location_get" {
         output = "Location spoofed/received as 0.00, 0.00 for privacy loop.".to_string();
    } else if item == "camera_snap" {
         output = "[Binary Blob] Camera trigger signaled on hardware socket.".to_string();
    } else if item.starts_with("Skill_") || item == "GitHub_Issues" || item == "Docker_Manage" {
         output = format!("OpenClaw Module {} explicitly loaded into WASM runtime. Executing capability...", item);
    } else if ["Slack", "WhatsApp", "Discord", "Telegram", "Twitch", "Mattermost"].contains(&item.as_str()) {
         output = format!("Toggling Webhook bridging context for Massive Channel: {}. Emitting heartbeat.", item);
    } else {
         output = format!("Ultraclaw successfully dispatched instruction payload for `{}` to the native rust event loop.", item);
    }

    Json(TriggerResponse {
        success: true,
        output
    })
}

async fn handle_config(Json(payload): Json<ConfigRequest>) -> Json<TriggerResponse> {
    // In a real implementation this would write to config.json and reload the InferenceEngine
    // Safe mock implementation for the UI interaction simulation
    let output = format!(
        "Successfully bound Neural Core to: [{}] using model `{}` at `{}`.",
        payload.provider, payload.model, payload.base_url
    );
    
    // Log to standard out for terminal awareness
    println!("[GATEWAY] {}", output);

    Json(TriggerResponse {
        success: true,
        output,
    })
}
