# Artful-Rs 架构设计文档

> Api RequesT Framework U Like - 你喜欢的 Rust API 请求框架
> 
> 基于 [yansongda/artful](https://github.com/yansongda/artful) 的架构理念，使用 Rust 实现的 HTTP 客户端框架

## 一、设计理念

### 1.1 核心原则

- **洋葱模型**: 所有请求处理通过 Pipeline（插件链）实现，请求层层穿透，响应层层返回
- **插件化**: 每个请求都是一个插件组合，高度灵活可定制
- **类型安全**: 使用 Rust 类型系统确保配置和参数的类型安全
- **符合标准**: 遵循 Rust async/await 最佳实践

### 1.2 与 PHP 版本的对比

| 特性 | PHP (yansongda/artful) | Rust (artful-rs) |
|------|------------------------|------------------|
| 洋葱模型 | Pipeline + Closure | FlowCtrl + async |
| 数据载体 | Rocket | Rocket |
| 配置参数 | `_` 开头参数在 HashMap | RocketConfig struct（类型安全） |
| 插件 | PluginInterface | Plugin trait |
| HTTP 客户端 | Guzzle | reqwest |
| 序列化 | JsonPacker | serde_json |
| 类型系统 | 动态类型 | 静态类型 + 泛型 |

---

## 二、核心概念

### 2.1 Rocket - 请求载体

Rocket 是整个请求生命周期中的数据载体。

```rust
/// 请求载体 - 携带整个请求生命周期中的所有数据
pub struct Rocket {
    /// 原始参数（不变）
    params: HashMap<String, Value>,
    
    /// 业务参数（动态）
    pub payload: HashMap<String, Value>,
    
    /// Rocket 配置（可修改）
    pub config: RocketConfig,
    
    /// HTTP 客户端（由 `Artful` 实例注入，默认为框架内置客户端）
    pub client: reqwest::Client,
    
    /// HTTP 请求对象（最终发送的请求）
    pub radar: Option<reqwest::Request>,
    
    /// HTTP 原始响应
    pub destination_origin: Option<reqwest::Response>,
    
    /// 最终解析结果
    pub destination: Option<Destination>,
    
    /// 序列化器
    pub packer: Arc<dyn Packer>,
}
```

**设计说明**：
- `params` - 原始参数，整个生命周期中保持不变
- `payload` - 业务参数，动态 HashMap
- `config` - 请求配置，包含 method、url、headers、direction 等
- `client` - HTTP 客户端，由 `Artful` 实例注入，插件不再依赖全局单例
- `radar` - 最终构建的 HTTP Request
- `destination_origin` - HTTP 响应
- `destination` - 解析后的结果
- `packer` - 序列化器

### 2.2 RocketConfig - 配置参数

RocketConfig 将配置参数封装为 struct，提供类型安全和 IDE 类型提示。所有字段可在 plugin 中动态修改。

```rust
/// Rocket 配置（所有字段可在 plugin 中动态修改）
#[derive(Debug, Clone)]
pub struct RocketConfig {
    /// HTTP 方法（默认 POST，可动态修改）
    pub method: reqwest::Method,
    
    /// 请求 URL（必填，可动态修改，如添加 query 参数）
    pub url: String,
    
    /// 请求头（可动态添加/修改）
    pub headers: HashMap<String, String>,
    
    /// 请求体（可动态设置）
    pub body: Option<String>,
    
    /// 请求级 HTTP 选项（仅 timeout，单次请求生效）
    pub http: RequestOptions,
    
    /// 响应解析策略（默认 Json，可动态修改）
    pub direction: DirectionKind,
}

/// 客户端级 HTTP 选项（仅在构建 `reqwest::Client` 时生效，如 `Artful::with_config`）
#[derive(Debug, Clone, Default)]
pub struct ClientOptions {
    /// 请求超时（秒），请求级可覆盖
    pub timeout: Option<u64>,
    
    /// 连接超时（秒）
    pub connect_timeout: Option<u64>,
    
    /// 连接池空闲连接超时（秒），默认 90
    pub pool_idle_timeout: Option<u64>,
    
    /// 每个 host 最大空闲连接数，默认 20
    pub pool_max_idle_per_host: Option<usize>,
    
    /// User-Agent，默认 yansongda/artisan-http:{version}
    pub user_agent: Option<String>,
}

/// 请求级 HTTP 选项（仅对单次请求生效）
///
/// 仅含 `timeout`：reqwest 0.13 的 `RequestBuilder` 不提供 `connect_timeout`
/// （该方法仅 `ClientBuilder` 支持），连接超时收敛为 client 级专属。
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestOptions {
    /// 请求超时（秒）
    pub timeout: Option<u64>,
}
```

**与 PHP 版本的对应关系**：

| PHP `_` 参数 | Rust RocketConfig 字段 |
|-------------|----------------------|
| `_method` | `config.method` |
| `_url` | `config.url` |
| `_headers` | `config.headers` |
| `_body` | `config.body` |
| `_http.timeout` | `config.http.timeout` |
| `_direction` | `config.direction` |

**优势**：
- 类型安全：字段类型明确，编译时检查
- IDE 类型提示：自动补全、类型提示
- 清晰分离：配置参数与业务参数分离
- 生命周期分离：请求级选项（`RequestOptions`）与 client 级选项（`ClientOptions`）按类型拆分，per-request 误设 pool 字段为编译错误

### 2.3 Config - 框架配置

Config 是框架级配置，在构造 `Artful` 实例时显式传入，支持任意扩展参数。

```rust
/// 框架配置
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// HTTP 客户端级默认选项
    pub http: ClientOptions,
    
    /// 扩展配置：支持任意渠道/模块参数
    pub extra: HashMap<String, Value>,
}
```

**extra 字段用途**：
- 存储任意渠道配置（如支付宝、微信支付配置）
- 支持动态扩展，无需修改 Config 结构
- 与 PHP 版本的灵活配置模式兼容

**使用示例**：

```rust
use artisan_http::{Artful, ClientOptions, Config};
use serde_json::json;
use std::collections::HashMap;

let mut extra = HashMap::new();
extra.insert("alipay".to_string(), json!({
    "app_id": "2016082000295641",
    "app_secret_cert": "...",
}));
extra.insert("wechat".to_string(), json!({
    "mch_id": "...",
    "mch_secret_key": "...",
}));

let config = Config {
    http: ClientOptions {
        timeout: Some(5),
        connect_timeout: Some(3),
        ..Default::default()
    },
    extra,
};

// 构造时即构建 HTTP 客户端（fail-fast，配置错误立即暴露）
let artful = Artful::with_config(config)?;

// 后续经实例读取配置
if let Some(alipay) = artful.config().extra.get("alipay") {
    let app_id = alipay.get("app_id");
}
```

**应用层全局单例（LazyLock 推荐）**：

```rust
use std::sync::LazyLock;

// 首访初始化，可读环境变量
static ARTFUL: LazyLock<Artful> = LazyLock::new(|| {
    Artful::with_config(load_config()).expect("failed to build Artful client")
});

// 零 panic 变体
static ARTFUL: LazyLock<Result<Artful, ArtfulError>> =
    LazyLock::new(|| Artful::with_config(load_config()));

// 多实例：不同渠道各一个 static，连接池独立
static ALIPAY: LazyLock<Artful> = /* ... */;
static WECHAT: LazyLock<Artful> = /* ... */;
```

### 2.4 Plugin - 插件

插件是洋葱模型的核心。

```rust
/// 插件 trait - 洋葱模型核心
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// 组装请求
    /// 
    /// # Arguments
    /// * `rocket` - 请求载体，包含所有数据
    /// * `next` - 下一个插件（闭包）
    /// 
    /// # Returns
    /// * `Result<()>` - 成功或错误，错误会终止整个插件链
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()>;
}

/// 下一个插件的闭包（洋葱穿透）
pub struct Next<'a> {
    ctrl: &'a mut FlowCtrl,
}

impl<'a> Next<'a> {
    /// 调用下一个插件
    /// 
    /// # Returns
    /// * `Result<()>` - 后续插件的结果，用 `?` 传播错误
    pub async fn call(self, rocket: &mut Rocket) -> Result<()> {
        self.ctrl.call_next(rocket).await
    }
}
```

**插件编写模式**:

```rust
pub struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()> {
        // ===== 前向逻辑 =====
        // 修改 config、payload 等
        
        rocket.config.headers.insert(
            "Authorization".to_string(),
            "Bearer token".to_string()
        );
        
        // ===== 调用下一层 =====
        next.call(rocket).await?;  // 用 ? 传播后续插件错误
        
        // ===== 后向逻辑 =====
        // 处理响应、错误处理等
        // 只有前面都成功才会执行到这里
        
        Ok(())
    }
}
```

**错误传播**: 任一插件返回 `Err` 会终止整个链，错误向上传播到 `Artful::artful`。

### 2.5 FlowCtrl - 流向控制器

FlowCtrl 控制洋葱模型的执行流程，借鉴 [salvo](https://github.com/salvo-rs/salvo) 的设计。

```rust
/// 洋葱模型流向控制器
pub struct FlowCtrl {
    /// 当前执行位置
    cursor: usize,
    
    /// 插件列表（线性排列）
    plugins: Vec<Arc<dyn Plugin>>,
    
    /// 是否已终止
    is_ceased: bool,
}

impl FlowCtrl {
    /// 创建新的流向控制器
    pub fn new(plugins: Vec<Arc<dyn Plugin>>) -> Self {
        Self {
            cursor: 0,
            plugins,
            is_ceased: false,
        }
    }
    
    /// 调用下一层插件（洋葱穿透）
    pub async fn call_next(&mut self, rocket: &mut Rocket) -> Result<()> {
        if self.is_ceased || !self.has_next() {
            return Ok(());  // 正常结束
        }
        
        let plugin = self.plugins[self.cursor].clone();
        self.cursor += 1;
        
        let next = Next { ctrl: self };
        plugin.assembly(rocket, next).await  // 传播错误
    }
    
    /// 检查是否还有下一层
    pub fn has_next(&self) -> bool {
        self.cursor < self.plugins.len()
    }
    
    /// 跳过剩余所有插件
    pub fn skip_rest(&mut self) {
        self.cursor = self.plugins.len();
        self.is_ceased = true;
    }
    
    /// 检查是否已终止
    pub fn is_ceased(&self) -> bool {
        self.is_ceased
    }
}
```

**执行流程示意**:

```
插件列表: [Start, Sign, AddRadar, Parser]

执行顺序（洋葱模型）:
┌─────────────────────────────────────────────────────────┐
│ Start.assembly()                                        │
│   ├─ 前向逻辑: 初始化                                    │
│   ├─ next.call()                                        │
│   │   └─────────────────────────────────────────────────│
│   │   │ Sign.assembly()                                 │
│   │   │   ├─ 前向逻辑: 添加签名                          │
│   │   │   ├─ next.call()                                │
│   │   │   │   └─────────────────────────────────────────│
│   │   │   │   │ AddRadar.assembly()                     │
│   │   │   │   │   ├─ 前向逻辑: 构建 Request              │
│   │   │   │   │   ├─ next.call()                        │
│   │   │   │   │   │   ┌─────────────────────────────────│
│   │   │   │   │   │   │ Parser.assembly()               │
│   │   │   │   │   │   │   ├─ 前向: 无                   │
│   │   │   │   │   │   │   ├─ HTTP 请求执行              │
│   │   │   │   │   │   │   ├─ 后向: 解析响应             │
│   │   │   │   │   │   └─────────────────────────────────│
│   │   │   │   │   ├─ 后向逻辑: 无                       │
│   │   │   │   └─────────────────────────────────────────│
│   │   │   ├─ 后向逻辑: 验签（可选）                      │
│   │   │   └─────────────────────────────────────────────│
│   │   └─────────────────────────────────────────────────│
│   ├─ 后向逻辑: 日志记录等                                │
│   └─────────────────────────────────────────────────────┘
```

### 2.6 Shortcut - 快捷方式

Shortcut 是一系列插件的组合，方便快速调用特定 API。

```rust
/// 快捷方式 trait（dyn compatible，支持 trait object）
pub trait Shortcut {
    /// 返回插件列表
    fn get_plugins(&self, params: &HashMap<String, Value>) 
        -> Vec<Arc<dyn Plugin>>;
}

// 示例实现
pub struct QueryOrderShortcut {
    base_url: String,
}

impl Shortcut for QueryOrderShortcut {
    fn get_plugins(&self, _params: &HashMap<String, Value>) 
        -> Vec<Arc<dyn Plugin>> 
    {
        vec![
            Arc::new(StartPlugin),
            Arc::new(QueryOrderPlugin {  // 可携带状态
                url: format!("{}{}", self.base_url, "/query"),
            }),
            Arc::new(AddSignaturePlugin),
            Arc::new(AddRadarPlugin),
            Arc::new(ParserPlugin),
        ]
    }
}
```

**设计优势**：
- **Dyn compatible**：支持 `Box<dyn Shortcut>` 或 `&dyn Shortcut`
- **携带状态**：Shortcut struct 可存储配置（如 base_url、api_key）
- **灵活构造**：无需 `Default` bound，可在任意上下文中创建实例

---

## 三、核心模块设计

### 3.1 Artful - 主入口

Artful 是实例类型：配置与 HTTP 客户端在构造时显式解析（fail-fast），支持多实例共存与测试隔离。

```rust
/// Artful 主类 - 框架入口
#[derive(Debug, Clone)]
pub struct Artful {
    config: Config,
    client: reqwest::Client,
}

impl Artful {
    /// 以默认配置创建实例
    pub fn new() -> Result<Self>;

    /// 以指定配置创建实例（构造时构建 client，失败返回 ClientBuildError）
    pub fn with_config(config: Config) -> Result<Self>;

    /// 以指定配置与自定义构建流程创建实例
    /// （先按 config.http 应用框架默认值，回调叠加，后写 setter 覆盖先写值）
    pub fn with_client_builder(
        config: Config,
        customize: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
    ) -> Result<Self>;

    /// 以指定配置与外部构建的 HTTP 客户端创建实例
    /// （config.http 不作用于注入的 client，仅作为配置记录）
    pub fn with_client(config: Config, client: reqwest::Client) -> Self;

    /// 创建链式构建器（统一构建入口）
    pub fn builder() -> ArtfulBuilder;

    /// 获取实例配置
    pub fn config(&self) -> &Config;

    /// 获取实例 HTTP 客户端
    pub fn client(&self) -> &reqwest::Client;
    
    /// 执行插件链
    pub async fn artful(
        &self,
        params: HashMap<String, Value>,
        plugins: Vec<Arc<dyn Plugin>>,
    ) -> Result<Destination> {
        // 构建载体（params 存储原始参数，payload 初始为空）
        let mut rocket = Rocket::new(params);
        // 注入实例客户端
        rocket.client = self.client.clone();
        
        // 构建流向控制器
        let mut ctrl = FlowCtrl::new(plugins);
        
        // 启动洋葱流程，用 ? 传播错误
        ctrl.call_next(&mut rocket).await?;
        
        // 返回结果
        Ok(rocket.destination.unwrap_or_default())
    }
    
    /// 使用快捷方式执行请求
    pub async fn shortcut<S: Shortcut>(
        &self,
        shortcut: S,
        params: HashMap<String, Value>,
    ) -> Result<Destination> {
        let plugins = shortcut.get_plugins(&params);
        self.artful(params, plugins).await
    }
    
    /// 直接调用 HTTP（跳过插件）
    pub async fn raw(&self, request: reqwest::Request) -> Result<reqwest::Response> {
        self.client
            .execute(request)
            .await
            .map_err(ArtfulError::RequestFailed)
    }
}

impl ArtfulBuilder {
    /// 设置实例配置（覆盖式：后写覆盖先写；config.http 仅在未注入 client 时参与构建）
    pub fn config(self, config: Config) -> Self;

    /// 设置 HTTP 客户端自定义构建回调（覆盖式：后写覆盖先写）
    pub fn customize<F>(self, f: F) -> Self
    where
        F: FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder + Send + 'static;

    /// 注入外部构建的 HTTP 客户端（优先级最高：build 时忽略 config.http 与 customize）
    pub fn client(self, client: reqwest::Client) -> Self;

    /// 按优先级构建：注入 client > config.http + customize
    pub fn build(self) -> Result<Artful>;
}
```

> `ArtfulBuilder` 另实现 `Default` 与手写 `Debug`（装箱字段仅打印是否注入），满足 `Send`（非 `Sync`）。

### 3.2 HTTP 客户端设计

**核心设计决策**：HTTP Client 由 `Artful` 实例持有，与全局状态解耦

**原因**：
- reqwest::Client 内部维护连接池（hyper 管理），per-instance
- Client 配置（timeout、headers、proxy）构建时固定，不可修改
- Per-request timeout 通过 `RocketConfig.http`（`RequestOptions`）设置，自动覆盖 client 级默认
- `reqwest::Client` 内部 `Arc`，实例 `Clone` 廉价且共享连接池
- 客户端级选项（`ClientOptions`）在构造时全部接线，配置错误编译期/构造期暴露

**构造入口与 client 控制权**（按控制权从低到高）：

| 入口 | 构建方式 | `config.http` 是否生效 |
|------|----------|------------------------|
| `Artful::new()` / `Artful::with_config(config)` | 框架全托管：`build_builder` 按 `config.http` 应用全部选项后构建（`with_config` 即 `with_client_builder(config, \|b\| b)`） | ✅ |
| `Artful::with_client_builder(config, customize)` | 先按 `config.http` 应用框架默认值，再由回调叠加 `ClientOptions` 无法表达的能力（代理、TLS 证书、cookie 会话、重定向策略等）后构建；回调内后写的 setter 覆盖先写值 | ✅（回调可覆盖） |
| `Artful::with_client(config, client)` | 完全接管：注入外部构建的 client（跨实例共享连接池时使用） | ❌（仅作配置记录） |
| `Artful::builder()` ... `build()` | 链式统一入口：`config` / `customize` / `client` 可选叠加（后写覆盖先写）；未注入 client 时等价 `with_client_builder`，注入 client 时等价 `with_client`（优先级 client > config+customize） | 注入 client 时 ❌（仅作配置记录）；否则 ✅（回调可覆盖） |

```rust
use std::sync::OnceLock;
use std::time::Duration;

const DEFAULT_POOL_IDLE_TIMEOUT: u64 = 90;
const DEFAULT_POOL_MAX_IDLE_PER_HOST: usize = 20;
const DEFAULT_USER_AGENT: &str = concat!("yansongda/artisan-http:", env!("CARGO_PKG_VERSION"));

/// 按 ClientOptions 配置 ClientBuilder（消费全部字段，不构建）
///
/// 框架默认值兜底：pool_idle_timeout=90s、pool_max_idle_per_host=20、
/// UA=yansongda/artisan-http:{version}；未设置的 timeout/connect_timeout
/// 保持 reqwest 默认（无超时）。
pub(crate) fn build_builder(options: ClientOptions) -> reqwest::ClientBuilder {
    let pool_idle_timeout = options.pool_idle_timeout.unwrap_or(DEFAULT_POOL_IDLE_TIMEOUT);
    let pool_max_idle_per_host = options
        .pool_max_idle_per_host
        .unwrap_or(DEFAULT_POOL_MAX_IDLE_PER_HOST);
    let user_agent = options
        .user_agent
        .unwrap_or_else(|| DEFAULT_USER_AGENT.to_string());

    let mut builder = reqwest::Client::builder()
        .pool_idle_timeout(Some(Duration::from_secs(pool_idle_timeout)))
        .pool_max_idle_per_host(pool_max_idle_per_host)
        .user_agent(user_agent);

    if let Some(secs) = options.timeout {
        builder = builder.timeout(Duration::from_secs(secs));
    }

    if let Some(secs) = options.connect_timeout {
        builder = builder.connect_timeout(Duration::from_secs(secs));
    }

    builder
}

/// 按 ClientOptions 构建 HTTP 客户端（消费全部字段）
pub(crate) fn build_client(options: ClientOptions) -> Result<reqwest::Client, reqwest::Error> {
    build_builder(options).build()
}

/// 框架默认客户端（供直接构造 Rocket 使用，惰性初始化一次）
pub(crate) fn default_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    CLIENT.get_or_init(|| {
        build_client(ClientOptions::default()).unwrap_or_else(|_| fallback_client())
    })
}

fn fallback_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(DEFAULT_USER_AGENT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
```

**Per-request timeout 应用**（在 AddRadarPlugin 中）：

```rust
// 应用 timeout
if let Some(timeout) = rocket.config.http.timeout {
    request_builder = request_builder.timeout(
        Duration::from_secs(timeout)
    );
}
```

### 3.3 Direction - 响应解析器

```rust
/// 响应解析器 trait
#[async_trait]
pub trait Direction: Send + Sync {
    /// 解析响应
    async fn parse(&self, rocket: &mut Rocket) -> Result<Destination>;
}

/// 响应解析策略
#[derive(Clone)]
pub enum DirectionKind {
    /// 解析为 JSON（默认）
    Json,
    /// 返回原始 Response
    Response,
    /// 不发起 HTTP 请求
    NoRequest,
    /// 自定义解析器
    Custom(Arc<dyn Direction>),
}

/// 解析结果
#[derive(Debug)]
pub enum Destination {
    /// JSON 值（默认）
    Json(Value),
    /// 原始响应
    Response(reqwest::Response),
    /// 空结果
    None,
}
```

---

## 四、内置插件

### 4.1 StartPlugin - 初始化

```rust
/// 初始化插件
pub struct StartPlugin;

#[async_trait]
impl Plugin for StartPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()> {
        // 将 params 合并到 payload（直接字段访问，省一次中间 HashMap 分配）
        if rocket.payload.is_empty() {
            rocket.merge_params_to_payload();
        }
        next.call(rocket).await
    }
}
```

### 4.2 AddPayloadBodyPlugin - 添加请求体

```rust
/// 添加 payload body 插件
pub struct AddPayloadBodyPlugin;

#[async_trait]
impl Plugin for AddPayloadBodyPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()> {
        // 如果未手动指定 body，将 payload 按 packer 序列化
        if rocket.config.body.is_none() && !rocket.payload.is_empty() {
            rocket.config.body = Some(rocket.packer.pack(&rocket.payload)?);

            // 请求头缺失 Content-Type 时按 packer 声明补头
            // （判重按头名不区分大小写，不覆盖用户以任意大小写显式设置的值）
            if let Some(ct) = rocket.packer.content_type() {
                if !rocket.has_header("Content-Type") {
                    rocket.config.headers
                        .insert("Content-Type".to_string(), ct.to_string());
                }
            }
        }
        
        next.call(rocket).await
    }
}
```

### 4.3 AddRadarPlugin - 构建 HTTP 请求

```rust
/// 构建 HTTP Request 插件
pub struct AddRadarPlugin;

#[async_trait]
impl Plugin for AddRadarPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()> {
        // 使用实例注入的客户端
        let mut request_builder =
            rocket.client.request(rocket.config.method.clone(), &rocket.config.url);
        
        // 添加 headers
        for (key, value) in &rocket.config.headers {
            request_builder = request_builder.header(key, value);
        }
        
        // 添加 body；body 未设置且 payload 非空时走 fallback 打包分支：
        // 缺失 Content-Type 时直接补到 request_builder
        // （判重按头名不区分大小写；该分支位于 headers 遍历之后，写回 config.headers 不会再生效）
        if let Some(body) = &rocket.config.body {
            request_builder = request_builder.body(body.clone());
        } else if !rocket.payload.is_empty() {
            let body = rocket.packer.pack(&rocket.payload)?;

            if !rocket.has_header("Content-Type") {
                if let Some(ct) = rocket.packer.content_type() {
                    request_builder = request_builder.header("Content-Type", ct);
                }
            }

            request_builder = request_builder.body(body);
        }
        
        // 应用 timeout（per-request，自动覆盖 client 级默认）
        if let Some(timeout) = rocket.config.http.timeout {
            request_builder = request_builder.timeout(
                std::time::Duration::from_secs(timeout)
            );
        }
        
        // build 失败返回 RequestBuildError 错误（覆盖全部构建失败而非仅 URL）
        let request = request_builder.build()
            .map_err(|e| ArtfulError::RequestBuildError { source: e })?;
        rocket.radar = Some(request);
        
        next.call(rocket).await
    }
}
```

### 4.4 ParserPlugin - 解析响应

```rust
/// 解析响应插件 - 执行 HTTP 请求并解析结果
pub struct ParserPlugin;

#[async_trait]
impl Plugin for ParserPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()> {
        // NoRequest - 不发起请求
        if rocket.config.direction == DirectionKind::NoRequest {
            return next.call(rocket).await;
        }
        
        // 检查 radar，不存在则返回 MissingRequest 错误
        let request = rocket.radar.take()
            .ok_or(ArtfulError::MissingRequest)?;
        
        // 使用实例注入的客户端发送请求，失败则返回 RequestFailed 错误
        let response = rocket.client.execute(request).await
            .map_err(ArtfulError::RequestFailed)?;
        rocket.destination_origin = Some(response);
        
        // 解析响应
        let direction_kind = rocket.config.direction.clone();
        let destination = match direction_kind {
            DirectionKind::Json => {
                // Json 从 Response body 解析 JSON
                Json.parse(rocket).await?
            }
            DirectionKind::Response => {
                // 返回原始 Response
                rocket.destination_origin.take()
                    .map(Destination::Response)
                    .ok_or(ArtfulError::MissingResponse)?
            }
            DirectionKind::Custom(d) => {
                d.parse(rocket).await?
            }
            DirectionKind::NoRequest => {
                Destination::None
            }
        };
        
        rocket.destination = Some(destination);
        
        next.call(rocket).await
    }
}
```

**错误处理说明**：
- `MissingRequest` - radar 未构建（AddRadarPlugin 未执行或失败）
- `MissingResponse` - destination_origin 不存在
- `RequestFailed` - HTTP 请求失败
- `JsonSerializeError` - JSON 解析失败

---

## 五、使用示例

### 5.1 初始化框架

```rust
use artisan_http::Artful;

// 默认配置创建实例
let artful = Artful::new()?;

// 或以自定义配置创建（构造时构建 client，fail-fast）
let artful = Artful::with_config(config)?;

// 需要 ClientOptions 表达不了的能力（代理/TLS 证书/cookie 会话等）时，回调叠加（config.http 仍生效）
let artful = Artful::with_client_builder(config, |builder| builder.cookie_store(true))?;

// 或用链式 builder（统一入口）：config / customize / client 可选叠加后 build；
// 设置 .client() 时 config.http 与 customize 均不参与构建（优先级 client > config+customize）
let artful = Artful::builder()
    .config(config)
    .customize(|builder| builder.cookie_store(true))
    .build()?;

// 应用层全局单例推荐 LazyLock（见 §2.3）
```

### 5.2 基础使用

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
    let artful = Artful::new()?;

    let params = HashMap::from([
        ("order_id", json!("123")),
        ("amount", json!(100)),
    ]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: "https://api.example.com/orders".to_string(),
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let result = artful.artful(params, plugins).await?;
    
    if let artisan_http::Destination::Json(json) = result {
        println!("Response: {}", json);
    }

    Ok(())
}
```

### 5.3 使用 Shortcut 快捷方式

```rust
use artisan_http::{Artful, Shortcut, Plugin};
use artisan_http::plugins::{StartPlugin, AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin};
use std::sync::Arc;
use std::collections::HashMap;

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

// 构造 Shortcut 实例并调用
let artful = Artful::new()?;
let shortcut = MyApiShortcut {
    method: reqwest::Method::POST,
    url: "https://api.example.com/orders".to_string(),
};
let result = artful.shortcut(shortcut, HashMap::new()).await?;
```

**说明**：Shortcut 不需要 `Default` bound，可以在构造时携带任意状态（method、url 等），更灵活地配置请求。

### 5.4 自定义插件

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

### 5.5 错误处理

```rust
// HTTP 请求失败
let result = artful.artful(params, plugins).await;
// result: Err(ArtfulError::RequestFailed(...))

// radar 未构建
let result = artful.artful(params, vec![Arc::new(ParserPlugin)]).await;
// result: Err(ArtfulError::MissingRequest)

// JSON 解析失败
let result = artful.artful(params, plugins).await;
// result: Err(ArtfulError::JsonDeserializeError { .. })

// HTTP 客户端构建失败（with_config 时 fail-fast）
let result = Artful::with_config(config);
// result: Err(ArtfulError::ClientBuildError { .. })
```

---

## 六、模块结构

采用 Rust 标准惯例：**Trait 定义放在对应模块顶层**。

```
artisan-http/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs                  # 框架入口，导出公共 API
│   │
│   ├── artful.rs               # Artful 主入口（实例类型）
│   ├── rocket.rs               # Rocket + RocketConfig + ClientOptions/RequestOptions
│   ├── flow_ctrl.rs            # FlowCtrl 流向控制 + Next 闭包
│   ├── config.rs               # Config（http: ClientOptions + extra）
│   ├── error.rs                # ArtfulError 错误定义（英文 Display）
│   │
│   ├── plugin.rs               # Plugin trait
│   ├── plugins/                # 内置插件实现
│   │   ├── mod.rs              # 导出所有内置插件
│   │   ├── start.rs            # StartPlugin
│   │   ├── add_radar.rs        # AddRadarPlugin
│   │   ├── parser.rs           # ParserPlugin
│   │   └── add_payload_body.rs # AddPayloadBodyPlugin
│   │
│   ├── shortcut.rs             # Shortcut trait
│   │
│   ├── direction.rs            # Direction trait + DirectionKind + Destination
│   ├── directions/             # 内置 Direction 实现
│   │   ├── mod.rs              # 导出所有内置 Direction
│   │   └── json.rs             # Json
│   │
│   ├── packer.rs               # Packer trait（pack/unpack/content_type）
│   ├── packers/                # 内置 Packer 实现
│   │   ├── mod.rs              # 导出所有内置 Packer
│   │   └── json.rs             # JsonPacker
│   │
│   └── http.rs                 # build_builder / build_client / default_client（模块私有）
│
├── examples/
│   ├── basic.rs                # 基础使用示例
│   ├── custom_plugin.rs        # 自定义插件示例
│   ├── config.rs               # 配置初始化示例
│   ├── shortcut.rs             # Shortcut 快捷方式示例
│   └── direction.rs            # Direction 响应解析策略示例
│
├── tests/
│   ├── artful_test.rs
│   ├── direction_test.rs
│   ├── flow_ctrl_test.rs
│   ├── integration_test.rs
│   ├── packer_test.rs
│   ├── rocket_test.rs
│   └── shortcut_test.rs
│
└── docs/
    └── ARCHITECTURE.md         # 架构设计文档
```

### 模块说明

| 模块 | 说明 | Trait/类型 |
|------|------|-----------|
| `src/lib.rs` | 框架入口 | 导出公共 API |
| `src/artful.rs` | 主入口 | `Artful` struct（实例类型） |
| `src/rocket.rs` | 请求载体 + 配置 | `Rocket`, `RocketConfig`, `ClientOptions`, `RequestOptions` |
| `src/flow_ctrl.rs` | 流向控制器 | `FlowCtrl`, `Next` |
| `src/config.rs` | 框架配置 | `Config` |
| `src/plugin.rs` | 插件 trait | `Plugin` trait |
| `src/plugins/` | 内置插件 | `StartPlugin`, `AddRadarPlugin`, `ParserPlugin`, `AddPayloadBodyPlugin` |
| `src/shortcut.rs` | 快捷方式 trait | `Shortcut` trait |
| `src/direction.rs` | 解析策略 trait | `Direction`, `DirectionKind`, `Destination` |
| `src/directions/` | 内置解析器 | `Json` |
| `src/packer.rs` | 序列化 trait | `Packer` trait |
| `src/packers/` | 内置序列化器 | `JsonPacker` |
| `src/http.rs` | HTTP 客户端构建（模块私有） | `build_builder` / `build_client` / `default_client` |
| `src/error.rs` | 错误 | `ArtfulError` enum |

---

## 七、依赖设计

与真实 `Cargo.toml` 对齐：

```toml
[dependencies]
async-trait = { version = "~0.1.89" }
reqwest = { version = "~0.13.2", features = ["json"] }
serde_json = { version = "~1.0.149" }
thiserror = { version = "~2.0.18" }

[dev-dependencies]
tokio = { version = "~1.52.0", features = ["rt-multi-thread", "macros"] }
wiremock = { version = "~0.6.5" }
```

---

## 八、后续迭代规划

### v0.1.0 - MVP

- [x] 核心架构设计
- [x] 核心架构实现（Rocket, FlowCtrl, Plugin）
- [x] 内置插件（Start, AddPayloadBody, AddRadar, Parser, Log）
- [x] reqwest HTTP 客户端单例封装
- [x] JSON Packer
- [x] Direction 解析策略（Json, Response 等）
- [x] Artful 主入口（artisan, shortcut, raw 方法）
- [x] Shortcut trait
- [x] 基础测试覆盖
- [x] README 文档

### v0.14.0 - 实例化与配置治理（2026-08-29）

- [x] `Artful` 由静态类 + `OnceLock` 全局配置改为实例类型（`new`/`with_config`，fail-fast）
- [x] `Artful::with_builder`（config.http 基座 + 回调叠加）与 `Artful::with_client`（注入外部 client）自定义客户端入口
- [x] `HttpOptions` 按 client/request 生命周期拆分为 `ClientOptions`/`RequestOptions`，消除全局 `timeout`/`connect_timeout` 死字段
- [x] `Packer::content_type()` 自描述，默认链 JSON 请求自动补 `Content-Type`（仅缺失时补）
- [x] 错误 Display 英文化；`InvalidUrl` → `RequestBuildError`；新增 `ClientBuildError`
- [x] 删除 `FlowCtrl::cease()`（与 `skip_rest()` 重复）
- [x] 文档（README/AGENTS/ARCHITECTURE）与 crate 元数据全量同步

### v0.2.0 - 增强

- [ ] 事件系统（类似 PHP 版本）
- [ ] 错误处理插件
- [ ] 更多内置插件（Retry、Cache 等）

### v0.3.0 - 生态

- [ ] 支付宝支付插件包 `artisan-alipay`
- [ ] 微信支付插件包 `artisan-wechat`
- [ ] XML Packer 支持（可选）

---

## 九、参考资源

- [yansongda/artisan](https://github.com/yansongda/artisan) - PHP 版本框架
- [salvo-rs/salvo](https://github.com/salvo-rs/salvo) - Rust Web 框架（洋葱模型参考）
- [reqwest](https://github.com/seanmonstar/reqwest) - HTTP 客户端（连接池设计参考）
- [tower](https://github.com/tower-rs/tower) - Rust Service 抽象（可选参考）
