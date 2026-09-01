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

FlowCtrl 控制洋葱模型的执行流程，借鉴 [salvo](https://github.com/salvo-rs/salvo) 的设计，并升级为"洋葱链 + 固定终点"模型（对齐 Laravel Pipeline `then(ignite)` 与 reqwest-middleware "Client 恒为链尾终点"的既有实践）：插件链负责前向/后向处理，请求发起由框架内置链尾核心动作 `IgniteCore` 保证必然执行（见 §4.4），响应解析由链尾插件 `ParserPlugin` 承担（见 §4.5，插件链必须挂载），由 `Artful::artful` 自动挂载核心动作。

```rust
/// 链尾核心动作 trait - 洋葱链固定终点（pub(crate)，不进公开 API）
///
/// 终点无下一层：执行完毕即整个链路结束，返回值沿洋葱后向阶段回退传播
#[async_trait]
pub(crate) trait CoreAction: Send + Sync {
    async fn run(&self, rocket: &mut Rocket) -> Result<()>;
}

/// 洋葱模型流向控制器
pub struct FlowCtrl {
    /// 当前执行位置
    cursor: usize,
    
    /// 插件列表（线性排列）
    plugins: Vec<Arc<dyn Plugin>>,
    
    /// 链尾核心动作（终点，一次性消费）
    core: Option<Arc<dyn CoreAction>>,
    
    /// 是否已终止
    is_ceased: bool,
}

impl FlowCtrl {
    /// 创建新的流向控制器
    pub fn new(plugins: Vec<Arc<dyn Plugin>>) -> Self {
        Self {
            cursor: 0,
            plugins,
            core: None,
            is_ceased: false,
        }
    }
    
    /// 设置链尾核心动作（pub(crate)，唯一挂载点为 `Artful::artful`）
    pub(crate) fn set_core(&mut self, core: Arc<dyn CoreAction>) {
        self.core = Some(core);
    }
    
    /// 调用下一层插件（洋葱穿透）
    pub async fn call_next(&mut self, rocket: &mut Rocket) -> Result<()> {
        if self.is_ceased {
            return Ok(());  // 已终止：优先返回，保证 skip_rest 抑制 core 执行
        }
        
        if !self.has_next() {
            // 链尾：执行核心动作，返回值沿洋葱后向阶段回退传播；
            // 未挂 core 时行为与纯插件链直用场景一致（静默结束）
            if let Some(core) = self.core.take() {
                return core.run(rocket).await;
            }
            
            return Ok(());
        }
        
        let plugin = self.plugins[self.cursor].clone();
        self.cursor += 1;
        
        plugin.assembly(rocket, Next { ctrl: self }).await  // 传播错误
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

**设计要点**：
- **独立 `CoreAction` trait**：终点无 `next`，不复用 `Plugin` trait，也防止核心动作被误挂回插件链（杜绝双执行）
- **`set_core` 为 `pub(crate)`**：唯一挂载点是 `Artful::artful()`，防止高级用户自造双入口
- **`core.take()` 一次性消费**：终点只执行一次；插件双重调用 `next` 时第二次回落 `Ok(())`
- **`skip_rest` 跳过 core**："主动中止流程"语义优先于终点执行
- **空插件链 + 有 core**：core 直接执行（对齐 Laravel 空 pipes 时终点立即执行）

**执行流程示意**:

```
插件列表: [Start, Sign, AddRadar, ParserPlugin]    链尾核心动作: IgniteCore（框架自动挂载，仅执行 HTTP）

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
│   │   │   │   │   │   └─────────────────────────────────│
│   │   │   │   │   │   │ ParserPlugin.assembly()（链尾插件）│
│   │   │   │   │   │   │   ├─ 前向: 穿透（不做事）         │
│   │   │   │   │   │   │   ├─ next.call()                │
│   │   │   │   │   │   │   │   ┌─────────────────────────│
│   │   │   │   │   │   │   │   │ IgniteCore.run()（终点） │
│   │   │   │   │   │   │   │   │   ├─ NoRequest? → 返回   │
│   │   │   │   │   │   │   │   │   ├─ dispatch HttpStart  │
│   │   │   │   │   │   │   │   │   ├─ HTTP 请求执行       │
│   │   │   │   │   │   │   │   │   └─ dispatch HttpEnd    │
│   │   │   │   │   │   │   │   └─────────────────────────│
│   │   │   │   │   │   │   ├─ 后向: 解析 → destination    │
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
        // 框架自动挂载链尾核心动作：请求必然发起、HTTP 生命周期事件必然触发
        // （响应解析不在核心动作内：由插件链链尾的 ParserPlugin 承担，忘挂时请求
        // 照常发出但 destination 保持 None）
        ctrl.set_core(Arc::new(crate::ignite::IgniteCore));
        
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

### 3.4 事件系统

实例级事件系统：`EventDispatcher` 随 `Artful` 实例持有（无全局状态），监听器经
`Artful::builder().event_listener(...)` **追加式**注册（注册顺序即执行顺序，区别于
config / customize / client 的覆盖语义）；未注册监听器时零开销（空表跳过注入与分发）。
分发点内嵌于既有代码路径，不新增插件、不改插件链结构：

```
Artful::artful()
   │
   │  ① dispatch ArtfulStart        链启动前（只读观测 params / plugins）
   ▼
插件链: StartPlugin → AddPayloadBodyPlugin → AddRadarPlugin → ... → ParserPlugin（链尾，见 §4.5）
                                       │
           rocket.events = Some(Arc<EventDispatcher>)（实例注入传载）
                                       │
              ② 链尾核心动作 IgniteCore（框架自动挂载，见 §4.4）
                                       │
                              ③ dispatch HttpStart          execute 前（正常链中 radar 已构建，可改请求）
                              ④ rocket.client.execute(radar)
                                   ├─ Ok  → ⑤ dispatch HttpEnd    解析前（只读，响应体不可读）
                                   └─ Err → ⑥ dispatch HttpError  错误照常传播（只读）
   │
   │  ⑦ dispatch ArtfulEnd          链成功后（可改写 rocket.destination）
   ▼
   return rocket.destination.unwrap_or_default()
```

**事件一览**（与 PHP 版 yansongda/artful 语义对齐，`HttpError` 为 Rust 新增）：

| 事件 | 触发时机 | 可变性 | 对应 PHP artful |
|------|---------|--------|----------------|
| `ArtfulStart` | 插件链启动前 | 只读 | `Event\ArtfulStart` |
| `HttpStart` | 到达链尾核心动作执行点、请求即将发出（正常链中 radar 已构建；缺 `AddRadarPlugin` 时 radar 为 `None`，事件仍触发） | 可改 radar | `Event\HttpStart` |
| `HttpEnd` | 请求成功返回、解析前（响应体不可读：body 消费权属于 direction 解析，仅可读 status / headers） | 只读 | `Event\HttpEnd` |
| `HttpError` | execute 失败（错误照常向上传播） | 只读 | —（Rust 新增） |
| `ArtfulEnd` | 链成功后、返回 destination 前 | 可改写 destination | `Event\ArtfulEnd` |

**触发矩阵**（契约：监听器可无条件依赖下表）：

| 场景 | ArtfulStart | HttpStart | HttpEnd | HttpError | ArtfulEnd |
|------|:-:|:-:|:-:|:-:|:-:|
| 正常请求 | ✅ | ✅ | ✅ | — | ✅ |
| HTTP 执行失败 | ✅ | ✅ | — | ✅ | — |
| `NoRequest` | ✅ | — | — | — | ✅ |
| 插件失败（前向阶段） | ✅ | — | — | — | — |
| 插件不调 next 返 Ok | ✅ | — | — | — | ✅ |
| 链尾缺 radar | ✅ | ✅ | — | — | — |
| 解析阶段失败（execute 成功后） | ✅ | ✅ | ✅ | — | — |

**错误语义**：监听器按注册顺序同步执行，任一监听器返回 `Err` → 立即停止后续监听器，
错误包装为 `EventListenerError { listener_name, message, source, original }` 向上
传播、中断主流程；仅需旁路观察（日志/metrics）的监听器应内部消化错误、恒返回
`Ok(())`。监听器必须**非阻塞**，耗时任务请自行 `tokio::spawn`。`HttpStart` 中修改
请求须经 `rocket.radar` 的 `*_mut` 访问器（到达链尾核心动作执行点时正常链中 radar
已构建，此时改 `rocket.config` 不影响本次请求；链中缺 `AddRadarPlugin` 时 radar
为 `None`，事件仍触发）。特殊地，`HttpError` 分发中监听器自身失败时，原始
`RequestFailed` 保留在 `EventListenerError.original` 字段（错误链不丢失）。

**与插件链的关系**：HTTP 生命周期事件由框架内置链尾核心动作 `IgniteCore`
（`Artful::artful()` 自动挂载，见 §4.4）在请求执行点分发，分发器经 `Rocket.events`
（`Option<Arc<EventDispatcher>>`，默认 `None`）由 `Artful::artful()` 注入传载进插件
链；`Artful` 生命周期事件在 `Artful::artful()` 入口 / 出口分发。手动构造的 `Rocket`
（`events` 为 `None`）不分发任何事件；`raw()` 完全跳过插件链与事件；`shortcut()`
内部走 `artful()`，完整经过事件路径。

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
        // 如果未手动指定 body，将 payload 按 packer 序列化（params 传空，对齐 PHP 内置链传 null）
        if rocket.config.body.is_none() && !rocket.payload.is_empty() {
            rocket.config.body = Some(rocket.packer.pack(&rocket.payload, &HashMap::new())?);

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
            let body = rocket.packer.pack(&rocket.payload, &HashMap::new())?;

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

### 4.4 IgniteCore - 链尾核心动作（HTTP 执行）

链尾核心动作是洋葱链的固定终点（对齐 artful PHP 的 `ignite()`）：仅执行 HTTP 请求并分发 HTTP 生命周期事件，**不解析响应**。它不是插件，实现 `pub(crate)` 的 `CoreAction` trait（终点无下一层，不复用 `Plugin`），由 `Artful::artful()` 自动挂载（`ctrl.set_core(Arc::new(IgniteCore))`）。响应解析由链尾插件 `ParserPlugin` 承担（见 §4.5）：用户插件链**必须**在末尾挂 `ParserPlugin`，忘挂时请求照常发出（`destination_origin` 持有原始响应）但 `rocket.destination` 保持 `None`。

```rust
/// 链尾核心动作 trait - 洋葱链固定终点（pub(crate)，不进公开 API）
#[async_trait]
pub(crate) trait CoreAction: Send + Sync {
    async fn run(&self, rocket: &mut Rocket) -> Result<()>;
}

/// 链尾核心动作 - 仅执行 HTTP 请求（解析由链尾插件 ParserPlugin 承担）
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct IgniteCore;

#[async_trait]
impl CoreAction for IgniteCore {
    async fn run(&self, rocket: &mut Rocket) -> Result<()> {
        // NoRequest - 不发起请求，直接结束（不触发任何 HTTP 事件）
        if let DirectionKind::NoRequest = rocket.config.direction {
            return Ok(());
        }

        // 先克隆分发器 Arc 再分发，规避 rocket 的自借用冲突
        let events = rocket.events.clone();

        // HttpStart：请求即将发出（radar 尚未被消费，监听器可见并可修改 radar）
        if let Some(events) = &events {
            events.dispatch(Event::HttpStart { rocket: &mut *rocket })?;
        }

        // 发送 HTTP 请求（radar 缺失返回 MissingRequest）
        let response = rocket
            .client
            .execute(rocket.radar.take().ok_or(ArtfulError::MissingRequest)?)
            .await
            .map_err(ArtfulError::RequestFailed);

        match response {
            Ok(response) => {
                rocket.destination_origin = Some(response);

                // HttpEnd：请求成功返回、响应解析（由链中后置插件完成）之前
                if let Some(events) = &events {
                    events.dispatch(Event::HttpEnd { rocket: &*rocket })?;
                }
            }
            Err(err) => {
                // HttpError：仅 execute 失败触发（MissingRequest 属请求前置失败，不触发）；
                // 监听器自身失败时，原始 RequestFailed 保留在 original 字段（错误链不丢失）；
                // 否则错误照常传播
                if let Some(events) = &events {
                    if let Err(mut listener_err) = events.dispatch(Event::HttpError {
                        rocket: &*rocket,
                        error: &err,
                    }) {
                        if let ArtfulError::EventListenerError { original, .. } = &mut listener_err
                        {
                            *original = Some(Box::new(err));
                        }

                        return Err(listener_err);
                    }
                }

                return Err(err);
            }
        }

        Ok(())
    }
}
```

**错误处理说明**：
- `MissingRequest` - radar 未构建（AddRadarPlugin 未执行或失败）；属请求前置失败，不触发 `HttpError`
- `RequestFailed` - HTTP 请求失败
- `EventListenerError` - 事件监听器失败（中断主流程；`HttpError` 分发中监听器失败时，原始 `RequestFailed` 保留在其 `original` 字段）

### 4.5 ParserPlugin - 后置响应解析插件

响应解析由链尾插件 `ParserPlugin` 承担（0.16.0 曾内置于 `IgniteCore`，0.17.0 移回插件形态，对齐 PHP artful 的 `ParserPlugin`）：前向阶段直接穿透，HTTP 完成后在后向阶段按 `rocket.config.direction` 分发解析方向，把 `destination_origin` 解析为 `rocket.destination`。**必须挂在链尾**：忘挂时请求照常发出但不解析（`destination` 保持 `None`）。

```rust
/// 后置响应解析插件：解析响应为 destination，必须挂在链尾
#[derive(Clone, Copy, Debug, Default)]
pub struct ParserPlugin;

#[async_trait]
impl Plugin for ParserPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()> {
        // 后置插件：前向直接穿透，HTTP 完成后在后向阶段解析
        next.call(rocket).await?;

        // 守卫：destination 只能是 None 或 Response（对齐 PHP artful 9208）
        if let Some(Destination::Json(_)) = rocket.destination {
            return Err(ArtfulError::InvalidParameter {
                param: "destination".to_string(),
                message: "ParserPlugin 中 Rocket 的 destination 只能是 None 或 Response".to_string(),
            });
        }

        // 按 direction 分发解析（各内置方向对应独立 Direction 实现）
        let destination = match &rocket.config.direction {
            DirectionKind::Json => JsonDirection.parse(rocket).await?,
            DirectionKind::Response => OriginResponseDirection.parse(rocket).await?,
            // 透传 destination 现有值（无值时为 Destination::None）
            DirectionKind::NoRequest => NoHttpRequestDirection.parse(rocket).await?,
            DirectionKind::Custom(direction) => direction.clone().parse(rocket).await?,
        };

        rocket.destination = Some(destination);

        Ok(())
    }
}
```

**行为要点**：
- `Json` 方向经 `rocket.packer.unpack` 解包响应体（默认 packer `JsonPacker`；替换为 `XmlPacker` 后响应即按 XML 解包）；`JsonDirection` 把 `rocket.payload` 全量作为 params 传给 packer（不过滤 `_` 前缀，对齐 PHP `$payload?->all()`），因此 `QueryPacker` 的 `_unpack_raw` 等控制参数可经 payload 生效
- `NoRequest` 方向 + 链尾 `ParserPlugin` 下 `rocket.destination` 为 `Some(Destination::None)`（0.16.0 中为 `None`；经 `Artful::artful` 入口的返回值不变）
- 守卫：预置 `rocket.destination` 只能是 `None` 或 `Destination::Response`，预置其他值返回 `InvalidParameter`

**错误处理说明**：
- `MissingResponse` - `Response` 方向下 `destination_origin` 不存在
- `JsonSerializeError` / `JsonDeserializeError` - JSON 序列化/解析失败
- `XmlSerializeError` / `XmlDeserializeError` - XML 序列化/解析失败
- `InvalidParameter` - 守卫拒绝（destination 预置了非 `None`/`Response` 值）

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
use artisan_http::plugins::{ParserPlugin, StartPlugin, AddPayloadBodyPlugin, AddRadarPlugin};
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
use artisan_http::plugins::{ParserPlugin, StartPlugin, AddPayloadBodyPlugin, AddRadarPlugin};
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

// radar 未构建（空插件链时链尾核心动作仍执行，发现 radar 缺失即 fail-fast）
let result = artful.artful(params, vec![]).await;
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
│   ├── flow_ctrl.rs            # FlowCtrl 流向控制 + Next 闭包 + CoreAction
│   ├── ignite.rs               # IgniteCore 链尾核心动作（仅 HTTP 执行；响应解析归链尾插件 ParserPlugin）
│   ├── config.rs               # Config（http: ClientOptions + extra）
│   ├── error.rs                # ArtfulError 错误定义（英文 Display）
│   │
│   ├── plugin.rs               # Plugin trait
│   ├── plugins/                # 内置插件实现
│   │   ├── mod.rs              # 导出所有内置插件
│   │   ├── start.rs            # StartPlugin
│   │   ├── add_radar.rs        # AddRadarPlugin
│   │   ├── add_payload_body.rs # AddPayloadBodyPlugin
│   │   └── parser.rs           # ParserPlugin（后置响应解析，必须链尾）
│   │
│   ├── shortcut.rs             # Shortcut trait
│   │
│   ├── direction.rs            # Direction trait + DirectionKind + Destination
│   ├── directions/             # 内置 Direction 实现
│   │   ├── mod.rs              # 导出所有内置 Direction
│   │   ├── json.rs             # JsonDirection
│   │   ├── no_http_request.rs  # NoHttpRequestDirection
│   │   └── origin_response.rs  # OriginResponseDirection
│   │
│   ├── packer.rs               # Packer trait（pack/unpack/content_type）
│   ├── packers/                # 内置 Packer 实现
│   │   ├── mod.rs              # 导出所有内置 Packer
│   │   ├── json.rs             # JsonPacker
│   │   ├── query.rs            # QueryPacker
│   │   └── xml.rs              # XmlPacker
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
│   ├── event_test.rs
│   ├── integration_test.rs
│   ├── parser_test.rs
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
| `src/flow_ctrl.rs` | 流向控制器 | `FlowCtrl`, `Next`（`CoreAction` 为 `pub(crate)`） |
| `src/ignite.rs` | 链尾核心动作 | `IgniteCore`（`pub(crate)`，仅 HTTP 执行，由 `Artful::artful()` 自动挂载） |
| `src/config.rs` | 框架配置 | `Config` |
| `src/plugin.rs` | 插件 trait | `Plugin` trait |
| `src/plugins/` | 内置插件 | `StartPlugin`, `AddRadarPlugin`, `AddPayloadBodyPlugin`, `ParserPlugin`（HTTP 执行由链尾核心动作 `IgniteCore` 承担，见 §4.4；响应解析由链尾插件 `ParserPlugin` 承担，见 §4.5） |
| `src/shortcut.rs` | 快捷方式 trait | `Shortcut` trait |
| `src/direction.rs` | 解析策略 trait | `Direction`, `DirectionKind`, `Destination` |
| `src/directions/` | 内置解析器 | `JsonDirection`, `NoHttpRequestDirection`, `OriginResponseDirection` |
| `src/packer.rs` | 序列化 trait | `Packer` trait |
| `src/packers/` | 内置序列化器 | `JsonPacker`, `QueryPacker`, `XmlPacker` |
| `src/http.rs` | HTTP 客户端构建（模块私有） | `build_builder` / `build_client` / `default_client` |
| `src/error.rs` | 错误 | `ArtfulError` enum |

---

## 七、依赖设计

与真实 `Cargo.toml` 对齐：

```toml
[dependencies]
async-trait = { version = "~0.1.89" }
quick-xml = { version = "~0.41" }
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
- [x] 内置插件（Start, AddPayloadBody, AddRadar, Log）+ 链尾核心动作 IgniteCore
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

### v0.16.0 - 事件系统（2026-08-31）

- [x] `Event` / `EventListener` / `EventDispatcher` 事件核心类型（同步监听器、注册顺序即执行顺序、首错中止并包装 `EventListenerError`）
- [x] 5 个生命周期事件：ArtfulStart / HttpStart / HttpEnd / HttpError / ArtfulEnd（PHP artful 4 事件语义对齐 + Rust 新增 HttpError，见 §3.4）
- [x] `ArtfulBuilder::event_listener` 追加式注册；`Rocket.events` 传载；分发点内嵌 `Artful::artful()`，HTTP 事件分发点位于框架内置链尾核心动作 `IgniteCore`（见 §4.4）

### v0.2.0 - 增强

- [x] 事件系统（v0.16.0 已实现，见 §3.4 与仓库根 `docs/event-system.md`）
- [ ] 错误处理插件
- [ ] 更多内置插件（Retry、Cache 等）

### v0.17.0 - 解析回归插件化 + Packer/Direction 家族扩展（2026-09-01）

- [x] `ParserPlugin` 回归：响应解析由 `IgniteCore` 移回链尾插件（`IgniteCore` 仅 HTTP 执行，见 §4.4 / §4.5）
- [x] `QueryPacker`（RFC1738 + `_unpack_raw` 原始模式）与 `XmlPacker`（CDATA 格式，quick-xml 0.41）
- [x] `NoHttpRequestDirection` / `OriginResponseDirection` 独立 Direction 实现
- [x] `Packer::pack`/`unpack` 增加 `params` 形参；`JsonDirection` 改经 `rocket.packer.unpack` 解包
- [x] XML Packer 支持（原 v0.3.0 规划项，提前落地）

### v0.3.0 - 生态

- [ ] 支付宝支付插件包 `artisan-alipay`
- [ ] 微信支付插件包 `artisan-wechat`

---

## 九、参考资源

- [yansongda/artisan](https://github.com/yansongda/artisan) - PHP 版本框架
- [salvo-rs/salvo](https://github.com/salvo-rs/salvo) - Rust Web 框架（洋葱模型参考）
- [reqwest](https://github.com/seanmonstar/reqwest) - HTTP 客户端（连接池设计参考）
- [tower](https://github.com/tower-rs/tower) - Rust Service 抽象（可选参考）
