use std::sync::Arc;
use std::net::SocketAddr;
use axum::{
    Router,
    routing::get,
    response::Json,
    extract::State,
    http::header,
    body::Body,
};
use serde::Serialize;
use tokio::sync::Mutex;
use tracing::info;

#[derive(Debug, Clone)]
pub struct DashboardState {
    pub uptime_secs: Arc<Mutex<u64>>,
    pub model_name: String,
    pub tier: String,
    pub inference_cost: String,
    pub hardware: String,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub project: String,
    pub version: String,
    pub model: String,
    pub tier: String,
    pub inference_cost: String,
    pub hardware: String,
    pub uptime_secs: u64,
    pub hackathon: String,
}

#[derive(Debug, Serialize)]
pub struct MemoryStatsResponse {
    pub total_entries: usize,
    pub categories: Vec<String>,
    pub oldest_entry_age_days: f64,
}

async fn api_status(State(state): State<Arc<DashboardState>>) -> Json<StatusResponse> {
    let uptime = *state.uptime_secs.lock().await;
    Json(StatusResponse {
        project: "Ultraclaw".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        model: state.model_name.clone(),
        tier: state.tier.clone(),
        inference_cost: state.inference_cost.clone(),
        hardware: state.hardware.clone(),
        uptime_secs: uptime,
        hackathon: "GARAGE_INFERENCE 2026".to_string(),
    })
}

async fn api_memory() -> Json<MemoryStatsResponse> {
    Json(MemoryStatsResponse {
        total_entries: 0,
        categories: vec!["preferences".to_string(), "context".to_string(), "facts".to_string()],
        oldest_entry_age_days: 0.0,
    })
}

const DASHBOARD_HTML: &str = r#"
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Ultraclaw Dashboard</title>
<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: 'Courier New', monospace; background: #0a0a0a; color: #e0e0e0; padding: 2rem; min-height: 100vh; }
.header { text-align: center; margin-bottom: 2rem; }
.header h1 { color: #00ffff; font-size: 2rem; }
.header .badge { display: inline-block; background: #1a1a2e; border: 1px solid #333; padding: 0.25rem 0.75rem; margin: 0.25rem; border-radius: 4px; font-size: 0.85rem; }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 1rem; max-width: 900px; margin: 0 auto; }
.card { background: #111; border: 1px solid #333; border-radius: 8px; padding: 1.25rem; }
.card h3 { color: #00ffff; margin-bottom: 0.75rem; font-size: 1rem; }
.card p { margin: 0.4rem 0; font-size: 0.9rem; }
.card .value { color: #00ff88; }
.card .warn { color: #ffaa00; }
.footer { text-align: center; margin-top: 2rem; font-size: 0.8rem; color: #555; }
#status { animation: pulse 2s infinite; }
@keyframes pulse { 0%,100% { opacity: 1; } 50% { opacity: 0.6; } }
</style>
</head>
<body>
<div class="header">
    <h1>ULTRACLAW</h1>
    <div>
        <span class="badge">GARAGE_INFERENCE 2026</span>
        <span class="badge">Tier 1</span>
        <span class="badge">$0 Inference</span>
    </div>
</div>
<div class="grid" id="status">Loading...</div>
<div class="footer">Ultraclaw · phi4-mini (3.8B) · Built with Rust</div>
<script>
async function load() {
    try {
        const resp = await fetch('/api/status');
        const data = await resp.json();
        document.getElementById('status').innerHTML = `
            <div class="card"><h3>STATUS</h3>
                <p>Project: <span class="value">${data.project}</span></p>
                <p>Version: <span class="value">${data.version}</span></p>
                <p>Uptime: <span class="value">${data.uptime_secs}s</span></p>
            </div>
            <div class="card"><h3>MODEL</h3>
                <p>Name: <span class="value">${data.model}</span></p>
                <p>Tier: <span class="value">${data.tier}</span></p>
                <p>Cost: <span class="value">${data.inference_cost}</span></p>
                <p>Hardware: <span class="value">${data.hardware}</span></p>
            </div>
            <div class="card"><h3>HACKATHON</h3>
                <p>Event: <span class="value">${data.hackathon}</span></p>
                <p>Engineer: <span class="value">8 Scaffolding Layers</span></p>
                <p>Status: <span class="warn">Live</span></p>
            </div>
        `;
    } catch(e) {
        document.getElementById('status').innerHTML = '<div class="card"><h3>Error</h3><p>Could not connect to agent</p></div>';
    }
}
load();
setInterval(load, 5000);
</script>
</body>
</html>
"#;

pub struct WebDashboard {
    state: Arc<DashboardState>,
    port: u16,
}

impl WebDashboard {
    pub fn new(model_name: String, port: u16) -> Self {
        Self {
            state: Arc::new(DashboardState {
                uptime_secs: Arc::new(Mutex::new(0)),
                model_name,
                tier: "1 — Absolute Garage MAX".to_string(),
                inference_cost: "$0.00".to_string(),
                hardware: "CPU-only, $500 laptop, 4-6GB RAM".to_string(),
            }),
            port,
        }
    }

    pub async fn serve(&self) {
        let state = self.state.clone();

        let app = Router::new()
            .route("/", get(|| async {
                axum::response::Response::builder()
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .body(Body::from(DASHBOARD_HTML))
                    .unwrap()
            }))
            .route("/api/status", get(api_status))
            .route("/api/memory", get(api_memory))
            .with_state(state.clone());

        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        info!(port = self.port, "Web Dashboard available at http://127.0.0.1:{}/", self.port);

        let state_clone = state.clone();
        tokio::spawn(async move {
            let start = std::time::Instant::now();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                *state_clone.uptime_secs.lock().await = start.elapsed().as_secs();
            }
        });

        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(error = %e, "Dashboard failed to bind to port {}", self.port);
                return;
            }
        };
        if let Err(e) = axum::serve(listener, app.into_make_service()).await {
            tracing::error!(error = %e, "Dashboard server error");
        }
    }
}