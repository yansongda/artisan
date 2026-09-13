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
artisan-http = "0.17.0"
```

## Quick Start

### Basic Usage

```rust
use artisan_http::{Artful, Plugin, Rocket, flow_ctrl::Next};
use artisan_http::plugins::{ParserPlugin, StartPlugin, AddPayloadBodyPlugin, AddRadarPlugin};
use async_trait::async_trait;
use std::sync::Arc;
use serde_json::{Map, json};

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
    let params = Map::from_iter([
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
use artisan_http::plugins::{ParserPlugin, StartPlugin, AddPayloadBodyPlugin, AddRadarPlugin};
use std::sync::Arc;
use serde_json::Map;

#[derive(Default)]
struct MyApiShortcut {
    method: reqwest::Method,
    url: String,
}

impl Shortcut for MyApiShortcut {
    fn get_plugins(&self, _params: &Map<String, serde_json::Value>) 
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
let result = artful.shortcut(shortcut, Map::new()).await?;
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
| `HttpStart` | before the HTTP request is sent, at the tail core action execution point (`IgniteCore`, mounted automatically by the framework; radar already built in a normal chain; `None` if the chain lacks `AddRadarPlugin` - the event still fires; mutate the request via the `*_mut` accessors on `rocket.radar`) | mutable |
| `HttpEnd` | after a successful response, before parsing (response body is NOT readable - body consumption belongs to direction parsing; only status / headers are readable) | read-only |
| `HttpError` | when the HTTP request execution fails (the error still propagates) | read-only |
| `ArtfulEnd` | after the chain succeeds, before returning the destination (may rewrite `rocket.destination`) | mutable |

> - Listeners are **synchronous** and must be non-blocking — spawn heavy work yourself (`tokio::spawn`).
> - A listener returning `Err` aborts the main flow (propagates as `EventListenerError`). Bypass listeners should consume errors internally and always return `Ok(())`.
> - `HttpEnd` cannot read the response body (ownership belongs to direction parsing); only status / headers are available there.

Try it: `cargo run -p artisan-http --example event`.

### Typed Helpers

0.18.0 adds a typed convenience layer on top of the plain `Value`-based flow, so business structs can go straight in and out:

```rust
use artisan_http::{Artful, Destination, pack_typed, unpack_typed};
use serde_json::Map;

// Serialize a business struct into the request body (packer-level, dyn-compatible)
let body = pack_typed(packer.as_ref(), &order, &params)?;

// Deserialize a response body into a business struct
let order: OrderResp = unpack_typed(packer.as_ref(), &body, &params)?;

// Destination → Value (`Json` direction; `Response`/`None` yield `DestinationMismatch`)
let value: serde_json::Value = destination.into_json()?;

// Strongly-typed entry point: artful() → into_json() → from_value::<T>
let order: OrderResp = artful.artful_as(params, plugins).await?;
```

Error handling: `pack_typed` reuses `JsonSerializeError` on serialization failure and `InvalidParameter` when the input does not serialize to a JSON object; `unpack_typed` reuses `JsonDeserializeError` (message names the target type); `into_json` / `artful_as` return the new `DestinationMismatch` variant when the destination is `Response` or `None`.

## Core Concepts

### Rocket - The Request Carrier

`Rocket` is the data carrier throughout the request lifecycle:

```rust
pub struct Rocket {
    params: Map<String, Value>,   // raw params (immutable)
    pub payload: Map<String, Value>, // business params (mutable)
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
| `ParserPlugin` | Parses the response into `destination` (must be the last entry of the chain) |

> HTTP execution is handled by the framework's built-in tail core action `IgniteCore`, mounted automatically by `Artful::artful` / `Artful::shortcut`. Response parsing is the job of `ParserPlugin`: **the plugin chain must include `ParserPlugin` as its last entry** - if you forget it, the request is still sent but nothing is parsed (`rocket.destination` stays `None`). The minimal chain shape is `[StartPlugin, ..., AddRadarPlugin, ParserPlugin]`.
>
> **Migrating from 0.16.0**: append `Arc::new(ParserPlugin)` to the end of your plugin chain. Also note `Packer::pack` / `Packer::unpack` now take an extra `params: &HashMap<String, Value>` argument (custom `Packer` implementations just add the parameter, usually ignored), and `JsonDirection` unpacks the response body via `rocket.packer.unpack` (default path unchanged; with `rocket.packer` set to `XmlPacker`, responses are unpacked as XML). See the CHANGELOG 0.17.0 entry for details.

### Migrating from 0.17.0

0.18.0 migrates the payload data domain from the old `HashMap` to `serde_json::Map<String, Value>` (BTreeMap-backed without the `preserve_order` feature; keys sort lexicographically). Six public APIs changed signature, and one impl was removed:

- `Packer::pack` / `Packer::unpack`: `params` parameter `&HashMap` → `&Map<String, Value>`
- `Rocket::new` / `payload` / `get_params`: `HashMap` → `Map<String, Value>`
- `Artful::artful` / `shortcut`: `params` → `Map<String, Value>`
- `Shortcut::get_plugins`: parameter → `&Map<String, Value>`
- `filter_params`: → `Map<String, Value>`
- `Event::ArtfulStart.params`: → `&Map<String, Value>`
- Removed: `From<HashMap> for Rocket` (construct via `Rocket::new(Map::new())` / `Rocket::new(Map::from_iter([...]))` instead)

Migration recipe:

```rust
// Old (0.17.x)
use std::collections::HashMap;
let params = HashMap::from([("order_id".to_string(), json!("123"))]);
let rocket = Rocket::new(params);

// New (0.18.0) — note serde_json::Map has no From<[(K,V); N]>, use from_iter
use serde_json::Map;
let params: Map<String, Value> = Map::from_iter([("order_id".to_string(), json!("123"))]);
let rocket = Rocket::new(params);

// Custom Packer migration: only the parameter type changes
// fn pack(&self, data: &Map<String, Value>, _params: &Map<String, Value>) -> Result<String>

// Typed entry point (new capability)
let order: OrderResp = artful.artful_as(params, plugins).await?;
```

Also note: `JsonPacker` output key order changed from random to lexicographic, packer is now positioned as **request-level configuration set early in the chain (no mid-chain replacement promise)**, and the new `DestinationMismatch` error variant requires an extra arm in exhaustive `ArtfulError` matches. See the CHANGELOG 0.18.0 entry for the full list.

### Built-in Packers & Directions

| Type | Kind | Purpose |
|------|------|---------|
| `JsonPacker` | Packer | JSON serialization (default) |
| `QueryPacker` | Packer | form-urlencoded (WHATWG URL Standard); `unpack` supports the raw mode via `_unpack_raw` |
| `XmlPacker` | Packer | CDATA-format XML (quick-xml based); leaf text stays `String` |
| `JsonDirection` | Direction | Parses via `rocket.packer.unpack` (default) |
| `NoHttpRequestDirection` | Direction | `NoRequest`: no HTTP request is sent |
| `OriginResponseDirection` | Direction | `Response`: returns the raw response |

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
