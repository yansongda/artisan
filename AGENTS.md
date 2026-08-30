# AGENTS.md

## Project Overview

**artisan** - Rust HTTP client framework using onion model pattern.

- Edition: 2024
- MSRV: 1.85
- License: MIT

## Workspace Structure

```
artisan/                    # Root package (facade)
├── Cargo.toml              # Workspace config + facade package
├── src/lib.rs              # Facade with feature-controlled re-export
└── artisan-http/           # HTTP implementation crate
    ├── Cargo.toml
    ├── src/                # All implementation code
    ├── tests/              # All tests
    └── examples/           # All examples
```

### Crate Roles

- **artisan**: Facade crate with feature-controlled re-export
  - Default feature includes "http" (re-exports artisan-http)
  - Can be used without HTTP functionality: `default-features = false`
- **artisan-http**: HTTP implementation crate
  - Contains all implementation code
  - Independent version management
  - See [artisan-http/AGENTS.md](artisan-http/AGENTS.md) for details

### Feature Control

```bash
# Default: includes HTTP functionality
cargo add artisan

# Without HTTP: pure facade
cargo add artisan --no-default-features

# Direct dependency: explicit HTTP
cargo add artisan-http
```

```toml
# Cargo.toml
[dependencies]
artisan = "X.Y.Z"

# Without HTTP: pure facade
[dependencies]
artisan = { version = "X.Y.Z", default-features = false }

# Direct dependency: explicit HTTP
[dependencies]
artisan-http = "X.Y.Z"
```

## Commands

```bash
# Build & check (workspace)
cargo check --workspace --all-features

# Test
cargo test --workspace --all-features

# Format & lint
cargo fmt --all
cargo clippy --workspace -- -D warnings

# Run examples
cargo run -p artisan-http --example basic
cargo run -p artisan --example basic

# Publish (automated via GitHub tag)
# Manual: cargo publish -p artisan-http --token $CARGO_TOKEN
# Manual: cargo publish -p artisan --token $CARGO_TOKEN
```

## Before Commit (Mandatory)

After modifying any `.rs` file, ensure all three pass:

```bash
cargo fmt --all -- --check  # Format check
cargo clippy --workspace -- -D warnings # Lint check
cargo test --workspace --all-features   # All tests
```

## README Bilingual Policy

`README.md` (English, default) and `README.zh-CN.md` (Chinese) exist as a pair in both the repo root and `artisan-http/`. When changing either one, update its counterpart in the same change so the two stay in sync.

## References

- **Implementation details**: [artisan-http/AGENTS.md](artisan-http/AGENTS.md)
- Architecture: `artisan-http/docs/ARCHITECTURE.md` (comprehensive)
- Examples: `artisan-http/examples/*.rs` (5 working demos)
- CI: `.github/workflows/coding-linter.yml`, `.github/workflows/publish.yml`
