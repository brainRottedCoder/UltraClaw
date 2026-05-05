# PowerShell install script for Windows
Write-Host ""
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host "  ULTRACLAW — GARAGE_INFERENCE Installer" -ForegroundColor Cyan
Write-Host "  Tier 1 | phi4-mini | $0 Inference" -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host ""

# 1. Check Rust
if (!(Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "> Installing Rust toolchain..." -ForegroundColor Yellow
    Invoke-WebRequest -Uri "https://win.rustup.rs" -OutFile rustup-init.exe
    .\rustup-init.exe -y
    Remove-Item rustup-init.exe
    $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
} else {
    Write-Host "✓ Rust found" -ForegroundColor Green
}

# 2. Check Ollama
if (!(Get-Command ollama -ErrorAction SilentlyContinue)) {
    Write-Host "> Download Ollama from: https://ollama.com/download/windows" -ForegroundColor Yellow
    Write-Host "  After install, run: ollama pull phi4-mini" -ForegroundColor Yellow
} else {
    Write-Host "✓ Ollama found" -ForegroundColor Green
    Write-Host "> Pulling phi4-mini:latest (3.8B)..." -ForegroundColor Yellow
    ollama pull phi4-mini
}

# 3. Python deps
$python = Get-Command python -ErrorAction SilentlyContinue
if ($python) {
    Write-Host "> Installing Python deps..." -ForegroundColor Yellow
    python -m pip install -r requirements.txt 2>$null
    python -m playwright install chromium 2>$null
} else {
    Write-Host "⚠ Python not found — browser/doc/TTS tools disabled" -ForegroundColor DarkYellow
}

# 4. Build
Write-Host "> Building Ultraclaw (release)..." -ForegroundColor Yellow
cargo build --release

Write-Host ""
Write-Host "=========================================="  -ForegroundColor Cyan
Write-Host "  INSTALLATION COMPLETE!" -ForegroundColor Green
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "  cargo run --release -- --demo" -ForegroundColor White
Write-Host "  cargo run --release -- --benchmark" -ForegroundColor White
Write-Host "  cargo run --release -- --compare" -ForegroundColor White
Write-Host ""