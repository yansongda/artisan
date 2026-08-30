English | [简体中文](./README.zh-CN.md)

# artisan-http

> Api RequesT Framework U Like - The Rust API request framework you like

A Rust HTTP client framework based on the onion model, inspired by [yansongda/artful](https://github.com/yansongda/artful).

## Features

- 🔄 **Onion model**: requests pass through layer by layer, responses return layer by layer
- 🔌 **Plugin-based**: every request is a composition of plugins, highly flexible and customizable
- 🛡️ **Type safety**: Rust's type system keeps configuration and parameters type-safe
- ⚡ **High performance**: instantiating `Artful` shares the `reqwest::Client` connection pool internally via `Arc`
- 📦 **Automatic Content-Type**: JSON requests automatically carry `Content-Type: application/json` (only added when missing; user-set headers are never overwritten)

## Installation

```bash
cargo add artisan-http
```

```toml
[dependencies]
artisan-http = "0.16.0"
```

## Quick Start

### Basic Usage

```rust
use artisan_http::{Artful, Plugin, Rocket, flow_ctrl::Next};
use artisan_http::plugins::{StartPlugin, AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin};
use async_trait::async_trait;
use std::sync::Arc;
use std::collections::HashMap;
use serde_json::json;

struct MethodUrlPlugin {
    method: reqwest::Method,
    url: String,
}

#[async_trait]
impl Plugin for MethodUrlPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.config.method = self.method.clone();
        rocket.config.url = self.url.clone();
        next.call(rocket).await
    }
}

#[tokio::main]
async fn main() -> artisan_http::Result<()> {
    let params = HashMap::from([
        ("order_id".to_string(), json!("123")),
        ("amount".to_string(), json!(100)),
    ]);

    let plugins: Vec<Arc<dyn artisan_http::Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: "https://api.example.com/orders".to_string(),
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new()?;
    let result = artful.artful(params, plugins).await?;
    
    if let artisan_http::Destination::Json(json) = result {
        println!("Response: {}", json);
    }

    Ok(())
}
```

### Using Shortcuts

```rust
use artisan_http::{Artful, Shortcut, Plugin};
use artisan_http::plugins::{StartPlugin, AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin};
use std::sync::Arc;
use std::collections::HashMap;

#[derive(Default)]
struct MyApiShortcut {
    method: reqwest::Method,
    url: String,
}

impl Shortcut for MyApiShortcut {
    fn get_plugins(&self, _params: &HashMap<String, serde_json::Value>) 
        -> Vec<Arc<dyn Plugin>> 
    {
        vec![
            Arc::new(StartPlugin),
            Arc::new(MethodUrlPlugin {
                method: self.method.clone(),
                url: self.url.clone(),
            }),
            Arc::new(AddPayloadBodyPlugin),
            Arc::new(AddRadarPlugin),
            Arc::new(ParserPlugin),
        ]
    }
}

let shortcut = MyApiShortcut {
    method: reqwest::Method::POST,
    url: "https://api.example.com/orders".to_string(),
};
let artful = Artful::new()?;
let result = artful.shortcut(shortcut, HashMap::new()).await?;
```

### Global Singleton (LazyLock)

`Artful` is an instance type (the `reqwest::Client` is wrapped in an `Arc` internally, so `Clone` is cheap and shares the connection pool). At the application layer, prefer building a global singleton with `std::sync::LazyLock`, initialized on first access (where environment variables can be read):

```rust
use std::sync::LazyLock;

static ARTFUL: LazyLock<Artful> = LazyLock::new(|| {
    Artful::with_config(load_config()).expect("failed to build Artful client")
});

// Zero-panic variant (ArtfulError is not Clone; call sites need map_err to transfer ownership)
static ARTFUL: LazyLock<Result<Artful, ArtfulError>> =
    LazyLock::new(|| Artful::with_config(load_config()));
// At the call site:
// let artful = ARTFUL.as_ref().map_err(|e| ArtfulError::Other(format!("Artful init failed: {e}")))?;

// Multiple instances: one static per channel, independent connection pools
static ALIPAY: LazyLock<Artful> = /* ... */;
static WECHAT: LazyLock<Artful> = /* ... */;
```

### Customizing the HTTP Client

`ClientOptions` only covers the common options (timeout / connect_timeout / connection pool / User-Agent). Ordered from least to most client control, pick one of the four constructors as needed:

```rust
// ① with_client_builder (recommended): builds on top of config.http, with the callback layering on
//    capabilities that ClientOptions cannot express;
//    setters written later inside the callback override the framework defaults (e.g. overriding the default UA)
let artful = Artful::with_client_builder(config, |builder| {
    builder
        .proxy(reqwest::Proxy::all("http://corp-proxy:8080")?)
        .cookie_store(true)
})?;

// ② with_client: inject an externally built client (use it to share a connection pool across Artful instances);
//    config.http does not apply to the injected client and is kept only as a configuration record (readable via artful.config())
let custom = reqwest::Client::builder()
    .proxy(reqwest::Proxy::all("http://corp-proxy:8080")?)
    .cookie_store(true)
    .build()?;
let artful = Artful::with_client(Config::default(), custom);

// ③ builder (chainable): accumulate config / customize / client optionally, then build;
//    once .client() is set, neither config.http nor customize participates in the build
let artful = Artful::builder()
    .config(config)
    .customize(|builder| builder.cookie_store(true))
    .build()?;

let result = artful.shortcut(MyApiShortcut, params).await?;
```

For most scenarios, `Artful::new()` / `Artful::with_config(config)` is all you need (fully managed by the framework).

### Custom Plugins

```rust
use artisan_http::{Plugin, Rocket, flow_ctrl::Next};
use async_trait::async_trait;

pub struct SignaturePlugin {
    api_key: String,
}

#[async_trait]
impl Plugin for SignaturePlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.config.headers.insert(
            "X-Signature".to_string(),
            sign(&self.api_key, &rocket.payload),
        );
        
        next.call(rocket).await
    }
}
```

**Error handling**: plugins return `Result<()>`; any plugin failure aborts the whole chain and propagates the error.

### Events

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
| `HttpStart` | before the HTTP request is sent (radar already built; mutate the request via the `*_mut` accessors on `rocket.radar`) | mutable |
| `HttpEnd` | after a successful response, before parsing | read-only |
| `HttpError` | when the HTTP request execution fails (the error still propagates) | read-only |
| `ArtfulEnd` | after the chain succeeds, before returning the destination (may rewrite `rocket.destination`) | mutable |

> - Listeners are **synchronous** and must be non-blocking — spawn heavy work yourself (`tokio::spawn`).
> - A listener returning `Err` aborts the main flow (propagates as `EventListenerError`). Bypass listeners should consume errors internally and always return `Ok(())`.

Try it: `cargo run -p artisan-http --example event`.

## Core Concepts

### Rocket - The Request Carrier

`Rocket` is the data carrier throughout the request lifecycle:

```rust
pub struct Rocket {
    params: HashMap<String, Value>,   // raw params (immutable)
    pub payload: HashMap<String, Value>, // business params (mutable)
    pub config: RocketConfig,         // HTTP config (mutable)
    pub radar: Option<Request>,       // the HTTP request object
    pub destination: Option<Destination>, // parsed result
    pub packer: Arc<dyn Packer>,      // serializer
}
```

**Design notes**:
- `params`: the raw parameters passed in by the caller, unchanged throughout the lifecycle
- `payload`: the business parameters, initialized from `params` by `StartPlugin`, modifiable by later plugins
- `config`: the HTTP configuration, including `direction` (the response parsing strategy), set by plugins

### RocketConfig - Request Configuration

```rust
pub struct RocketConfig {
    pub method: reqwest::Method,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub http: RequestOptions,        // request-level options (timeout only)
    pub direction: DirectionKind,     // response parsing strategy
}
```

### Plugin - The Onion Model

Plugins are the core of the onion model. Each plugin can perform operations in the forward (before) and backward (after) phases of a request:

```rust
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()>;
}
```

Execution flow:
```
Request  → Plugin1 forward → Plugin2 forward → Plugin3 forward → HTTP request
Response ← Plugin1 backward ← Plugin2 backward ← Plugin3 backward ← HTTP response
```

### Direction - Response Parsing Strategy

```rust
pub enum DirectionKind {
    Json,             // Parse as JSON (default)
    Response,         // Return the raw Response
    NoRequest,        // Do not send an HTTP request
    Custom(Arc<dyn Direction>), // Custom parser
}
```

## Built-in Plugins

| Plugin | Purpose |
|--------|---------|
| `StartPlugin` | Initializes `payload` from `params` |
| `AddPayloadBodyPlugin` | Serializes the payload into the request body |
| `AddRadarPlugin` | Builds the HTTP Request |
| `ParserPlugin` | Sends the request and parses the response |

## Examples

```bash
# Run an example
cargo run -p artisan-http --example basic
cargo run -p artisan-http --example config
cargo run -p artisan-http --example shortcut
cargo run -p artisan-http --example custom_plugin
cargo run -p artisan-http --example direction
```

## Testing

```bash
# Run all tests
cargo test -p artisan-http --all-features
```

## Documentation

- Architecture design in detail: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Project guide: [AGENTS.md](AGENTS.md)

## License

MIT License
