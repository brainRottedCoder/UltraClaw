#!/bin/bash
set -e

echo ""
echo "=========================================="
echo "  ULTRACLAW — GARAGE_INFERENCE Installer"
echo "  Tier 1 | phi4-mini | $0 Inference"
echo "=========================================="
echo ""

# 1. Install Rust
if ! command -v cargo &>/dev/null; then
    echo "› Installing Rust toolchain..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    . "$HOME/.cargo/env"
else
    echo "✓ Rust toolchain found: $(rustc --version)"
fi

# 2. Install Ollama
if ! command -v ollama &>/dev/null; then
    echo "› Installing Ollama (local LLM runtime)..."
    curl -fsSL https://ollama.com/install.sh | sh
else
    echo "✓ Ollama found: $(ollama --version)"
fi

# 3. Pull the model (phi4-mini, Tier 1)
echo "› Pulling phi4-mini:latest (3.8B) — this downloads ~4GB once..."
ollama pull phi4-mini || echo "⚠ Model pull failed — make sure Ollama is running"

# 4. Install Python deps (optional but recommended)
if command -v python3 &>/dev/null || command -v python &>/dev/null; then
    PYTHON=$(command -v python3 || command -v python)
    echo "› Installing Python dependencies..."
    $PYTHON -m pip install -r requirements.txt 2>/dev/null || echo "⚠ Python deps optional"
    
    if $PYTHON -c "import playwright" 2>/dev/null; then
        $PYTHON -m playwright install chromium 2>/dev/null || echo "⚠ Playwright browser install skipped"
    fi
else
    echo "⚠ Python not found — browser tools, doc parsing, TTS will be disabled"
fi

# 5. Build Ultraclaw
echo "› Building Ultraclaw (release mode)..."
cargo build --release

echo ""
echo "=========================================="
echo "  INSTALLATION COMPLETE!"
echo "=========================================="
echo ""
echo "  Run the demo:"
echo "    cargo run --release -- --demo"
echo ""
echo "  Run benchmarks:"
echo "    cargo run --release -- --benchmark"
echo ""
echo "  Compare bare vs scaffolded:"
echo "    cargo run --release -- --compare"
echo ""
echo "  Interactive CLI:"
echo "    cargo run --release -- --cli"
echo ""
echo "  Use a different model:"
echo "    cargo run --release -- --model=qwen3:0.6b --cli"
echo ""