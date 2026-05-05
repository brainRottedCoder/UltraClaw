use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{info, warn};

static OFFLINE_MODE: AtomicBool = AtomicBool::new(false);
static INTERNET_AVAILABLE: AtomicBool = AtomicBool::new(true);
static OLLAMA_AVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn is_offline() -> bool {
    OFFLINE_MODE.load(Ordering::Relaxed)
}

pub fn is_internet_available() -> bool {
    INTERNET_AVAILABLE.load(Ordering::Relaxed)
}

pub fn is_ollama_available() -> bool {
    OLLAMA_AVAILABLE.load(Ordering::Relaxed)
}

pub fn set_ollama_available(available: bool) {
    OLLAMA_AVAILABLE.store(available, Ordering::Relaxed);
}

pub async fn detect_connectivity() {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok();

    if let Some(client) = client {
        match client.get("https://ollama.com").send().await {
            Ok(resp) if resp.status().is_success() => {
                INTERNET_AVAILABLE.store(true, Ordering::Relaxed);
                OFFLINE_MODE.store(false, Ordering::Relaxed);
                info!("✓ Internet connectivity confirmed");
                return;
            }
            _ => {}
        }
    }

    // Try a simpler check
    if let Some(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()
    {
        if client.get("https://www.google.com").send().await.is_ok() {
            INTERNET_AVAILABLE.store(true, Ordering::Relaxed);
            OFFLINE_MODE.store(false, Ordering::Relaxed);
            info!("✓ Internet connectivity confirmed (fallback)");
            return;
        }
    }

    INTERNET_AVAILABLE.store(false, Ordering::Relaxed);
    OFFLINE_MODE.store(true, Ordering::Relaxed);
    warn!("⚠ OFFLINE MODE — No internet connectivity detected");
    info!("  Local model will be used for all inference.");
    info!("  Web search, cloud APIs, and browser tools that require network will be disabled.");
}

pub fn print_offline_status() {
    if is_offline() {
        println!();
        println!("╔══════════════════════════════════════════╗");
        println!("║  ⚠ OFFLINE MODE                         ║");
        println!("║  No internet detected. Running local     ║");
        println!("║  model only. Some features disabled:     ║");
        println!("║  - Cloud inference fallback              ║");
        println!("║  - Web search / browsing                 ║");
        println!("║  - Media generation API calls            ║");
        println!("║  - MCP remote tools                      ║");
        println!("╠══════════════════════════════════════════╣");
        println!("║  Available: Local phi4-mini, file tools  ║");
        println!("║  memory, debate, self-healing            ║");
        println!("╚══════════════════════════════════════════╝");
        println!();
    }
}