[English](./README.md) | 简体中文

# artisan-http

> Api RequesT Framework U Like - 你喜欢的 Rust API 请求框架

基于洋葱模型的 Rust HTTP 客户端框架，灵感来自 [yansongda/artful](https://github.com/yansongda/artful)。

## 特性

- 🔄 **洋葱模型**: 请求层层穿透，响应层层返回
- 🔌 **插件化**: 每个请求都是一个插件组合，高度灵活可定制
- 🛡️ **类型安全**: Rust 类型系统确保配置和参数的类型安全
- ⚡ **高性能**: 实例化 `Artful`，`reqwest::Client` 内部 `Arc` 共享连接池
- 📦 **Content-Type 自动补头**: JSON 请求默认自动携带 `Content-Type: application/json`（仅缺失时补，用户显式设置不覆盖）

## 安装

```bash
cargo add artisan-http
```

```toml
[dependencies]
artisan-http = "0.16.0"
```

## 快速开始

### 基础使用

```rust
use artisan_http::{Artful, Plugin, Rocket, flow_ctrl::Next};
use artisan_http::plugins::{StartPlugin, AddPayloadBodyPlugin, AddRadarPlugin};
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
    ];

    let artful = Artful::new()?;
    let result = artful.artful(params, plugins).await?;
    
    if let artisan_http::Destination::Json(json) = result {
        println!("Response: {}", json);
    }

    Ok(())
}
```

### 使用 Shortcut 快捷方式

```rust
use artisan_http::{Artful, Shortcut, Plugin};
use artisan_http::plugins::{StartPlugin, AddPayloadBodyPlugin, AddRadarPlugin};
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

### 全局单例（LazyLock）

`Artful` 是实例类型（`reqwest::Client` 内部 `Arc`，`Clone` 廉价且共享连接池）。应用层推荐用 `std::sync::LazyLock` 构建全局单例，首访时初始化（可读环境变量）：

```rust
use std::sync::LazyLock;

static ARTFUL: LazyLock<Artful> = LazyLock::new(|| {
    Artful::with_config(load_config()).expect("failed to build Artful client")
});

// 零 panic 变体（ArtfulError 非 Clone，调用点需 map_err 转移错误所有权）
static ARTFUL: LazyLock<Result<Artful, ArtfulError>> =
    LazyLock::new(|| Artful::with_config(load_config()));
// 调用点：
// let artful = ARTFUL.as_ref().map_err(|e| ArtfulError::Other(format!("Artful init failed: {e}")))?;

// 多实例：不同渠道各一个 static，连接池独立
static ALIPAY: LazyLock<Artful> = /* ... */;
static WECHAT: LazyLock<Artful> = /* ... */;
```

### 自定义 HTTP 客户端

`ClientOptions` 仅覆盖常用选项（timeout / connect_timeout / 连接池 / User-Agent）。按 client 控制权从低到高，四个构造函数按需选择：

```rust
// ① with_client_builder（推荐）：以 config.http 为基座，回调叠加 ClientOptions 表达不了的能力；
//    回调内后写的 setter 覆盖框架默认值（如覆盖默认 UA）
let artful = Artful::with_client_builder(config, |builder| {
    builder
        .proxy(reqwest::Proxy::all("http://corp-proxy:8080")?)
        .cookie_store(true)
})?;

// ② with_client：注入外部构建的 client（跨 Artful 实例共享连接池时使用）；
//    config.http 不作用于注入的 client，仅作为配置记录（可经 artful.config() 读取）
let custom = reqwest::Client::builder()
    .proxy(reqwest::Proxy::all("http://corp-proxy:8080")?)
    .cookie_store(true)
    .build()?;
let artful = Artful::with_client(Config::default(), custom);

// ③ builder（链式）：config / customize / client 可选叠加后统一 build；
//    一旦设置 .client()，config.http 与 customize 均不参与构建
let artful = Artful::builder()
    .config(config)
    .customize(|builder| builder.cookie_store(true))
    .build()?;

let result = artful.shortcut(MyApiShortcut, params).await?;
```

绝大多数场景用 `Artful::new()` / `Artful::with_config(config)` 即可（框架全托管）。

### 自定义插件

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

**错误处理**: 插件返回 `Result<()>`，任一插件失败会终止整个链并传播错误。

### 事件系统

每个 `Artful` 实例内置事件分发器：经 builder 注册监听器，即可观测请求生命周期，无需编写完整插件（未注册监听器时零开销）。

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
        Ok(()) // 旁路监听器：内部消化错误，永不返回 Err
    }
}

let artful = Artful::builder()
    .event_listener(Arc::new(LoggingListener))
    .build()?;
```

| 事件 | 触发时机 | 可变性 |
|------|---------|--------|
| `ArtfulStart` | 插件链启动前 | 只读 |
| `HttpStart` | HTTP 请求即将发出、到达链尾核心动作执行点（`IgniteCore`，由框架自动挂载；正常链中 radar 已构建；链中缺 `AddRadarPlugin` 时为 `None`，事件仍触发；须经 `rocket.radar` 的 `*_mut` 访问器修改请求） | 可变 |
| `HttpEnd` | 请求成功返回、解析前（响应体不可读：body 消费权属于 direction 解析，仅可读 status / headers） | 只读 |
| `HttpError` | HTTP 请求执行失败（错误照常向上传播） | 只读 |
| `ArtfulEnd` | 链成功后、返回 destination 前（可改写 `rocket.destination`） | 可变 |

> - 监听器是**同步**回调，必须非阻塞——耗时任务请自行 `tokio::spawn`。
> - 监听器返回 `Err` 会中止主流程（以 `EventListenerError` 传播）。旁路监听器应内部消化错误、恒返回 `Ok(())`。
> - `HttpEnd` 无法读取响应体（消费权属于 direction 解析），该事件下仅可读 status / headers。

试一试：`cargo run -p artisan-http --example event`。

## 核心概念

### Rocket - 请求载体

`Rocket` 是整个请求生命周期中的数据载体：

```rust
pub struct Rocket {
    params: HashMap<String, Value>,   // 原始参数（不变）
    pub payload: HashMap<String, Value>, // 业务参数（可修改）
    pub config: RocketConfig,         // HTTP 配置（可修改）
    pub radar: Option<Request>,       // HTTP 请求对象
    pub destination: Option<Destination>, // 解析结果
    pub packer: Arc<dyn Packer>,      // 序列化器
}
```

**设计说明**：
- `params`: 原始参数，由调用方传入，整个生命周期中保持不变
- `payload`: 业务参数，由 `StartPlugin` 从 `params` 初始化，后续插件可修改
- `config`: HTTP 配置，包含 `direction`（响应解析策略），由插件负责设置

### RocketConfig - 请求配置

```rust
pub struct RocketConfig {
    pub method: reqwest::Method,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub http: RequestOptions,        // 请求级选项（仅 timeout）
    pub direction: DirectionKind,     // 响应解析策略
}
```

### Plugin - 插件（洋葱模型）

插件是洋葱模型的核心，每个插件可以在请求前向和后向阶段执行操作：

```rust
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()>;
}
```

执行流程：
```
请求 → Plugin1 前向 → Plugin2 前向 → Plugin3 前向 → HTTP 请求
响应 ← Plugin1 后向 ← Plugin2 后向 ← Plugin3 后向 ← HTTP 响应
```

### Direction - 响应解析策略

```rust
pub enum DirectionKind {
    Json,             // 解析为 JSON（默认）
    Response,         // 返回原始 Response
    NoRequest,        // 不发起 HTTP 请求
    Custom(Arc<dyn Direction>), // 自定义解析器
}
```

## 内置插件

| 插件 | 功能 |
|------|------|
| `StartPlugin` | 将 params 初始化到 payload |
| `AddPayloadBodyPlugin` | 将 payload 序列化为请求体 |
| `AddRadarPlugin` | 构建 HTTP Request |

> HTTP 执行与响应解析由框架内置链尾核心动作 `IgniteCore` 自动完成（经 `Artful::artful` / `Artful::shortcut` 自动挂载），插件链无需也不可再挂解析插件。插件链最小形态为 `[StartPlugin, ..., AddRadarPlugin]`。
>
> **从 0.15.x 迁移**：从插件链中删除解析插件一项即可（该类型已删除，老代码将编译失败；旧类型名见 CHANGELOG 0.16.0 条目）。注意：链中位于原解析插件之后的插件，其 `next.call` 之前的逻辑（前向阶段）现运行于请求执行之前（destination / destination_origin 尚为 None、radar 未消费；后向阶段不受影响），此类链型需复核。详见 ARCHITECTURE.md §3.4 事件系统。

## 示例

```bash
# 运行示例
cargo run -p artisan-http --example basic
cargo run -p artisan-http --example config
cargo run -p artisan-http --example shortcut
cargo run -p artisan-http --example custom_plugin
cargo run -p artisan-http --example direction
```

## 测试

```bash
# 运行所有测试
cargo test -p artisan-http --all-features
```

## 文档

- 详细架构设计：[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- 项目说明：[AGENTS.md](AGENTS.md)

## 许可证

MIT License
