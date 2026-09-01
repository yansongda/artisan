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
artisan = "0.17.0"

# Depend on the implementation crate directly
[dependencies]
artisan-http = "0.17.0"

# Pure facade (HTTP disabled)
[dependencies]
artisan = { version = "0.17.0", default-features = false }
```

## Quick Start

### artisan-http

- **Getting started**: [README](./artisan-http/README.md#quick-start)
- **Architecture design**: [docs/ARCHITECTURE.md](./artisan-http/docs/ARCHITECTURE.md)
- **Examples**: [examples/](./artisan-http/examples/)

### Events (artisan-http)

Each `Artful` instance carries a built-in event dispatcher: register listeners via the builder to observe the request lifecycle without writing a full plugin (zero cost when no listener is registered).

```rust
use artisan_http::{Artful, Event, EventListener};
use std::sync::Arc;

struct LoggingListener;

impl EventListener for LoggingListener {
    fn name(&self) -> &'static str {
        "LoggingListener"
    }

    fn on_event(&self, event: &mut Event<'_>) -> artisan_http::Result<()> {
        match event {
            Event::ArtfulStart { params, plugins } => {
                eprintln!("ArtfulStart: {} params, {} plugins", params.len(), plugins.len())
            }
            Event::HttpStart { rocket } => {
                eprintln!("HttpStart: {} {}", rocket.config.method, rocket.config.url)
            }
            Event::HttpEnd { rocket } => {
                eprintln!("HttpEnd: {:?}", rocket.destination_origin.as_ref().map(|r| r.status()))
            }
            Event::HttpError { error, .. } => eprintln!("HttpError: {error}"),
            Event::ArtfulEnd { .. } => eprintln!("ArtfulEnd"),
        }
        Ok(()) // bypass listener: consume errors internally, never return Err
    }
}

let artful = Artful::builder()
    .event_listener(Arc::new(LoggingListener))
    .build()?;
```

| Event | Fires | Mutability |
|-------|-------|------------|
| `ArtfulStart` | before the plugin chain starts | read-only |
| `HttpStart` | before the HTTP request is sent, at the tail core action execution point (`IgniteCore`, mounted automatically by the framework; radar already built in a normal chain; `None` if the chain lacks `AddRadarPlugin` - the event still fires; mutate the request via the `*_mut` accessors on `rocket.radar`) | mutable |
| `HttpEnd` | after a successful response, before parsing (response body is NOT readable - body consumption belongs to direction parsing; only status / headers are readable) | read-only |
| `HttpError` | when the HTTP request execution fails (the error still propagates) | read-only |
| `ArtfulEnd` | after the chain succeeds, before returning the destination (may rewrite `rocket.destination`) | mutable |

> - Listeners are **synchronous** and must be non-blocking — spawn heavy work yourself (`tokio::spawn`).
> - A listener returning `Err` aborts the main flow (propagates as `EventListenerError`). Bypass listeners should consume errors internally and always return `Ok(())`.
> - `HttpEnd` cannot read the response body (ownership belongs to direction parsing); only status / headers are available there.

See the [artisan-http README Events section](./artisan-http/README.md#events) for details, or try `cargo run -p artisan-http --example event`.

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
