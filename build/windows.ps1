# ============================================================================
# Ultraclaw — Windows Build Script
# ============================================================================
# Builds a release binary (.exe) for Windows.
#
# Usage:   .\build\windows.ps1
# Output:  target\release\ultraclaw.exe
#
# OPTIONAL: To create a proper Windows installer (.msi), install cargo-wix:
#   cargo install cargo-wix
#   cargo wix init   # generates WiX source files
#   cargo wix        # builds the .msi installer
# ============================================================================

Write-Host "╔══════════════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "║      ULTRACLAW — Windows Release Build           ║" -ForegroundColor Cyan
Write-Host "╚══════════════════════════════════════════════════╝" -ForegroundColor Cyan

# Ensure we're in the project root
$ErrorActionPreference = "Stop"

# Build the release binary
Write-Host ""
Write-Host "[1/3] Building release binary..." -ForegroundColor Yellow
cargo build --release

if ($LASTEXITCODE -ne 0) {
    Write-Host "Build FAILED!" -ForegroundColor Red
    exit 1
}

# Check the binary size
$binary = "target\release\ultraclaw.exe"
if (Test-Path $binary) {
    $size = (Get-Item $binary).Length / 1MB
    Write-Host "[2/3] Binary built: $binary ($([math]::Round($size, 2)) MB)" -ForegroundColor Green
} else {
    Write-Host "Binary not found at $binary" -ForegroundColor Red
    exit 1
}

# Optional: Create installer with cargo-wix
$hasWix = $null
try { $hasWix = Get-Command cargo-wix -ErrorAction SilentlyContinue } catch {}

if ($hasWix) {
    Write-Host "[3/3] Building Windows Installer (.msi)..." -ForegroundColor Yellow
    cargo wix
    if ($LASTEXITCODE -eq 0) {
        $msi = Get-ChildItem "target\wix\*.msi" | Select-Object -First 1
        if ($msi) {
            Write-Host "Installer built: $($msi.FullName)" -ForegroundColor Green
        }
    } else {
        Write-Host "WiX installer build failed (standalone .exe still available)" -ForegroundColor Yellow
    }
} else {
    Write-Host "[3/3] cargo-wix not installed — skipping .msi generation" -ForegroundColor Gray
    Write-Host "      Install with: cargo install cargo-wix" -ForegroundColor Gray
}

Write-Host ""
Write-Host "Build complete!" -ForegroundColor Green
Write-Host "Binary: $binary" -ForegroundColor Cyan
