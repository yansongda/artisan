English | [简体中文](./README.zh-CN.md)

# Artisan

## Workspace Structure

```
artisan/
├── Cargo.toml              # Workspace configuration
├── src/lib.rs              # Facade (feature-controlled re-exports)
└── artisan-http/           # HTTP implementation
    ├── src/                # Core implementation
    ├── tests/              # Tests
    ├── examples/           # Examples
    └── docs/               # Architecture docs
```

## Crates

| Crate | Role | Docs |
|-------|------|------|
| [`artisan`](.) | Facade with feature-controlled re-exports | [docs.rs/artisan](https://docs.rs/artisan) |
| [`artisan-http`](./artisan-http) | HTTP client, onion model, plugin system | [README](./artisan-http/README.md#quick-start) |

## Installation

```bash
# Recommended: via the facade (HTTP included by default)
cargo add artisan

# Depend on the implementation crate directly
cargo add artisan-http

# Pure facade (HTTP disabled)
cargo add artisan --no-default-features
```

```toml
# Cargo.toml
[dependencies]
artisan = "0.15.0"

# Depend on the implementation crate directly
[dependencies]
artisan-http = "0.15.0"

# Pure facade (HTTP disabled)
[dependencies]
artisan = { version = "0.15.0", default-features = false }
```

## Quick Start

### artisan-http

- **Getting started**: [README](./artisan-http/README.md#quick-start)
- **Architecture design**: [docs/ARCHITECTURE.md](./artisan-http/docs/ARCHITECTURE.md)
- **Examples**: [examples/](./artisan-http/examples/)

## Examples

### artisan-http

```bash
cargo run -p artisan-http --example basic
cargo run -p artisan-http --example config
cargo run -p artisan-http --example shortcut
cargo run -p artisan-http --example custom_plugin
cargo run -p artisan-http --example direction
```

## License

MIT License
