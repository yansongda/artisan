# artisan-http AGENTS.md

## Overview

**artisan-http** - HTTP implementation crate for artisan framework.

- Edition: 2024
- MSRV: 1.85
- License: MIT

## Commands

```bash
# Build & check
cargo check -p artisan-http --all-features

# Test (65 tests)
cargo test -p artisan-http --all-features

# Format & lint
cargo fmt --all -- --check
cargo clippy -p artisan-http -- -D warnings

# Run examples
cargo run -p artisan-http --example basic
cargo run -p artisan-http --example config
cargo run -p artisan-http --example shortcut
cargo run -p artisan-http --example custom_plugin
cargo run -p artisan-http --example direction
```

## Module Structure

```
src/
├── lib.rs           # Public API exports
├── artful.rs        # Artful struct (instance: new/with_config, artful, shortcut, raw)
├── rocket.rs        # Rocket + RocketConfig + ClientOptions/RequestOptions
├── flow_ctrl.rs     # FlowCtrl + Next (onion control)
├── plugin.rs        # Plugin trait (async_trait)
├── plugins/         # Built-in plugins
│   ├── start.rs     # StartPlugin (init payload)
│   ├── add_radar.rs # AddRadarPlugin (build Request, fallback CT header)
│   ├── parser.rs    # ParserPlugin (execute + parse)
│   └── add_payload_body.rs
├── direction.rs     # Direction trait + DirectionKind + Destination
├── directions/      # Built-in parsers (JsonDirection)
├── packer.rs        # Packer trait (pack/unpack/content_type)
├── packers/         # Built-in serializers (JsonPacker)
├── shortcut.rs      # Shortcut trait
├── config.rs        # Config (http: ClientOptions + extra)
├── error.rs         # ArtfulError enum (thiserror, English Display)
└── http.rs          # build_client / default_client (pub(crate))
```

## Key Types

| Type | Role | File |
|------|------|------|
| `Artful` | Main entry point (instance type) | `src/artful.rs` |
| `Rocket` | Request/response carrier | `src/rocket.rs` |
| `ClientOptions` | Client-level HTTP options | `src/rocket.rs` |
| `RequestOptions` | Request-level HTTP options (timeout only) | `src/rocket.rs` |
| `Plugin` | Middleware trait | `src/plugin.rs` |
| `FlowCtrl` | Execution controller | `src/flow_ctrl.rs` |
| `Next` | Chain continuation | `src/flow_ctrl.rs` |
| `Direction` | Response parser trait | `src/direction.rs` |
| `Packer` | Serializer trait | `src/packer.rs` |
| `Shortcut` | Plugin preset trait | `src/shortcut.rs` |

## Patterns & Conventions

### Plugin Implementation

```rust
#[derive(Clone, Copy, Debug, Default)]  // Required for zero-size plugins
pub struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()> {
        // Forward logic
        next.call(rocket).await?;  // Propagate to next layer
        // Backward logic
        Ok(())
    }
}
```

### Artful Instance & Client

`Artful` is an instance type: config and `reqwest::Client` are resolved at construction (`Artful::new()` / `Artful::with_config(config)`, fail-fast via `build_client`). `rocket.client` is injected per request. App layer can wrap an instance in `std::sync::LazyLock` for a global singleton (see README). Client-level options live in `ClientOptions` (`Config.http`); request-level options in `RequestOptions` (`RocketConfig.http`, timeout only).

### Error Handling

- `ArtfulError` uses `thiserror` with English Display messages
- `JsonDeserializeError` requires `source: Option<serde_json::Error>`
- `RequestBuildError`/`ClientBuild` use `source: reqwest::Error`; `ClientBuild` must use explicit `#[source]` (`#[from]` is taken by `RequestFailed`)

### Shortcut Trait

```rust
pub trait Shortcut {  // no Default bound required
    fn get_plugins(&self, params: &HashMap<String, Value>) -> Vec<Arc<dyn Plugin>>;
}
```

## Testing

- 65 tests across 7 files
- Use `wiremock` for HTTP mocking in integration tests
- `#[tokio::test]` for async tests
- Tests in `tests/` directory, not inline

### Test Files

| File | Coverage |
|------|----------|
| `artful_test.rs` | Artful methods, instance accessors, HTTP errors, plugin error propagation |
| `direction_test.rs` | DirectionKind, Destination, custom Direction |
| `flow_ctrl_test.rs` | FlowCtrl::skip_rest, empty chain |
| `rocket_test.rs` | Rocket convenience methods, RocketConfig, ClientOptions/RequestOptions |
| `integration_test.rs` | Full pipeline, Content-Type auto-header, client timeout |
| `packer_test.rs` | JsonPacker pack/unpack |
| `shortcut_test.rs` | Shortcut trait |

## Gotchas

1. **Crate name vs struct name**: Crate is `artisan_http`, main struct is `Artful`
   ```rust
   use artisan_http::Artful;  // Correct
   ```

2. **Plugin error propagation**: Use `?` after `next.call(rocket).await`
   ```rust
   next.call(rocket).await?;  // Required - not just .await
   ```

3. **DirectionKind enum**: `Json`, `Response`, `NoRequest`, `Custom`

4. **Rocket params vs payload**: `params` immutable, `payload` mutable by plugins

5. **No binary**: Library crate only, examples for demo
