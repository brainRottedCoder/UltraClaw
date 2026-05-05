# Contributing to Ultraclaw

First off, thanks for taking the time to contribute! 🎉

The goal of Ultraclaw is to build the world's most efficient, capable, and
modular AI agent for the Matrix network. We welcome contributions of all kinds,
from bug fixes and documentation improvements to new features and architectural
changes.

## Code of Conduct

This project connects people and AI. Please keep all interactions respectful and
constructive.

## How Can I Contribute?

### Reporting Bugs

- **Search Existing Issues**: Before creating a new issue, please search
  `https://github.com/brainRottedCoder/Ultraclaw/issues` to see if it has
  already been reported.
- **Create a Detailed Issue**: If you find a new bug, please provide as much
  detail as possible, including your OS, Rust version, and reproduction steps.

### Suggesting Enhancements

- **Feature Requests**: Open an issue with the label `enhancement`. Describe the
  feature you'd like to see and why it would be useful.
- **Architectural Proposals**: For major changes (e.g., swapping the database
  engine, adding a new protocol), please open a "RFC" (Request for Comments)
  issue first to discuss the design.

### Pull Requests

1. **Fork the Repo**: Click the "Fork" button on GitHub.
2. **Create a Branch**: `git checkout -b feature/amazing-feature`
3. **Commit Changes**: `git commit -m 'Add some amazing feature'`
4. **Push to Branch**: `git push origin feature/amazing-feature`
5. **Open a Pull Request**: Go to the original repository and click "New Pull
   Request".

## Development Setup

1. **Install Rust**: Ensure you have the latest stable Rust toolchain installed
   (`rustup update`).
2. **Dependencies**:
   - **Windows**: Make sure you have the MSVC build tools installed.
   - **Linux**: You may need `libssl-dev`, `pkg-config`, and `libsqlite3-dev`.
3. **Environment**: Copy `.env.example` to `.env` and configure your keys.
4. **Test**: Run `cargo check` and `cargo test` before submitting your PR.

## Style Guide

- **Rustfmt**: We use standard `rustfmt`. Please run `cargo fmt` before
  committing.
- **Clippy**: We aim for zero warnings. Run `cargo clippy` and fix any issues.
- **Comments**: Document public structs and functions using `///` doc comments.

## Project Structure

- `src/main.rs`: Entry point and boot sequence.
- `src/matrix.rs`: Matrix protocol handling.
- `src/inference.rs`: LLM abstraction (Cloud + Local).
- `src/media.rs`: Media generation engine (15+ providers).
- `src/config.rs`: Configuration loading.
- `src/soul.rs`: Agent personality and directives.
- `src/skill.rs`: Tool/Skill system.

Happy coding! 🦀
