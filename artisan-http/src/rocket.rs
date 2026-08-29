//! 请求载体模块
//!
//! 定义请求生命周期中的数据载体和配置类型。
//!
//! # 核心类型
//!
//! - [`Rocket`] - 请求载体，携带所有请求/响应数据
//! - [`RocketConfig`] - HTTP 请求配置（method, url, headers 等）
//! - [`ClientOptions`] - 客户端级 HTTP 选项（构建 `reqwest::Client` 时生效）
//! - [`RequestOptions`] - 请求级 HTTP 选项（仅 timeout，单次请求生效）
//!
//! # 设计说明
//!
//! - `params`: 原始参数，整个生命周期中保持不变
//! - `payload`: 业务参数，由 `StartPlugin` 从 params 初始化，后续插件可修改
//! - `RocketConfig` 所有字段可在 plugin 中动态修改

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::direction::DirectionKind;
use crate::packer::Packer;
use crate::packers::JsonPacker;

/// HTTP 请求配置
///
/// 所有字段可在插件中动态修改。
#[derive(Debug, Clone)]
pub struct RocketConfig {
    pub method: reqwest::Method,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub http: RequestOptions,
    pub direction: DirectionKind,
}

impl Default for RocketConfig {
    fn default() -> Self {
        Self {
            method: reqwest::Method::POST,
            url: String::new(),
            headers: HashMap::new(),
            body: None,
            http: RequestOptions::default(),
            direction: DirectionKind::Json,
        }
    }
}

/// 客户端级 HTTP 选项
///
/// 仅在构建 `reqwest::Client` 时生效（如 [`Artful::with_config`]），
/// 对该客户端发出的所有请求生效。未设置的字段使用 reqwest 默认值或框架默认值。
///
/// [`Artful::with_config`]: crate::artful::Artful::with_config
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

    /// User-Agent 字符串
    pub user_agent: Option<String>,
}

/// 请求级 HTTP 选项
///
/// 仅对单次请求生效（[`RocketConfig::http`]）。
///
/// 仅含 `timeout`：reqwest 0.13 的 `RequestBuilder` 不提供 `connect_timeout`
/// （该方法仅 `ClientBuilder` 支持），请求级连接超时无法表达。
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestOptions {
    /// 请求超时（秒）
    pub timeout: Option<u64>,
}

/// 请求载体
///
/// 贯穿整个插件链，携带请求参数、HTTP 配置、请求对象和响应数据。
/// 插件通过修改 `payload`、`config` 等字段来组装请求。
pub struct Rocket {
    /// 原始参数（不变）
    params: HashMap<String, Value>,

    /// 业务参数（可修改）
    pub payload: HashMap<String, Value>,

    /// Rocket 配置（可修改）
    pub config: RocketConfig,

    /// HTTP 客户端（由 `Artful` 实例注入，默认为框架内置客户端）
    pub client: reqwest::Client,

    /// HTTP 请求对象
    pub radar: Option<reqwest::Request>,

    /// HTTP 原始响应
    pub destination_origin: Option<reqwest::Response>,

    /// 最终解析结果
    pub destination: Option<crate::direction::Destination>,

    /// 序列化器
    pub packer: Arc<dyn Packer>,
}

impl std::fmt::Debug for Rocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rocket")
            .field("params", &self.params)
            .field("payload", &self.payload)
            .field("config", &self.config)
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

impl Rocket {
    /// 创建 Rocket
    ///
    /// params 存储原始参数，payload 初始为空（由 `StartPlugin` 初始化）
    /// config 使用默认值，由插件负责设置 method、url 等
    #[must_use]
    pub fn new(params: HashMap<String, Value>) -> Self {
        Self {
            params,
            payload: HashMap::new(),
            config: RocketConfig::default(),
            client: crate::http::default_client().clone(),
            radar: None,
            destination_origin: None,
            destination: None,
            packer: Arc::new(JsonPacker),
        }
    }

    /// 获取原始参数（不变）
    pub fn get_params(&self) -> &HashMap<String, Value> {
        &self.params
    }

    /// 合并参数到 payload
    ///
    /// 将 params 中的参数合并到 payload，用于 `StartPlugin` 初始化 payload
    pub fn merge_params_to_payload(&mut self) {
        for (k, v) in self.params.iter() {
            self.payload.insert(k.clone(), v.clone());
        }
    }

    /// 设置 HTTP 方法
    pub fn set_method(&mut self, method: reqwest::Method) {
        self.config.method = method;
    }

    /// 设置请求 URL
    pub fn set_url(&mut self, url: impl Into<String>) {
        self.config.url = url.into();
    }

    /// 添加请求头
    pub fn add_header(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.config.headers.insert(key.into(), value.into());
    }

    /// 判断是否已设置指定请求头（头名不区分大小写，符合 RFC 9110 语义）
    pub(crate) fn has_header(&self, name: &str) -> bool {
        self.config
            .headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case(name))
    }

    /// 设置请求体
    pub fn set_body(&mut self, body: impl Into<String>) {
        self.config.body = Some(body.into());
    }

    /// 设置超时时间（秒）
    pub fn set_timeout(&mut self, timeout: u64) {
        self.config.http.timeout = Some(timeout);
    }
}

impl From<HashMap<String, Value>> for Rocket {
    fn from(params: HashMap<String, Value>) -> Self {
        Self::new(params)
    }
}
